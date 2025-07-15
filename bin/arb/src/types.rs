//! 类型定义模块
//!
//! 本模块定义了 MEV 套利系统中使用的核心数据类型：
//! - Action: 系统可执行的动作类型（通知、交易执行、Shio 竞价）
//! - Event: 系统监听的事件类型（公共交易、私有交易、Shio 事件）
//! - Source: 套利机会的来源类型（公共、Shio、超时）
//!
//! 这些类型构成了整个 MEV 系统的数据流基础，定义了
//! 事件收集、策略处理和动作执行之间的接口。

use std::fmt;

use burberry::executor::telegram_message::Message;
use shio::ShioItem;
use sui_json_rpc_types::{SuiEvent, SuiTransactionBlockEffects};
use sui_types::{digests::TransactionDigest, transaction::TransactionData};

/// 系统动作枚举
/// 
/// 定义了 MEV 系统可以执行的所有动作类型。
/// 这些动作由策略生成，由相应的执行器处理。
/// 
/// # 动作类型
/// - `NotifyViaTelegram`: 发送 Telegram 通知消息
/// - `ExecutePublicTx`: 执行公共交易到内存池
/// - `ShioSubmitBid`: 向 Shio 提交竞价交易
#[derive(Debug, Clone)]
pub enum Action {
    /// 通过 Telegram 发送通知消息
    /// 
    /// 用于发送套利结果、系统状态等通知信息
    NotifyViaTelegram(Message),
    
    /// 执行公共交易
    /// 
    /// 将交易数据提交到 Sui 网络的公共内存池
    ExecutePublicTx(TransactionData),
    
    /// 提交 Shio 竞价
    /// 
    /// 向 Shio 系统提交竞价，包含：
    /// - 交易数据
    /// - 竞价金额
    /// - 目标交易摘要
    ShioSubmitBid((TransactionData, u64, TransactionDigest)),
}

// 为 Action 实现便捷的类型转换

impl From<Message> for Action {
    /// 将 Telegram 消息转换为通知动作
    fn from(msg: Message) -> Self {
        Self::NotifyViaTelegram(msg)
    }
}

impl From<TransactionData> for Action {
    /// 将交易数据转换为公共交易执行动作
    fn from(tx_data: TransactionData) -> Self {
        Self::ExecutePublicTx(tx_data)
    }
}

impl From<(TransactionData, u64, TransactionDigest)> for Action {
    /// 将竞价参数转换为 Shio 竞价动作
    /// 
    /// # 参数
    /// * `tx_data` - 交易数据
    /// * `bid_amount` - 竞价金额
    /// * `opp_tx_digest` - 目标交易摘要
    fn from((tx_data, bid_amount, opp_tx_digest): (TransactionData, u64, TransactionDigest)) -> Self {
        Self::ShioSubmitBid((tx_data, bid_amount, opp_tx_digest))
    }
}

/// 系统事件枚举
/// 
/// 定义了 MEV 系统监听的所有事件类型。
/// 这些事件由收集器产生，由策略处理以发现套利机会。
/// 
/// # 事件类型
/// - `PublicTx`: 已执行的公共交易及其事件
/// - `PrivateTx`: 尚未执行的私有交易数据
/// - `Shio`: Shio 系统的竞价事件
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Event {
    /// 公共交易事件
    /// 
    /// 包含已在链上执行的交易效果和相关事件。
    /// 用于分析市场状态变化和发现套利机会。
    /// 
    /// # 字段
    /// - `SuiTransactionBlockEffects`: 交易执行效果
    /// - `Vec<SuiEvent>`: 交易产生的事件列表
    PublicTx(SuiTransactionBlockEffects, Vec<SuiEvent>),
    
    /// 私有交易事件
    /// 
    /// 包含尚未广播到公共网络的交易数据。
    /// 通常来自中继服务，可能包含 MEV 机会。
    PrivateTx(TransactionData),
    
    /// Shio 竞价事件
    /// 
    /// 来自 Shio 系统的竞价机会，包含时间限制和竞价要求。
    Shio(ShioItem),
}

/// 套利机会来源枚举
/// 
/// 标识套利机会的来源和相关的时间约束信息。
/// 不同来源有不同的执行策略和时间要求。
/// 
/// # 来源类型
/// - `Public`: 来自公共交易的套利机会
/// - `Shio`: 来自 Shio 竞价的套利机会（有时间限制）
/// - `ShioDeadlineMissed`: Shio 竞价超时的套利机会
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Source {
    /// 公共套利机会
    /// 
    /// 来自公共交易池的套利机会，没有时间限制。
    Public,
    
    /// Shio 竞价套利机会
    /// 
    /// 来自 Shio 系统的竞价机会，有严格的时间限制。
    /// 必须在截止时间前完成竞价和执行。
    Shio {
        /// 目标交易摘要
        opp_tx_digest: TransactionDigest,
        /// 竞价金额（单位：MIST）
        bid_amount: u64,
        /// 竞价开始时间（毫秒时间戳）
        start: u64,
        /// 发现套利机会的时间（毫秒时间戳）
        arb_found: u64,
        /// 竞价截止时间（毫秒时间戳）
        deadline: u64,
    },
    
    /// Shio 竞价超时
    /// 
    /// 发现套利机会时已超过 Shio 竞价截止时间。
    /// 这种情况下无法参与竞价，但可以记录用于分析。
    ShioDeadlineMissed {
        /// 竞价开始时间（毫秒时间戳）
        start: u64,
        /// 发现套利机会的时间（毫秒时间戳）
        arb_found: u64,
        /// 竞价截止时间（毫秒时间戳）
        deadline: u64,
    },
}

impl fmt::Display for Source {
    /// 格式化 Source 为可读字符串
    /// 
    /// 提供详细的时间信息，便于调试和日志记录。
    /// 包含竞价时间窗口、发现时间、提前/超时时间等关键指标。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Public => write!(f, "Public"),
            Source::Shio {
                start,
                arb_found,
                deadline,
                ..
            } => write!(
                f,
                "Shio(start={}, deadline={}, time_window={}ms, arb_found={}, early={}ms)",
                *start,
                *deadline,
                (*deadline).saturating_sub(*start),  // 总时间窗口
                *arb_found,
                (*deadline).saturating_sub(*arb_found)  // 提前完成时间
            ),
            Source::ShioDeadlineMissed {
                start,
                arb_found,
                deadline,
            } => write!(
                f,
                "ShioDeadlineMissed(start={}, deadline={}, time_window={}ms, arb_found={}, overdue={}ms)",
                *start,
                *deadline,
                (*deadline).saturating_sub(*start),  // 总时间窗口
                *arb_found,
                (*arb_found).saturating_sub(*deadline)  // 超时时间
            ),
        }
    }
}

impl Source {
    /// 检查是否为 Shio 竞价来源
    /// 
    /// # 返回
    /// * `bool` - 如果是 Shio 竞价则返回 true
    pub fn is_shio(&self) -> bool {
        matches!(self, Source::Shio { .. })
    }

    /// 获取目标交易摘要
    /// 
    /// 仅对 Shio 竞价有效，返回需要竞价的目标交易摘要。
    /// 
    /// # 返回
    /// * `Option<TransactionDigest>` - Shio 竞价的目标交易摘要
    pub fn opp_tx_digest(&self) -> Option<TransactionDigest> {
        match self {
            Source::Shio { opp_tx_digest, .. } => Some(*opp_tx_digest),
            _ => None,
        }
    }

    /// 获取竞价截止时间
    /// 
    /// 仅对 Shio 竞价有效，返回竞价的截止时间戳。
    /// 
    /// # 返回
    /// * `Option<u64>` - 竞价截止时间（毫秒时间戳）
    pub fn deadline(&self) -> Option<u64> {
        match self {
            Source::Shio { deadline, .. } => Some(*deadline),
            _ => None,
        }
    }

    /// 获取竞价金额
    /// 
    /// 返回 Shio 竞价的金额，非 Shio 来源返回 0。
    /// 
    /// # 返回
    /// * `u64` - 竞价金额（单位：MIST）
    pub fn bid_amount(&self) -> u64 {
        match self {
            Source::Shio { bid_amount, .. } => *bid_amount,
            _ => 0,
        }
    }

    /// 更新竞价金额
    /// 
    /// 仅对 Shio 竞价有效，更新竞价金额并返回新的 Source。
    /// 非 Shio 来源保持不变。
    /// 
    /// # 参数
    /// * `bid_amount` - 新的竞价金额（单位：MIST）
    /// 
    /// # 返回
    /// * `Self` - 更新后的 Source
    pub fn with_bid_amount(self, bid_amount: u64) -> Self {
        match self {
            Source::Shio {
                opp_tx_digest,
                start,
                deadline,
                arb_found,
                ..
            } => Source::Shio {
                opp_tx_digest,
                bid_amount,
                start,
                deadline,
                arb_found,
            },
            _ => self,
        }
    }

    /// 设置套利发现时间
    /// 
    /// 仅对 Shio 竞价有效，设置发现套利机会的时间。
    /// 如果发现时间超过截止时间，会自动转换为 ShioDeadlineMissed。
    /// 
    /// # 参数
    /// * `arb_found` - 发现套利机会的时间（毫秒时间戳）
    /// 
    /// # 返回
    /// * `Self` - 更新后的 Source（可能转换为 ShioDeadlineMissed）
    pub fn with_arb_found_time(self, arb_found: u64) -> Self {
        match self {
            Source::Shio {
                opp_tx_digest,
                start,
                deadline,
                bid_amount,
                ..
            } => {
                if arb_found < deadline {
                    // 在截止时间前发现，保持 Shio 状态
                    Source::Shio {
                        opp_tx_digest,
                        bid_amount,
                        start,
                        arb_found,
                        deadline,
                    }
                } else {
                    // 超过截止时间，转换为超时状态
                    Source::ShioDeadlineMissed {
                        start,
                        arb_found,
                        deadline,
                    }
                }
            }
            _ => self,
        }
    }
}
