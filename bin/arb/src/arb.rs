//! Sui MEV 套利机器人核心逻辑
//!
//! 本文件实现了 Sui 区块链上的 MEV (Maximal Extractable Value) 套利机器人的核心算法，
//! 提供以下主要功能：
//!
//! ## 核心功能
//! - **套利机会发现**: 自动检测不同 DEX 之间的价格差异
//! - **路径优化**: 寻找最优的买入和卖出路径组合
//! - **利润最大化**: 使用网格搜索和黄金分割搜索优化投入金额
//! - **交易构建**: 生成可执行的区块链交易数据
//!
//! ## 算法流程
//! 1. **路径发现**: 查找目标代币的所有可能买入和卖出路径
//! 2. **网格搜索**: 使用不同投入金额进行并行试算，找到初步最优解
//! 3. **黄金分割搜索**: 在网格搜索结果基础上进一步优化投入金额
//! 4. **交易构建**: 基于最优结果构建完整的交易数据
//!
//! ## 使用示例
//! ```bash
//! cargo run -r --bin arb run --coin-type \
//!     "0xa8816d3a6e3136e86bc2873b1f94a15cadc8af2703c075f2d546c2ae367f4df9::ocean::OCEAN"
//! ```

use std::{
    fmt,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use clap::Parser;
use eyre::{ensure, ContextCompat, Result};
use itertools::Itertools;
use object_pool::ObjectPool;
use simulator::{HttpSimulator, SimulateCtx, Simulator};
use sui_sdk::SuiClientBuilder;
use sui_types::{
    base_types::{ObjectID, ObjectRef, SuiAddress},
    transaction::TransactionData,
};
use tokio::task::JoinSet;
use tracing::{debug, info, instrument, Instrument};
use utils::coin;

use crate::{
    common::get_latest_epoch,
    common::search::{golden_section_search_maximize, SearchGoal},
    defi::{Defi, Path, TradeType},
    types::Source,
    HttpConfig,
};

/// 套利命令行参数配置
/// 
/// 定义了运行套利机器人所需的所有命令行参数
#[derive(Clone, Debug, Parser)]
pub struct Args {
    /// 目标代币类型的完整路径
    /// 例如: "0x2::sui::SUI" 或自定义代币类型
    #[arg(long)]
    pub coin_type: String,

    /// 可选的特定池子 ID，用于针对特定池子的套利
    /// 如果指定，套利路径必须包含此池子
    #[arg(long)]
    pub pool_id: Option<String>,

    /// 交易发送者地址
    /// 必须是有效的 Sui 地址格式
    #[arg(
        long,
        default_value = ""
    )]
    pub sender: String,

    /// HTTP 配置参数（RPC URL 等）
    #[command(flatten)]
    pub http_config: HttpConfig,
}

/// 运行套利机器人的主函数
/// 
/// 该函数是套利机器人的入口点，负责初始化所有必要的组件并执行套利逻辑。
/// 
/// # 参数
/// * `args` - 命令行参数，包含代币类型、发送者地址等配置
/// 
/// # 执行流程
/// 1. 初始化日志系统
/// 2. 创建模拟器池和 Sui 客户端
/// 3. 获取 Gas 代币和当前 epoch 信息
/// 4. 执行套利机会发现算法
/// 5. 输出结果
pub async fn run(args: Args) -> Result<()> {
    // 初始化控制台日志，启用调试级别的套利和 DEX 索引日志
    mev_logger::init_console_logger_with_directives(None, &["arb=debug", "dex_indexer=debug"]);

    info!("Running arb with {:?}", args);
    let rpc_url = args.http_config.rpc_url.clone();
    let ipc_path = args.http_config.ipc_path.clone();

    // 解析发送者地址
    let sender = SuiAddress::from_str(&args.sender).map_err(|e| eyre::eyre!(e))?;

    // 创建模拟器对象池，用于并发执行交易模拟
    // 每个模拟器实例都连接到 Sui RPC 节点
    let simulator_pool = ObjectPool::new(1, move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { Box::new(HttpSimulator::new(&rpc_url, &ipc_path).await) as Box<dyn Simulator> })
    });

    // 初始化套利引擎和 Sui 客户端
    let arb = Arb::new(&args.http_config.rpc_url, Arc::new(simulator_pool)).await?;
    let sui = SuiClientBuilder::default().build(&args.http_config.rpc_url).await?;
    
    // 获取发送者的 Gas 代币用于支付交易费用
    let gas_coins = coin::get_gas_coin_refs(&sui, sender, None).await?;
    // 获取当前 epoch 信息，用于交易模拟
    let epoch = get_latest_epoch(&sui).await?;
    let sim_ctx = SimulateCtx::new(epoch, vec![]);
    
    // 解析可选的池子 ID
    let pool_id = args.pool_id.as_deref().map(ObjectID::from_hex_literal).transpose()?;

    // 执行套利机会发现算法
    let result = arb
        .find_opportunity(
            sender,
            &args.coin_type,
            pool_id,
            gas_coins,
            sim_ctx,
            true,  // 启用黄金分割搜索优化
            Source::Public,  // 标记为公共交易源
        )
        .await?;

    // 输出详细的套利结果
    info!("{result:#?}");
    Ok(())
}

/// 套利结果数据结构
/// 
/// 包含了套利算法执行的完整结果，包括性能指标和最终交易数据
#[derive(Debug)]
pub struct ArbResult {
    /// 创建试算上下文的耗时
    pub create_trial_ctx_duration: Duration,
    /// 网格搜索的耗时
    pub grid_search_duration: Duration,
    /// 黄金分割搜索的耗时（如果启用）
    pub gss_duration: Option<Duration>,
    /// 最佳试算结果
    pub best_trial_result: TrialResult,
    /// 缓存未命中次数，用于性能分析
    pub cache_misses: u64,
    /// 交易来源标识
    pub source: Source,
    /// 最终构建的交易数据，可直接提交到区块链
    pub tx_data: TransactionData,
}

/// 套利引擎主结构体
/// 
/// 封装了所有套利相关的逻辑和依赖，是套利机器人的核心组件
pub struct Arb {
    /// DeFi 协议交互层，负责与各种 DEX 协议通信
    defi: Defi,
}

impl Arb {
    /// 创建新的套利引擎实例
    /// 
    /// # 参数
    /// * `http_url` - Sui RPC 节点的 HTTP URL
    /// * `simulator_pool` - 交易模拟器对象池，用于并发模拟
    /// 
    /// # 返回值
    /// 返回初始化完成的套利引擎实例
    pub async fn new(http_url: &str, simulator_pool: Arc<ObjectPool<Box<dyn Simulator>>>) -> Result<Self> {
        let defi = Defi::new(http_url, simulator_pool).await?;
        Ok(Self { defi })
    }

    /// 发现套利机会的核心算法
    /// 
    /// 该方法实现了完整的套利机会发现流程，包括路径搜索、网格搜索优化和交易构建。
    /// 
    /// # 参数
    /// * `sender` - 交易发送者地址
    /// * `coin_type` - 目标代币类型
    /// * `pool_id` - 可选的特定池子 ID
    /// * `gas_coins` - Gas 代币引用列表
    /// * `sim_ctx` - 模拟上下文
    /// * `use_gss` - 是否启用黄金分割搜索优化
    /// * `source` - 交易来源标识
    /// 
    /// # 返回值
    /// 返回包含最优套利方案的 ArbResult
    /// 
    /// # 算法流程
    /// 1. 创建试算上下文，查找买入和卖出路径
    /// 2. 网格搜索：并行测试不同投入金额的盈利性
    /// 3. 黄金分割搜索：在网格搜索基础上进一步优化
    /// 4. 构建最终交易数据
    #[allow(clippy::too_many_arguments)]
    pub async fn find_opportunity(
        &self,
        sender: SuiAddress,
        coin_type: &str,
        pool_id: Option<ObjectID>,
        gas_coins: Vec<ObjectRef>,
        sim_ctx: SimulateCtx,
        use_gss: bool,
        source: Source,
    ) -> Result<ArbResult> {
        let gas_price = sim_ctx.epoch.gas_price;

        // 第一阶段：创建试算上下文
        // 这个阶段会查找所有可能的买入和卖出路径
        let (ctx, create_trial_ctx_duration) = {
            let timer = Instant::now();
            let ctx = Arc::new(
                TrialCtx::new(
                    self.defi.clone(),
                    sender,
                    coin_type,
                    pool_id,
                    gas_coins.clone(),
                    sim_ctx,
                )
                .await?,
            );

            (ctx, timer.elapsed())
        };

        // 第二阶段：网格搜索
        // 使用指数级递增的投入金额并行测试，快速找到盈利区间
        let starting_grid = 1_000_000u64; // 0.001 SUI 作为起始网格
        let mut cache_misses = 0;
        let (mut max_trial_res, grid_search_duration) = {
            let timer = Instant::now();
            let mut joinset = JoinSet::new();
            
            // 创建 10 个并行任务，测试从 0.01 SUI 到 100 SUI 的投入金额
            for inc in 1..11 {
                let ctx = ctx.clone();
                let grid = starting_grid.checked_mul(10u64.pow(inc)).context("Grid overflow")?;

                // 在当前 span 中异步执行试算
                joinset.spawn(async move { ctx.trial(grid).await }.in_current_span());
            }

            // 收集所有并行任务的结果，找到最大利润
            let mut max_trial_res = TrialResult::default();
            while let Some(Ok(trial_res)) = joinset.join_next().await {
                if let Ok(trial_res) = trial_res {
                    // 更新缓存未命中统计
                    if trial_res.cache_misses > cache_misses {
                        cache_misses = trial_res.cache_misses;
                    }
                    // 更新最佳结果
                    if trial_res > max_trial_res {
                        max_trial_res = trial_res;
                    }
                }
            }
            (max_trial_res, timer.elapsed())
        };

        // 确保网格搜索找到了盈利机会
        ensure!(
            max_trial_res.profit > 0,
            "cache_misses: {}. No profitable grid found",
            cache_misses
        );

        // 第三阶段：黄金分割搜索（可选）
        // 在网格搜索结果的基础上进行精细优化，寻找真正的最优投入金额
        let gss_duration = if use_gss {
            let timer = Instant::now();
            
            // 设置搜索边界：以网格搜索结果为中心，扩展 10 倍范围
            let upper_bound = max_trial_res.amount_in.saturating_mul(10);
            let lower_bound = max_trial_res.amount_in.saturating_div(10);

            // 执行黄金分割搜索，寻找利润最大化的投入金额
            let goal = TrialGoal;
            let (_, _, trial_res) = golden_section_search_maximize(lower_bound, upper_bound, goal, &ctx).await;
            
            // 更新统计信息和最佳结果
            if trial_res.cache_misses > cache_misses {
                cache_misses = trial_res.cache_misses;
            }
            if trial_res > max_trial_res {
                max_trial_res = trial_res;
            }

            Some(timer.elapsed())
        } else {
            None
        };

        // 最终验证：确保找到了盈利的交易路径
        ensure!(
            max_trial_res.profit > 0,
            "cache_misses: {}. No profitable trade path found",
            cache_misses
        );

        // 提取最佳试算结果的关键信息
        let TrialResult {
            amount_in,
            trade_path,
            profit,
            ..
        } = &max_trial_res;

        // 更新交易来源信息
        let mut source = source;
        if source.deadline().is_some() {
            // 记录套利机会发现的时间戳
            source = source.with_arb_found_time(utils::current_time_ms());
        }
        // 设置竞价金额为利润的 90%（TODO: 使其可配置）
        source = source.with_bid_amount(*profit / 10 * 9);

        // 第四阶段：构建最终交易数据
        // 基于最优路径和投入金额生成可执行的区块链交易
        let tx_data = self
            .defi
            .build_final_tx_data(sender, *amount_in, trade_path, gas_coins, gas_price, source)
            .await?;

        // 返回完整的套利结果
        Ok(ArbResult {
            create_trial_ctx_duration,
            grid_search_duration,
            gss_duration,
            best_trial_result: max_trial_res,
            cache_misses,
            source,
            tx_data,
        })
    }
}

/// 套利试算上下文
/// 
/// 包含执行套利试算所需的所有上下文信息，包括交易路径、模拟环境等
pub struct TrialCtx {
    /// DeFi 协议交互层
    defi: Defi,
    /// 交易发送者地址
    sender: SuiAddress,
    /// 目标代币类型
    coin_type: String,
    /// 可选的特定池子 ID，如果指定则路径必须包含此池子
    pool_id: Option<ObjectID>,
    /// 买入路径列表，用于获取目标代币
    buy_paths: Vec<Path>,
    /// 卖出路径列表，用于出售目标代币
    sell_paths: Vec<Path>,
    /// Gas 代币引用列表
    gas_coins: Vec<ObjectRef>,
    /// 模拟上下文，包含 epoch 信息等
    sim_ctx: SimulateCtx,
}

impl TrialCtx {
    /// 创建新的试算上下文
    /// 
    /// 该方法会查找所有可能的买入和卖出路径，为后续的套利试算做准备。
    /// 
    /// # 参数
    /// * `defi` - DeFi 协议交互层
    /// * `sender` - 交易发送者地址
    /// * `coin_type` - 目标代币类型
    /// * `pool_id` - 可选的特定池子 ID
    /// * `gas_coins` - Gas 代币引用列表
    /// * `sim_ctx` - 模拟上下文
    /// 
    /// # 返回值
    /// 返回初始化完成的试算上下文
    pub async fn new(
        defi: Defi,
        sender: SuiAddress,
        coin_type: &str,
        pool_id: Option<ObjectID>,
        gas_coins: Vec<ObjectRef>,
        sim_ctx: SimulateCtx,
    ) -> Result<Self> {
        // 查找买入路径：从 SUI 到目标代币
        let buy_paths = defi.find_buy_paths(coin_type).await?;
        ensure!(!buy_paths.is_empty(), "no buy paths found for {}", coin_type);

        // 查找卖出路径：从目标代币到 SUI
        let sell_paths = defi.find_sell_paths(coin_type).await?;
        ensure!(!sell_paths.is_empty(), "no sell paths found for {}", coin_type);

        // 如果指定了特定池子，验证至少有一条路径包含该池子
        if pool_id.is_some() {
            let buy_paths_contain_pool = buy_paths.iter().any(|p| p.contains_pool(pool_id));
            let sell_paths_contain_pool = sell_paths.iter().any(|p| p.contains_pool(pool_id));
            ensure!(
                buy_paths_contain_pool || sell_paths_contain_pool,
                "no paths found for the fluctuating pool: {:?}",
                pool_id
            );
        }

        Ok(Self {
            defi,
            sender,
            coin_type: coin_type.to_string(),
            pool_id,
            buy_paths,
            sell_paths,
            gas_coins,
            sim_ctx,
        })
    }

    /// 执行单次套利试算
    /// 
    /// 该方法模拟完整的套利流程：买入目标代币 -> 卖出目标代币，计算最终利润。
    /// 
    /// # 参数
    /// * `amount_in` - 投入的 SUI 数量（以最小单位计算）
    /// 
    /// # 返回值
    /// 返回包含利润、路径等信息的试算结果
    /// 
    /// # 算法流程
    /// 1. 查找最佳买入路径（SUI -> 目标代币）
    /// 2. 查找最佳卖出路径（目标代币 -> SUI）
    /// 3. 计算净利润
    #[instrument(
        name = "trial",
        skip_all,
        fields(
            in = %format!("{:<15}", (amount_in as f64 / 1_000_000_000.0)),
            len = %format!("{:<2}", self.buy_paths.len()),
            action="init"
        )
    )]
    pub async fn trial(&self, amount_in: u64) -> Result<TrialResult> {
        // 第一步：查找最佳买入路径
        // 使用指定的 SUI 数量购买目标代币
        tracing::Span::current().record("action", "buy");

        let timer = Instant::now();
        let best_buy_res = self
            .defi
            .find_best_path_exact_in(
                &self.buy_paths,
                self.sender,
                amount_in,
                TradeType::Swap,
                &self.gas_coins,
                &self.sim_ctx,
            )
            .await?;
        let buy_elapsed = timer.elapsed();

        // 第二步：构建完整的交易路径
        // 将买入路径与卖出路径组合，确保路径不重叠且包含指定池子
        let timer = Instant::now();
        let best_buy_path = best_buy_res.path;
        let buy_path_contains_pool = best_buy_path.contains_pool(self.pool_id);
        let trade_paths = self
            .sell_paths
            .iter()
            .filter_map(|p| {
                // 路径组合规则：
                // - 买入路径和卖出路径不能有共同的池子（避免冲突）
                // - 买入路径或卖出路径必须包含指定的池子（如果有的话）
                if best_buy_path.is_disjoint(p) && (buy_path_contains_pool || p.contains_pool(self.pool_id)) {
                    let mut path = best_buy_path.clone();
                    path.path.extend(p.path.clone());
                    Some(path)
                } else {
                    None
                }
            })
            .collect_vec();
        ensure!(
            !trade_paths.is_empty(),
            "no trade paths found for coin {}, pool_id: {:?}",
            self.coin_type,
            self.pool_id
        );

        // 第三步：执行完整的套利路径模拟
        // 使用闪电贷模式执行买入->卖出的完整流程
        tracing::Span::current().record("action", "sell");
        let best_trade_res = self
            .defi
            .find_best_path_exact_in(
                &trade_paths,
                self.sender,
                amount_in,
                TradeType::Flashloan,
                &self.gas_coins,
                &self.sim_ctx,
            )
            .await?;

        // 计算最终结果和性能指标
        let sell_elapsed = timer.elapsed();
        debug!(coin_type = ?self.coin_type, result = %best_trade_res, ?buy_elapsed, ?sell_elapsed, "trial result");

        let profit = best_trade_res.profit();
        if profit <= 0 {
            return Ok(TrialResult::default());
        }

        let result = TrialResult::new(
            &self.coin_type,
            amount_in,
            profit as u64,
            best_trade_res.path,
            best_trade_res.cache_misses,
        );

        Ok(result)
    }
}

/// 单次套利试算结果
/// 
/// 存储一次套利试算的完整结果，包括投入金额、利润、交易路径等信息。
/// 实现了比较 trait，可以根据利润大小进行排序。
#[derive(Debug, Default, Clone)]
pub struct TrialResult {
    /// 目标代币类型
    pub coin_type: String,
    /// 投入的 SUI 数量（最小单位）
    pub amount_in: u64,
    /// 预期利润（最小单位）
    pub profit: u64,
    /// 完整的交易路径（买入路径 + 卖出路径）
    pub trade_path: Path,
    /// 缓存未命中次数，用于性能分析
    pub cache_misses: u64,
}

impl PartialOrd for TrialResult {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.profit.partial_cmp(&other.profit)
    }
}

impl PartialEq for TrialResult {
    fn eq(&self, other: &Self) -> bool {
        self.profit == other.profit
    }
}

impl TrialResult {
    /// 创建新的试算结果
    /// 
    /// # 参数
    /// * `coin_type` - 目标代币类型
    /// * `amount_in` - 投入金额
    /// * `profit` - 预期利润
    /// * `trade_path` - 交易路径
    /// * `cache_misses` - 缓存未命中次数
    pub fn new(coin_type: &str, amount_in: u64, profit: u64, trade_path: Path, cache_misses: u64) -> Self {
        Self {
            coin_type: coin_type.to_string(),
            amount_in,
            profit,
            trade_path,
            cache_misses,
        }
    }
}

/// 为 TrialResult 实现格式化显示
/// 
/// 提供人类可读的试算结果格式，便于调试和日志输出
impl fmt::Display for TrialResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TrialResult {{ coin_type: {}, amount_in: {}, profit: {}, trade_path: {:?} ... }}",
            self.coin_type, self.amount_in, self.profit, self.trade_path
        )
    }
}

/// 黄金分割搜索的目标函数
/// 
/// 实现 SearchGoal trait，用于在黄金分割搜索算法中评估不同投入金额的盈利性
pub struct TrialGoal;

/// 为 TrialGoal 实现 SearchGoal trait
/// 
/// 该实现定义了如何评估特定投入金额的套利潜力，返回利润作为优化目标
#[async_trait]
impl SearchGoal<TrialCtx, u64, TrialResult> for TrialGoal {
    /// 评估指定投入金额的套利效果
    /// 
    /// # 参数
    /// * `amount_in` - 投入金额
    /// * `ctx` - 试算上下文
    /// 
    /// # 返回值
    /// 返回 (利润, 完整试算结果) 元组
    async fn evaluate(&self, amount_in: u64, ctx: &TrialCtx) -> (u64, TrialResult) {
        let trial_res = ctx.trial(amount_in).await.unwrap_or_default();
        (trial_res.profit, trial_res)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use simulator::{DBSimulator, HttpSimulator, Simulator};
    use sui_types::base_types::SuiAddress;

    use super::*;
    use crate::config::tests::{TEST_ATTACKER, TEST_HTTP_URL};

    #[tokio::test]
    async fn test_find_best_trade_path() {
        mev_logger::init_console_logger_with_directives(None, &["arb=debug"]);

        let simulator_pool = ObjectPool::new(1, move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { Box::new(HttpSimulator::new(&TEST_HTTP_URL, &None).await) as Box<dyn Simulator> })
        });

        let start = Instant::now();

        let sender = SuiAddress::from_str(TEST_ATTACKER).unwrap();
        let sui = SuiClientBuilder::default().build(TEST_HTTP_URL).await.unwrap();
        let epoch = get_latest_epoch(&sui).await.unwrap();
        let sim_ctx = SimulateCtx::new(epoch, vec![]);

        let gas_coins = coin::get_gas_coin_refs(&sui, sender, None).await.unwrap();
        let arb = Arb::new(TEST_HTTP_URL, Arc::new(simulator_pool)).await.unwrap();
        let coin_type = "0xce7ff77a83ea0cb6fd39bd8748e2ec89a3f41e8efdc3f4eb123e0ca37b184db2::buck::BUCK";

        let arb_res = arb
            .find_opportunity(
                sender,
                coin_type,
                None,
                gas_coins,
                sim_ctx.clone(),
                true,
                Source::Public,
            )
            .await
            .unwrap();
        info!(?arb_res, "Best trade path");

        info!("Creating DB simulator ...");
        let db_sim: Arc<dyn Simulator> = Arc::new(DBSimulator::new_default_slow().await);
        info!("DB simulator created in {:?}", start.elapsed());

        let tx_data = arb_res.tx_data;
        let http_sim: Arc<dyn Simulator> = Arc::new(HttpSimulator::new(TEST_HTTP_URL, &None).await);

        let http_res = http_sim.simulate(tx_data.clone(), sim_ctx.clone()).await.unwrap();
        info!(?http_res, "🧀 HTTP simulation result");

        let db_res = db_sim.simulate(tx_data, sim_ctx).await.unwrap();
        info!(?db_res, "🧀 DB simulation result");
    }
}
