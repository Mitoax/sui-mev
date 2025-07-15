//! # Sui MEV 套利系统主程序
//!
//! 这是 Sui 区块链上的 MEV（最大可提取价值）套利系统的主入口点。
//! 系统支持多种运行模式：
//! - StartBot: 启动完整的套利机器人，包含事件监听、策略执行、交易提交
//! - Run: 单次套利机会分析和执行
//! - PoolIds: 生成所有流动性池的对象 ID 文件
//!
//! ## 核心功能
//! - 多协议 DEX 套利（Cetus、Turbos、DeepBook 等）
//! - Shio 竞价机制支持
//! - 实时事件监听和处理
//! - 智能路径优化和利润最大化

mod arb;           // 套利核心算法模块
mod collector;     // 交易事件收集器
mod common;        // 通用工具和搜索功能
mod config;        // 系统配置和常量
mod defi;          // DeFi 协议集成模块
mod executor;      // 交易执行器
mod pool_ids;      // 流动性池 ID 管理
mod start_bot;     // 机器人启动器
mod strategy;      // 套利策略和工作线程
mod types;         // 核心数据类型定义

use clap::Parser;
use eyre::Result;

/// 构建版本信息，用于版本追踪和调试
pub const BUILD_VERSION: &str = version::build_version!();

/// 主程序命令行参数结构
/// 
/// 使用 clap 进行命令行解析，支持多个子命令。
#[derive(clap::Parser)]
pub struct Args {
    /// 要执行的子命令
    #[command(subcommand)]
    pub command: Command,
}

/// HTTP 连接配置
/// 
/// 用于配置与 Sui 节点的 RPC 连接参数。
/// 支持环境变量配置，便于不同环境部署。
#[derive(Clone, Debug, Parser)]
#[command(about = "Common configuration")]
pub struct HttpConfig {
    /// Sui RPC 节点 URL
    /// 
    /// 可通过环境变量 SUI_RPC_URL 设置，默认连接本地节点
    #[arg(long, env = "SUI_RPC_URL", default_value = "http://localhost:9000")]
    pub rpc_url: String,

    /// IPC 路径（已废弃）
    /// 
    /// 保留用于向后兼容，不再使用
    #[arg(long, help = "deprecated")]
    pub ipc_path: Option<String>,
}

/// 支持的子命令枚举
/// 
/// 定义了系统支持的三种主要运行模式，每种模式针对不同的使用场景。
#[derive(clap::Subcommand)]
pub enum Command {
    /// 启动完整的套利机器人
    /// 
    /// 包含事件监听、策略执行、交易提交等完整功能。
    /// 适用于生产环境的持续套利操作。
    StartBot(start_bot::Args),
    
    /// 执行单次套利分析
    /// 
    /// 分析特定交易或状态下的套利机会，适用于测试和调试。
    Run(arb::Args),
    
    /// 生成流动性池对象 ID 文件
    /// 
    /// 扫描并生成所有 DEX 协议中流动性池的对象 ID，
    /// 用于系统初始化和池信息维护。
    PoolIds(pool_ids::Args),
}

/// 程序主入口点
/// 
/// 解析命令行参数并根据子命令分发到相应的处理函数。
/// 使用 tokio 异步运行时支持高并发操作。
/// 
/// # 返回
/// * `Result<()>` - 程序执行结果，错误时包含详细信息
#[tokio::main]
async fn main() -> Result<()> {
    // 解析命令行参数
    let args = Args::parse();

    // 根据子命令分发到相应的处理函数
    match args.command {
        Command::StartBot(args) => {
            // 启动完整的套利机器人
            start_bot::run(args).await
        }
        Command::Run(args) => {
            // 执行单次套利分析
            arb::run(args).await
        }
        Command::PoolIds(args) => {
            // 生成流动性池 ID 文件
            pool_ids::run(args).await
        }
    }
}
