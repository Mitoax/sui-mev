//! 交易执行器模块
//!
//! 本模块实现了公共交易执行器，负责将构建好的交易数据提交到 Sui 网络。
//! 执行器是 MEV 系统的输出端，将套利策略生成的交易实际执行到链上。
//!
//! 主要功能：
//! - 交易签名：使用私钥对交易数据进行签名
//! - 交易提交：通过 RPC 接口将交易提交到 Sui 网络
//! - 结果监控：跟踪交易执行状态和结果

use async_trait::async_trait;
use burberry::Executor;
use eyre::Result;
use fastcrypto::hash::HashFunction;
use shared_crypto::intent::{Intent, IntentMessage};
use sui_json_rpc_types::{SuiTransactionBlockResponse, SuiTransactionBlockResponseOptions};
use sui_sdk::{SuiClient, SuiClientBuilder};
use sui_types::{
    crypto::{Signer, SuiKeyPair},
    signature::GenericSignature,
    transaction::{Transaction, TransactionData},
};
use tracing::info;

/// 公共交易执行器
/// 
/// 负责将交易数据签名并提交到 Sui 网络的公共内存池。
/// 这是套利交易的最终执行环节，直接影响套利的成功率和时效性。
/// 
/// 执行流程：
/// 1. 接收 TransactionData
/// 2. 创建 Intent 消息
/// 3. 计算交易哈希
/// 4. 使用私钥签名
/// 5. 构建完整交易
/// 6. 提交到网络
pub struct PublicTxExecutor {
    /// Sui 客户端，用于与网络通信
    sui: SuiClient,
    /// 签名密钥对
    keypair: SuiKeyPair,
}

impl PublicTxExecutor {
    /// 创建新的公共交易执行器
    /// 
    /// # 参数
    /// * `rpc_url` - Sui 网络的 RPC 端点 URL
    /// * `keypair` - 用于签名交易的密钥对
    /// 
    /// # 返回
    /// * `Result<Self>` - 初始化后的执行器实例
    pub async fn new(rpc_url: &str, keypair: SuiKeyPair) -> Result<Self> {
        let sui = SuiClientBuilder::default().build(rpc_url).await?;
        Ok(Self { sui, keypair })
    }

    /// 执行交易并返回响应
    /// 
    /// 完整的交易执行流程，包括签名、提交和结果获取。
    /// 
    /// # 参数
    /// * `tx_data` - 待执行的交易数据
    /// 
    /// # 返回
    /// * `Result<SuiTransactionBlockResponse>` - 交易执行响应
    /// 
    /// # 执行步骤
    /// 1. 创建 Intent 消息包装交易数据
    /// 2. 序列化消息并计算哈希
    /// 3. 使用私钥对哈希进行签名
    /// 4. 构建完整的签名交易
    /// 5. 通过 quorum driver 提交到网络
    pub async fn execute_tx(&self, tx_data: TransactionData) -> Result<SuiTransactionBlockResponse> {
        // 创建 Intent 消息，标识这是一个 Sui 交易
        let intent_msg = IntentMessage::new(Intent::sui_transaction(), tx_data);
        
        // 序列化 Intent 消息为字节数组
        let raw_tx = bcs::to_bytes(&intent_msg)?;

        // 计算交易哈希用于签名
        let digest = {
            let mut hasher = sui_types::crypto::DefaultHash::default();
            hasher.update(raw_tx.clone());
            hasher.finalize().digest
        };

        // 使用私钥对交易哈希进行签名
        let sig = self.keypair.sign(&digest);
        
        // 构建包含签名的完整交易
        let tx = Transaction::from_generic_sig_data(intent_msg.value, vec![GenericSignature::Signature(sig)]);

        // 设置交易响应选项（使用默认配置）
        let options = SuiTransactionBlockResponseOptions::default();
        
        // 通过 quorum driver 提交交易到网络
        let tx_resp = self
            .sui
            .quorum_driver_api()
            .execute_transaction_block(tx, options, None)
            .await?;

        Ok(tx_resp)
    }
}

#[async_trait]
impl Executor<TransactionData> for PublicTxExecutor {
    /// 返回执行器名称
    fn name(&self) -> &str {
        "PublicTxExecutor"
    }

    /// 执行交易动作
    /// 
    /// 实现 Executor trait 的核心方法，负责执行传入的交易数据。
    /// 这是 burberry 框架调用的统一接口。
    /// 
    /// # 参数
    /// * `action` - 要执行的交易数据
    /// 
    /// # 返回
    /// * `Result<()>` - 执行结果（成功或失败）
    async fn execute(&self, action: TransactionData) -> Result<()> {
        // 执行交易并获取响应
        let resp = self.execute_tx(action).await?;
        
        // 提取交易摘要（Base58 编码）
        let digest = resp.digest.base58_encode();

        // 记录交易执行结果
        info!(?digest, status_ok = ?resp.status_ok(), "Executed tx");
        Ok(())
    }
}
