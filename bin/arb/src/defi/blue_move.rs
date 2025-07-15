//! BlueMove DEX 协议适配器
//!
//! 本文件实现了 BlueMove 去中心化交易所的协议适配，提供以下核心功能：
//! - BlueMove 流动性池的状态查询和验证
//! - 代币交换交易的构建和执行
//! - 与 Sui 区块链的交互接口
//! - 套利机器人的交易路径扩展支持
//!
//! BlueMove 是 Sui 生态系统中的一个重要 DEX，支持多种代币对的交易。
//! 该适配器确保与 BlueMove 智能合约的正确交互，并提供统一的 Dex trait 实现。

use std::sync::Arc;

use dex_indexer::types::{Pool, Protocol};
use eyre::{ensure, eyre, OptionExt, Result};
use move_core_types::annotated_value::MoveStruct;
use simulator::Simulator;
use sui_types::{
    base_types::{ObjectID, ObjectRef, SuiAddress},
    transaction::{Argument, Command, ObjectArg, ProgrammableTransaction, TransactionData},
    Identifier, TypeTag,
};
use tokio::sync::OnceCell;
use utils::{coin, new_test_sui_client, object::*};

use super::{TradeCtx, CETUS_AGGREGATOR};
use crate::{config::*, defi::Dex};

/// BlueMove DEX 信息对象的固定地址
/// 这个对象包含了 BlueMove 协议的全局配置和状态信息
const DEX_INFO: &str = "0x3f2d9f724f4a1ce5e71676448dc452be9a6243dac9c5b975a588c8c867066e92";

/// 全局对象参数缓存，避免重复获取 DEX 信息对象
/// 使用 OnceCell 确保线程安全的单次初始化
static OBJ_CACHE: OnceCell<ObjectArgs> = OnceCell::const_new();

/// 获取 BlueMove DEX 的对象参数
/// 
/// 该函数使用缓存机制，确保 DEX 信息对象只被获取一次，提高性能。
/// DEX 信息对象是共享对象，包含了协议的全局状态。
/// 
/// # 参数
/// * `simulator` - 区块链状态模拟器，用于获取对象数据
/// 
/// # 返回值
/// 返回包含 DEX 信息对象参数的 ObjectArgs 结构体
async fn get_object_args(simulator: Arc<Box<dyn Simulator>>) -> ObjectArgs {
    OBJ_CACHE
        .get_or_init(|| async {
            // 将十六进制字符串转换为 ObjectID
            let id = ObjectID::from_hex_literal(DEX_INFO).unwrap();
            // 从模拟器获取 DEX 信息对象
            let dex_info = simulator.get_object(&id).await.unwrap();

            ObjectArgs {
                // 创建共享对象参数，第二个参数 true 表示这是一个可变的共享对象
                dex_info: shared_obj_arg(&dex_info, true),
            }
        })
        .await
        .clone()
}

/// BlueMove DEX 交易所需的对象参数
/// 
/// 包含了与 BlueMove 智能合约交互时需要的对象引用
#[derive(Clone)]
pub struct ObjectArgs {
    /// DEX 信息对象参数，包含协议的全局配置
    dex_info: ObjectArg,
}

/// BlueMove DEX 实例
/// 
/// 代表一个特定的 BlueMove 流动性池，包含了执行交易所需的所有信息。
/// 每个实例对应一个特定的交易方向（如 A->B 或 B->A）。
#[derive(Clone)]
pub struct BlueMove {
    /// 流动性池的基本信息（池子 ID、代币类型等）
    pool: Pool,
    /// 池子的流动性数量，用于评估交易容量
    liquidity: u128,
    /// 输入代币类型的完整路径
    coin_in_type: String,
    /// 输出代币类型的完整路径
    coin_out_type: String,
    /// Move 类型参数，用于泛型函数调用
    type_params: Vec<TypeTag>,
    /// DEX 信息对象参数，交易时需要传递给智能合约
    dex_info: ObjectArg,
}

impl BlueMove {
    /// 创建新的 BlueMove DEX 实例
    /// 
    /// 该函数会验证池子的有效性，获取流动性信息，并确定交易方向。
    /// 
    /// # 参数
    /// * `simulator` - 区块链状态模拟器
    /// * `pool` - 流动性池信息
    /// * `coin_in_type` - 输入代币类型
    /// 
    /// # 返回值
    /// 成功时返回配置好的 BlueMove 实例，失败时返回错误
    /// 
    /// # 错误情况
    /// - 池子不是 BlueMove 协议
    /// - 池子被冻结
    /// - 无法获取池子对象或布局
    pub async fn new(simulator: Arc<Box<dyn Simulator>>, pool: &Pool, coin_in_type: &str) -> Result<Self> {
        // 验证这确实是一个 BlueMove 池子
        ensure!(pool.protocol == Protocol::BlueMove, "not a BlueMove pool");

        // 获取并解析池子对象的详细信息
        let parsed_pool = {
            // 从模拟器获取池子对象
            let pool_obj = simulator
                .get_object(&pool.pool)
                .await
                .ok_or_else(|| eyre!("pool not found: {}", pool.pool))?;

            // 获取对象的内存布局信息
            let layout = simulator
                .get_object_layout(&pool.pool)
                .ok_or_eyre("pool layout not found")?;

            // 确保这是一个 Move 对象
            let move_obj = pool_obj.data.try_as_move().ok_or_eyre("not a move object")?;
            // 反序列化 Move 结构体
            MoveStruct::simple_deserialize(move_obj.contents(), &layout).map_err(|e| eyre!(e))?
        };

        // 检查池子是否被冻结
        let is_freeze = extract_bool_from_move_struct(&parsed_pool, "is_freeze")?;
        ensure!(!is_freeze, "pool is frozen");

        // 提取流动性信息
        let liquidity = {
            // 获取 LSP (Liquidity Share Pool) 供应量结构体
            let lsp_supply = extract_struct_from_move_struct(&parsed_pool, "lsp_supply")?;
            // 提取实际的数值
            extract_u64_from_move_struct(&lsp_supply, "value")? as u128
        };

        // 根据输入代币类型确定输出代币类型
        // 如果输入代币是 token0，则输出 token1，反之亦然
        let coin_out_type = if let Some(0) = pool.token_index(coin_in_type) {
            pool.token1_type()
        } else {
            pool.token0_type()
        };

        // 获取类型参数，用于泛型函数调用
        let type_params = parsed_pool.type_.type_params.clone();

        // 获取 DEX 信息对象参数
        let ObjectArgs { dex_info } = get_object_args(simulator).await;

        Ok(Self {
            pool: pool.clone(),
            liquidity,
            coin_in_type: coin_in_type.to_string(),
            coin_out_type,
            type_params,
            dex_info,
        })
    }

    /// 构建完整的交换交易
    /// 
    /// 创建一个完整的可编程交易，包括代币分割、交换和转账操作。
    /// 
    /// # 参数
    /// * `sender` - 交易发送者地址
    /// * `recipient` - 接收者地址
    /// * `coin_in` - 输入代币对象引用
    /// * `amount_in` - 输入代币数量
    /// 
    /// # 返回值
    /// 返回构建好的可编程交易
    async fn build_swap_tx(
        &self,
        sender: SuiAddress,
        recipient: SuiAddress,
        coin_in: ObjectRef,
        amount_in: u64,
    ) -> Result<ProgrammableTransaction> {
        let mut ctx = TradeCtx::default();

        // 从输入代币中分割出指定数量
        let coin_in = ctx.split_coin(coin_in, amount_in)?;
        // 执行交换操作，获得输出代币
        let coin_out = self.extend_trade_tx(&mut ctx, sender, coin_in, None).await?;
        // 将输出代币转账给接收者
        ctx.transfer_arg(recipient, coin_out);

        Ok(ctx.ptb.finish())
    }

    /// 构建交换函数的参数列表
    /// 
    /// BlueMove 的交换函数签名：
    /// ```move
    /// public fun swap_a2b<CoinA, CoinB>(
    ///     dex_info: &mut Dex_Info,
    ///     coin_a: Coin<CoinA>,
    ///     ctx: &mut TxContext,
    /// ): Coin<CoinB>
    /// ```
    /// 
    /// # 参数
    /// * `ctx` - 交易构建上下文
    /// * `coin_in_arg` - 输入代币参数
    /// 
    /// # 返回值
    /// 返回按顺序排列的函数参数列表
    fn build_swap_args(&self, ctx: &mut TradeCtx, coin_in_arg: Argument) -> Result<Vec<Argument>> {
        // 获取 DEX 信息对象参数
        let dex_info_arg = ctx.obj(self.dex_info).map_err(|e| eyre!(e))?;

        // 按照 Move 函数签名的顺序返回参数
        // 注意：TxContext 由运行时自动提供，不需要显式传递
        Ok(vec![dex_info_arg, coin_in_arg])
    }
}

/// 实现 Dex trait，提供统一的 DEX 接口
#[async_trait::async_trait]
impl Dex for BlueMove {
    /// 扩展交易上下文，添加 BlueMove 交换操作
    /// 
    /// 该方法将 BlueMove 的交换调用添加到交易构建器中，
    /// 是套利路径构建的核心组件。
    /// 
    /// # 参数
    /// * `ctx` - 交易构建上下文，用于添加命令
    /// * `_sender` - 交易发送者（BlueMove 不需要此参数）
    /// * `coin_in` - 输入代币参数
    /// * `_amount_in` - 输入数量（BlueMove 使用整个代币对象）
    /// 
    /// # 返回值
    /// 返回输出代币的参数引用
    async fn extend_trade_tx(
        &self,
        ctx: &mut TradeCtx,
        _sender: SuiAddress,
        coin_in: Argument,
        _amount_in: Option<u64>,
    ) -> Result<Argument> {
        // 根据交易方向选择对应的函数名
        let function = if self.is_a2b() { "swap_a2b" } else { "swap_b2a" };

        // 构建 Move 函数调用
        let package = ObjectID::from_hex_literal(CETUS_AGGREGATOR)?;  // 聚合器包地址
        let module = Identifier::new("bluemove").map_err(|e| eyre!(e))?;  // 模块名
        let function = Identifier::new(function).map_err(|e| eyre!(e))?;  // 函数名
        let type_arguments = self.type_params.clone();  // 泛型类型参数
        let arguments = self.build_swap_args(ctx, coin_in)?;  // 函数参数
        
        // 添加 Move 调用命令到交易中
        ctx.command(Command::move_call(package, module, function, type_arguments, arguments));

        // 返回最后一个命令的结果作为输出代币
        let last_idx = ctx.last_command_idx();
        Ok(Argument::Result(last_idx))
    }

    /// 获取输入代币类型
    fn coin_in_type(&self) -> String {
        self.coin_in_type.clone()
    }

    /// 获取输出代币类型
    fn coin_out_type(&self) -> String {
        self.coin_out_type.clone()
    }

    /// 获取协议类型
    fn protocol(&self) -> Protocol {
        Protocol::BlueMove
    }

    /// 获取池子流动性
    /// 
    /// 流动性数量用于评估交易容量和滑点影响
    fn liquidity(&self) -> u128 {
        self.liquidity
    }

    /// 获取池子对象 ID
    fn object_id(&self) -> ObjectID {
        self.pool.pool
    }

    /// 翻转交易方向
    /// 
    /// 将输入和输出代币类型互换，用于反向交易路径
    fn flip(&mut self) {
        std::mem::swap(&mut self.coin_in_type, &mut self.coin_out_type);
    }

    /// 判断是否为 A 到 B 的交易方向
    /// 
    /// 返回 true 表示从 token0 到 token1，false 表示从 token1 到 token0
    fn is_a2b(&self) -> bool {
        self.pool.token_index(&self.coin_in_type) == Some(0)
    }

    /// 构建完整的交换交易数据（用于测试）
    /// 
    /// 该方法创建一个完整的交易数据结构，包括获取代币、构建交易和设置 Gas。
    /// 主要用于单元测试和集成测试。
    /// 
    /// # 参数
    /// * `sender` - 交易发送者地址
    /// * `recipient` - 代币接收者地址
    /// * `amount_in` - 输入代币数量
    /// 
    /// # 返回值
    /// 返回可以直接提交到区块链的交易数据
    async fn swap_tx(&self, sender: SuiAddress, recipient: SuiAddress, amount_in: u64) -> Result<TransactionData> {
        // 创建测试用的 Sui 客户端
        let sui = new_test_sui_client().await;

        // 获取指定数量的输入代币
        let coin_in = coin::get_coin(&sui, sender, &self.coin_in_type, amount_in).await?;

        // 构建可编程交易
        let pt = self
            .build_swap_tx(sender, recipient, coin_in.object_ref(), amount_in)
            .await?;

        // 获取 Gas 代币（排除已使用的输入代币）
        let gas_coins = coin::get_gas_coin_refs(&sui, sender, Some(coin_in.coin_object_id)).await?;
        // 获取当前的 Gas 价格
        let gas_price = sui.read_api().get_reference_gas_price().await?;
        // 创建完整的交易数据
        let tx_data = TransactionData::new_programmable(sender, gas_coins, pt, GAS_BUDGET, gas_price);

        Ok(tx_data)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use itertools::Itertools;
    use object_pool::ObjectPool;
    use simulator::DBSimulator;
    use simulator::HttpSimulator;
    use simulator::Simulator;
    use tracing::info;

    use super::*;
    use crate::{
        config::tests::{TEST_ATTACKER, TEST_HTTP_URL},
        defi::{indexer_searcher::IndexerDexSearcher, DexSearcher},
    };

    #[tokio::test]
    async fn test_flowx_swap_tx() {
        mev_logger::init_console_logger_with_directives(None, &["arb=debug", "dex_indexer=debug"]);

        let http_simulator = HttpSimulator::new(TEST_HTTP_URL, &None).await;

        let owner = SuiAddress::from_str(TEST_ATTACKER).unwrap();
        let recipient =
            SuiAddress::from_str("0x0cbe287984143ef232336bb39397bd10607fa274707e8d0f91016dceb31bb829").unwrap();
        let token_in_type = "0x2::sui::SUI";
        let token_out_type = "0x0bffc4f0333fb1256431156395a93fc252432152b0ff732197e8459a365e5a9f::suicat::SUICAT";
        let amount_in = 10000;

        let simulator_pool = Arc::new(ObjectPool::new(1, move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { Box::new(DBSimulator::new_test(true).await) as Box<dyn Simulator> })
        }));

        // find dexes and swap
        let searcher = IndexerDexSearcher::new(TEST_HTTP_URL, simulator_pool).await.unwrap();
        let dexes = searcher
            .find_dexes(token_in_type, Some(token_out_type.into()))
            .await
            .unwrap();
        info!("🧀 dexes_len: {}", dexes.len());
        let dex = dexes
            .into_iter()
            .filter(|dex| dex.protocol() == Protocol::BlueMove)
            .sorted_by(|a, b| a.liquidity().cmp(&b.liquidity()))
            .last()
            .unwrap();
        let tx_data = dex.swap_tx(owner, recipient, amount_in).await.unwrap();
        info!("🧀 tx_data: {:?}", tx_data);

        let response = http_simulator.simulate(tx_data, Default::default()).await.unwrap();
        info!("🧀 {:?}", response);
    }
}
