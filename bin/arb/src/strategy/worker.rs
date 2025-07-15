//! # 工作线程模块
//!
//! 实现套利处理的工作线程，负责：
//! - 接收和处理套利任务
//! - 执行套利机会的验证和优化
//! - 提交套利交易和竞价
//! - 发送通知消息
//! - 管理模拟器资源

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use burberry::ActionSubmitter;
use eyre::{bail, ensure, Context, OptionExt, Result};
use object_pool::ObjectPool;
use simulator::{ReplaySimulator, SimulateCtx, Simulator};
use sui_json_rpc_types::SuiTransactionBlockEffectsAPI;
use sui_sdk::SuiClient;
use sui_types::{
    base_types::{ObjectID, SuiAddress},
    object::Owner,
    transaction::{GasData, TransactionData, TransactionDataAPI},
};
use tracing::{error, info, instrument};
use utils::coin;

use crate::{
    arb::{Arb, ArbResult},
    common::notification::new_tg_messages,
    types::{Action, Source},
};

use super::arb_cache::ArbItem;

/// 工作线程结构体
/// 
/// 负责处理套利任务的工作线程，每个线程独立运行，
/// 从通道接收套利项并执行相应的套利操作。
pub struct Worker {
    /// 工作线程ID
    pub _id: usize,
    /// 套利交易的发送者地址
    pub sender: SuiAddress,

    /// 接收套利任务的通道
    pub arb_item_receiver: async_channel::Receiver<ArbItem>,

    /// 模拟器对象池，用于并行模拟
    pub simulator_pool: Arc<ObjectPool<Box<dyn Simulator>>>,
    /// 模拟器名称，用于日志记录
    pub simulator_name: String,

    /// 专用重放模拟器（可选）
    pub dedicated_simulator: Option<Arc<ReplaySimulator>>,

    /// 动作提交器，用于提交生成的动作
    pub submitter: Arc<dyn ActionSubmitter<Action>>,
    /// Sui 客户端
    pub sui: SuiClient,
    /// 套利核心逻辑
    pub arb: Arc<Arb>,
}

impl Worker {
    /// 运行工作线程
    /// 
    /// 主循环，持续监听套利任务并处理。
    /// 
    /// # 返回
    /// * `Result<()>` - 运行结果
    #[tokio::main]
    pub async fn run(mut self) -> Result<()> {
        loop {
            tokio::select! {
                arb_item = self.arb_item_receiver.recv() => {
                    if let Err(error) = self.handle_arb_item(arb_item.context("arb_item channel error")?).await {
                        error!(?error, "Handle arb_item failed");
                    }
                }
                else => bail!("strategy channels undefined behavior"),
            }
        }
    }

    /// 处理套利任务
    /// 
    /// 接收套利项，执行套利机会搜索，验证交易，并提交相应的动作。
    /// 
    /// # 参数
    /// * `arb_item` - 套利任务项
    /// 
    /// # 返回
    /// * `Result<()>` - 处理结果
    #[instrument(skip_all, fields(coin = %arb_item.coin.split("::").nth(2).unwrap_or(&arb_item.coin), tx = %arb_item.tx_digest))]
    pub async fn handle_arb_item(&mut self, arb_item: ArbItem) -> Result<()> {
        let ArbItem {
            coin,
            pool_id,
            tx_digest,
            sim_ctx,
            source,
        } = arb_item;

        // 尝试为该币种找到套利机会
        if let Some((arb_result, elapsed)) = arbitrage_one_coin(
            self.arb.clone(),
            self.sender,
            &coin,
            pool_id,
            sim_ctx.clone(),
            false,
            source,
        )
        .await
        {
            // 验证最终交易数据
            let tx_data = match self.dry_run_tx_data(arb_result.tx_data.clone(), sim_ctx.clone()).await {
                Ok(tx_data) => tx_data,
                Err(error) => {
                    error!(?arb_result, ?error, "Dry run final tx_data failed");
                    return Ok(());
                }
            };

            let arb_tx_digest = tx_data.digest();
            // 根据事件来源创建相应的动作
            let action = match arb_result.source {
                Source::Shio { bid_amount, .. } => Action::ShioSubmitBid((tx_data, bid_amount, tx_digest)),
                _ => Action::ExecutePublicTx(tx_data),
            };

            // 提交套利动作
            self.submitter.submit(action);

            // 发送 Telegram 通知消息
            let tg_msgs = new_tg_messages(tx_digest, arb_tx_digest, &arb_result, elapsed, &self.simulator_name);
            for tg_msg in tg_msgs {
                self.submitter.submit(tg_msg.into());
            }

            // 通知专用模拟器更频繁地更新
            if let Some(dedicated_sim) = &self.dedicated_simulator {
                dedicated_sim.update_notifier.send(()).await.unwrap();
            }
        }

        Ok(())
    }

    /// 验证交易数据
    /// 
    /// 使用最新的对象版本验证交易，确保交易能够成功执行并产生预期收益。
    /// 
    /// # 参数
    /// * `tx_data` - 待验证的交易数据
    /// * `sim_ctx` - 模拟执行上下文
    /// 
    /// # 返回
    /// * `Result<TransactionData>` - 验证后的最终交易数据
    async fn dry_run_tx_data(&self, tx_data: TransactionData, sim_ctx: SimulateCtx) -> Result<TransactionData> {
        // 修复对象引用，使用最新版本
        let tx_data: TransactionData = self.fix_object_refs(tx_data).await?;

        // 选择合适的模拟器进行验证
        let resp = if let Some(dedicated_sim) = &self.dedicated_simulator {
            dedicated_sim.simulate(tx_data.clone(), sim_ctx).await?
        } else {
            self.simulator_pool.get().simulate(tx_data.clone(), sim_ctx).await?
        };

        // 检查交易执行状态
        let status = &resp.effects.status();
        ensure!(status.is_ok(), "Dry run result: {:?}", status);

        // 验证套利者的余额变化
        let bc = &resp
            .balance_changes
            .into_iter()
            .find(|bc| bc.owner == Owner::AddressOwner(self.sender))
            .ok_or_eyre("No balance change for attacker")?;
        ensure!(bc.amount > 0, "Attacker's balance not increased {:?}", bc);

        Ok(tx_data)
    }

    /// 修复对象引用
    /// 
    /// 获取 gas 币的最新对象引用，避免等待索引 API 返回正确的 gas 币。
    /// 
    /// # 参数
    /// * `tx_data` - 原始交易数据
    /// 
    /// # 返回
    /// * `Result<TransactionData>` - 修复后的交易数据
    async fn fix_object_refs(&self, tx_data: TransactionData) -> Result<TransactionData> {
        // 获取最新的 gas 币引用
        let gas_coins = coin::get_gas_coin_refs(&self.sui, self.sender, None).await?;

        // 更新交易数据中的 gas 支付信息
        let mut tx_data = tx_data;
        let gas_data: &mut GasData = tx_data.gas_data_mut();
        gas_data.payment = gas_coins;

        Ok(tx_data)
    }
}

/// 为单个币种执行套利搜索
/// 
/// 尝试为指定币种找到套利机会，包括路径搜索、收益计算和优化。
/// 
/// # 参数
/// * `arb` - 套利核心逻辑
/// * `attacker` - 套利者地址
/// * `coin_type` - 币种类型
/// * `pool_id` - 流动性池ID（可选）
/// * `sim_ctx` - 模拟执行上下文
/// * `use_gss` - 是否使用黄金分割搜索
/// * `source` - 事件来源
/// 
/// # 返回
/// * `Option<(ArbResult, Duration)>` - 套利结果和执行时间（如果找到机会）
async fn arbitrage_one_coin(
    arb: Arc<Arb>,
    attacker: SuiAddress,
    coin_type: &str,
    pool_id: Option<ObjectID>,
    sim_ctx: SimulateCtx,
    use_gss: bool,
    source: Source,
) -> Option<(ArbResult, Duration)> {
    let start = Instant::now();
    // 尝试找到套利机会
    let arb_result = match arb
        .find_opportunity(attacker, coin_type, pool_id, vec![], sim_ctx, use_gss, source)
        .await
    {
        Ok(r) => r,
        Err(error) => {
            let elapsed = start.elapsed();
            // 根据执行时间选择不同的日志颜色
            if elapsed > Duration::from_secs(1) {
                info!(elapsed = ?elapsed, %coin_type, "🥱 \x1b[31mNo opportunity: {error:#}\x1b[0m");
            } else {
                info!(elapsed = ?elapsed, %coin_type, "🥱 No opportunity: {error:#}");
            }
            return None;
        }
    };

    // 记录成功找到套利机会的详细信息
    info!(
        elapsed = ?start.elapsed(),
        elapsed.ctx_creation = ?arb_result.create_trial_ctx_duration,
        elapsed.grid_search = ?arb_result.grid_search_duration,
        elapsed.gss = ?arb_result.gss_duration,
        cache_misses = ?arb_result.cache_misses,
        coin = %coin_type,
        "💰 Profitable opportunity found: {:?}",
        &arb_result.best_trial_result
    );

    Some((arb_result, start.elapsed()))
}
