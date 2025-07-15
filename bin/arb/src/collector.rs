//! 交易收集器模块
//!
//! 本模块实现了多种交易收集器，用于从不同来源收集 Sui 网络上的交易信息：
//! - PublicTxCollector: 从本地 socket 收集公共交易和事件
//! - PrivateTxCollector: 从 WebSocket 收集私有交易（中继服务）
//!
//! 收集器是 MEV 系统的数据入口，负责实时监听和解析交易数据，
//! 为后续的套利策略提供必要的交易信息。

use burberry::{async_trait, Collector, CollectorStream};
use eyre::Result;
use fastcrypto::encoding::{Base64, Encoding};
use futures::stream::StreamExt;
use interprocess::local_socket::{
    tokio::{prelude::*, Stream},
    GenericNamespaced,
};
use serde::Deserialize;
use sui_json_rpc_types::{SuiEvent, SuiTransactionBlockEffects};
use sui_types::{effects::TransactionEffects, transaction::TransactionData};
use tokio::{io::AsyncReadExt, pin, time};
use tracing::{debug, error};

use crate::types::Event;

/// 公共交易收集器
/// 
/// 通过本地 socket 连接收集已执行的公共交易及其事件。
/// 这些交易已经在链上确认，包含完整的执行效果和事件信息。
/// 
/// 数据格式：
/// - 交易效果 (TransactionEffects): 包含交易执行结果
/// - 事件列表 (Vec<SuiEvent>): 交易产生的所有事件
pub struct PublicTxCollector {
    /// 本地 socket 路径
    path: String,
}

impl PublicTxCollector {
    /// 创建新的公共交易收集器
    /// 
    /// # 参数
    /// * `path` - 本地 socket 路径
    pub fn new(path: &str) -> Self {
        Self { path: path.to_string() }
    }

    /// 连接到本地 socket
    /// 
    /// # 返回
    /// * `Result<Stream>` - socket 连接流
    async fn connect(&self) -> Result<Stream> {
        let name = self.path.as_str().to_ns_name::<GenericNamespaced>()?;
        let conn = Stream::connect(name).await?;
        Ok(conn)
    }
}

#[async_trait]
impl Collector<Event> for PublicTxCollector {
    /// 返回收集器名称
    fn name(&self) -> &str {
        "PublicTxCollector"
    }

    /// 获取事件流
    /// 
    /// 从本地 socket 读取交易数据，数据格式为：
    /// 1. effects_len (4 bytes) - 交易效果数据长度
    /// 2. effects_data (effects_len bytes) - 序列化的交易效果
    /// 3. events_len (4 bytes) - 事件数据长度  
    /// 4. events_data (events_len bytes) - JSON 格式的事件列表
    /// 
    /// # 返回
    /// * `Result<CollectorStream<'_, Event>>` - 事件流
    async fn get_event_stream(&self) -> Result<CollectorStream<'_, Event>> {
        let mut conn = self.connect().await?;
        let mut effects_len_buf = [0u8; 4];  // 交易效果长度缓冲区
        let mut events_len_buf = [0u8; 4];   // 事件长度缓冲区

        let stream = async_stream::stream! {
            loop {
                tokio::select! {
                    // 读取交易效果数据
                    result = conn.read_exact(&mut effects_len_buf) => {
                        if result.is_err() {
                            debug!("Failed to read effects length");
                            // 连接断开时自动重连
                            conn = self.connect().await.expect("Failed to reconnect to tx socket");
                            continue;
                        }

                        // 解析交易效果长度并读取数据
                        let effects_len = u32::from_be_bytes(effects_len_buf);
                        let mut effects_buf = vec![0u8; effects_len as usize];
                        if conn.read_exact(&mut effects_buf).await.is_err() {
                            debug!("Failed to read effects");
                            conn = self.connect().await.expect("Failed to reconnect to tx socket");
                            continue;
                        }

                        // 读取事件长度
                        if conn.read_exact(&mut events_len_buf).await.is_err() {
                            debug!("Failed to read events length");
                            conn = self.connect().await.expect("Failed to reconnect to tx socket");
                            continue;
                        }

                        // 解析事件长度并读取事件数据
                        let events_len = u32::from_be_bytes(events_len_buf);
                        let mut events_buf = vec![0u8; events_len as usize];
                        if conn.read_exact(&mut events_buf).await.is_err() {
                            debug!("Failed to read events");
                            conn = self.connect().await.expect("Failed to reconnect to tx socket");
                            continue;
                        }

                        // 反序列化交易效果（使用 bincode 格式）
                        let tx_effects: TransactionEffects = match bincode::deserialize(&effects_buf) {
                            Ok(tx_effects) => tx_effects,
                            Err(e) => {
                                error!("Invalid tx_effects: {:?}", e);
                                continue;
                            }
                        };

                        // 反序列化事件列表（使用 JSON 格式）
                        let events: Vec<SuiEvent> = if events_len == 0 {
                            vec![]  // 无事件的交易
                        } else {
                            match serde_json::from_slice(&events_buf) {
                                Ok(events) => events,
                                Err(e) => {
                                    error!("Invalid events: {:?}", e);
                                    continue;
                                }
                            }
                        };

                        // 转换为 Sui JSON RPC 格式并生成事件
                        if let Ok(tx_effects) = SuiTransactionBlockEffects::try_from(tx_effects) {
                            yield Event::PublicTx(tx_effects, events);
                        }

                    }
                    // 防止 CPU 占用过高的休眠
                    else => {
                        time::sleep(time::Duration::from_millis(10)).await;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }
}

/// 私有交易消息结构
/// 
/// 从中继服务接收的交易消息格式，包含 Base64 编码的交易数据。
/// 这些交易通常是尚未提交到链上的待处理交易。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TxMessage {
    /// Base64 编码的交易字节数据
    tx_bytes: String,
}

impl TryFrom<TxMessage> for TransactionData {
    type Error = eyre::Error;

    /// 将 TxMessage 转换为 TransactionData
    /// 
    /// # 转换流程
    /// 1. Base64 解码交易字节数据
    /// 2. BCS 反序列化为 TransactionData
    /// 
    /// # 参数
    /// * `tx_message` - 包含 Base64 编码交易数据的消息
    /// 
    /// # 返回
    /// * `Result<Self>` - 解析后的交易数据
    fn try_from(tx_message: TxMessage) -> Result<Self> {
        let tx_bytes = Base64::decode(&tx_message.tx_bytes)?;
        let tx_data: TransactionData = bcs::from_bytes(&tx_bytes)?;
        Ok(tx_data)
    }
}

/// 私有交易收集器
/// 
/// 通过 WebSocket 连接从中继服务收集私有交易。
/// 这些交易通常是尚未广播到公共网络的待处理交易，
/// 可能包含 MEV 机会或需要抢跑的交易。
pub struct PrivateTxCollector {
    /// WebSocket 连接 URL
    ws_url: String,
}

impl PrivateTxCollector {
    /// 创建新的私有交易收集器
    /// 
    /// # 参数
    /// * `ws_url` - 中继服务的 WebSocket URL
    pub fn new(ws_url: &str) -> Self {
        Self {
            ws_url: ws_url.to_string(),
        }
    }
}

#[async_trait]
impl Collector<Event> for PrivateTxCollector {
    /// 返回收集器名称
    fn name(&self) -> &str {
        "PrivateTxCollector"
    }

    /// 获取私有交易事件流
    /// 
    /// 连接到中继服务的 WebSocket，接收 JSON 格式的交易消息。
    /// 每个消息包含一个 Base64 编码的交易数据。
    /// 
    /// # 返回
    /// * `Result<CollectorStream<'_, Event>>` - 私有交易事件流
    async fn get_event_stream(&self) -> Result<CollectorStream<'_, Event>> {
        // 连接到中继服务的 WebSocket
        let (ws_stream, _) = tokio_tungstenite::connect_async(&self.ws_url)
            .await
            .expect("Failed to connect to relay server");

        // 分离读写流，只需要读取流
        let (_, read) = ws_stream.split();

        let stream = async_stream::stream! {
            pin!(read);
            // 持续监听 WebSocket 消息
            while let Some(message) = read.next().await {
                let message = match message {
                    Ok(msg) => msg,
                    Err(e) => {
                        error!("Relay websocket error: {:?}", e);
                        continue;
                    }
                };

                // 解析 JSON 格式的交易消息
                let tx_message: TxMessage = serde_json::from_str(message.to_text().unwrap()).unwrap();
                
                // 将消息转换为交易数据
                let tx_data = match TransactionData::try_from(tx_message) {
                    Ok(tx_data) => tx_data,
                    Err(e) => {
                        error!("Invalid tx_message: {:?}", e);
                        continue;
                    }
                };

                // 生成私有交易事件
                yield Event::PrivateTx(tx_data);
            }
        };

        Ok(Box::pin(stream))
    }
}
