//! Sui MEV 套利机器人启动器
//!
//! 本文件实现了 Sui MEV 套利机器人的完整启动流程，负责初始化和协调所有核心组件：
//!
//! ## 核心组件
//! - **事件收集器**: 监听公共交易、私有交易和 Shio 事件
//! - **交易执行器**: 处理不同类型的交易提交
//! - **模拟器池**: 提供高性能的交易模拟能力
//! - **套利策略**: 核心的套利逻辑和决策引擎
//! - **通知系统**: Telegram 消息推送
//!
//! ## 架构特点
//! - **多数据源**: 支持公共交易池、私有中继和 Shio 协议
//! - **高并发**: 使用对象池和工作线程实现并发处理
//! - **灵活配置**: 支持数据库模拟器和 HTTP 模拟器切换
//! - **实时监控**: 集成心跳检测和日志系统

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use ::utils::heartbeat;
use burberry::{executor::telegram_message::TelegramMessageDispatcher, map_collector, map_executor, Engine};
use clap::Parser;
use eyre::Result;
use object_pool::ObjectPool;
use shio::{new_shio_collector_and_executor, ShioRPCExecutor};
use simulator::{DBSimulator, HttpSimulator, ReplaySimulator, Simulator};
use sui_types::{base_types::SuiAddress, crypto::SuiKeyPair};
use tracing::{info, warn};

use crate::{
    collector::{PrivateTxCollector, PublicTxCollector},
    executor::PublicTxExecutor,
    strategy::ArbStrategy,
    types::{Action, Event},
    HttpConfig,
};

/// 套利机器人启动参数配置
/// 
/// 包含了启动套利机器人所需的所有配置参数，支持从命令行和环境变量读取
#[derive(Clone, Debug, Parser)]
pub struct Args {
    /// 私钥字符串，用于签名交易
    /// 可通过环境变量 SUI_PRIVATE_KEY 设置
    #[arg(long, env = "SUI_PRIVATE_KEY")]
    pub private_key: String,

    /// 是否使用 RPC 方式提交 Shio 竞价
    /// 默认使用 WebSocket 连接
    #[arg(long, help = "shio executor uses RPC to submit bid")]
    pub shio_use_rpc: bool,

    /// HTTP 连接配置（RPC URL 等）
    #[command(flatten)]
    pub http_config: HttpConfig,

    /// 事件收集器配置
    #[command(flatten)]
    collector_config: CollectorConfig,

    /// 数据库模拟器配置
    #[command(flatten)]
    db_sim_config: DbSimConfig,

    /// 工作线程配置
    #[command(flatten)]
    worker_config: WorkerConfig,
}

/// 事件收集器配置
/// 
/// 定义了不同类型事件收集器的连接参数
#[derive(Clone, Debug, Parser)]
struct CollectorConfig {
    /// 中继交易收集器的 WebSocket URL
    /// 与公共交易收集器互斥使用
    #[arg(long)]
    pub relay_ws_url: Option<String>,

    /// Shio 协议收集器的 WebSocket URL
    /// 用于接收 Shio 拍卖事件
    #[arg(long)]
    pub shio_ws_url: Option<String>,

    /// 公共交易收集器的 Unix Socket 路径
    /// 用于监听 Sui 节点的交易事件
    #[arg(long, env = "SUI_TX_SOCKET_PATH", default_value = "/tmp/sui_tx.sock")]
    pub tx_socket_path: String,
}

/// 数据库模拟器配置
/// 
/// 配置高性能的数据库模拟器，提供比 HTTP 模拟器更快的交易模拟能力
#[derive(Clone, Debug, Parser)]
struct DbSimConfig {
    /// Sui 数据库路径
    /// 数据库模拟器直接读取节点数据库以获得最佳性能
    #[arg(long, env = "SUI_DB_PATH", default_value = "/home/ubuntu/sui/db/live/store")]
    pub db_path: String,

    /// Sui 节点配置文件路径
    /// 用于初始化数据库模拟器
    #[arg(long, env = "SUI_CONFIG_PATH", default_value = "/home/ubuntu/sui/fullnode.yaml")]
    pub config_path: String,

    /// 缓存更新 Socket 路径
    /// 数据库模拟器通过此 Socket 接收 Sui 节点的对象变更通知
    #[arg(long, env = "SUI_UPDATE_CACHE_SOCKET", default_value = "/tmp/sui_cache_updates.sock")]
    pub update_cache_socket: String,

    /// 池子相关对象预加载文件路径
    /// 包含需要预加载到缓存的对象 ID 列表
    #[arg(
        long,
        env = "SUI_PRELOAD_PATH",
        default_value = "/home/ubuntu/suiflow-relay/pool_related_ids.txt"
    )]
    pub preload_path: String,

    /// 是否使用数据库模拟器
    /// false 时使用 HTTP 模拟器（仅用于测试）
    #[arg(long, default_value_t = false)]
    pub use_db_simulator: bool,

    /// 追赶间隔（秒）
    /// 数据库模拟器定期同步最新状态的间隔
    #[arg(long, default_value_t = 60)]
    pub catchup_interval: u64,
}

/// 工作线程和性能配置
/// 
/// 控制套利机器人的并发性能和资源使用
#[derive(Clone, Debug, Parser)]
struct WorkerConfig {
    /// 事件处理工作线程数量
    /// 用于并发处理公共交易、私有交易和 Shio 事件
    /// 通常 8 个线程足够
    #[arg(long, default_value_t = 8)]
    pub workers: usize,

    /// 模拟器池中的模拟器数量
    /// 更多模拟器可以提高并发处理能力，但会消耗更多内存
    #[arg(long, default_value_t = 32)]
    pub num_simulators: usize,

    /// 最近套利记录的最大数量
    /// 如果某个代币在最近 N 次处理中已被处理过，将被忽略以避免重复计算
    #[arg(long, default_value_t = 20)]
    pub max_recent_arbs: usize,

    /// 专用模拟器的短间隔（毫秒）
    /// 用于高频率的快速模拟
    #[arg(long, default_value_t = 50)]
    pub dedicated_short_interval: u64,

    /// 专用模拟器的长间隔（毫秒）
    /// 用于较复杂的模拟计算
    #[arg(long, default_value_t = 200)]
    pub dedicated_long_interval: u64,
}

/// 启动套利机器人的主函数
/// 
/// 该函数是整个套利机器人系统的入口点，负责初始化所有组件并启动事件处理引擎。
/// 
/// # 参数
/// * `args` - 启动参数配置
/// 
/// # 执行流程
/// 1. 初始化日志和错误处理
/// 2. 解析私钥和地址
/// 3. 配置事件收集器（公共/私有/Shio）
/// 4. 配置交易执行器
/// 5. 初始化模拟器池
/// 6. 启动套利策略
/// 7. 启动心跳监控
/// 8. 运行事件处理引擎
pub async fn run(args: Args) -> Result<()> {
    // 设置 panic 处理钩子，确保程序崩溃时能够正确记录日志
    utils::set_panic_hook();
    
    // 初始化日志系统，启用指定模块的日志输出
    mev_logger::init_with_whitelisted_modules(
        "mainnet",
        "sui-arb".to_string(),
        &["arb", "utils", "shio", "cache_metrics=debug"],
    );

    // 解析私钥并生成对应的公钥和地址
    let keypair = SuiKeyPair::decode(&args.private_key)?;
    let pubkey = keypair.public();
    let attacker = SuiAddress::from(&pubkey);

    // 记录启动参数，便于调试和监控
    info!(
        "start_bot with attacker: {}, http_config: {:#?}, collector_config: {:#?}, db_sim_config: {:#?}, worker_config: {:#?}",
        attacker, args.http_config, args.collector_config, args.db_sim_config, args.worker_config
    );

    // 提取配置参数
    let rpc_url = args.http_config.rpc_url;
    let db_path = args.db_sim_config.db_path;
    let tx_socket_path = args.collector_config.tx_socket_path;
    let config_path = args.db_sim_config.config_path;
    let update_cache_socket = args.db_sim_config.update_cache_socket;
    let preload_path = args.db_sim_config.preload_path;
    
    // 创建事件处理引擎
    let mut engine = Engine::default();

    // 配置事件收集器：Shio 或公共交易
    if let Some(ref ws_url) = args.collector_config.shio_ws_url {
        // 如果配置了 Shio WebSocket URL，则使用 Shio 收集器
        let (shio_collector, shio_executor) =
            new_shio_collector_and_executor(keypair, Some(ws_url.clone()), None).await;
        engine.add_collector(map_collector!(shio_collector, Event::Shio));

        // 根据配置选择 Shio 执行器类型
        if args.shio_use_rpc {
            // 使用 RPC 方式提交 Shio 竞价
            let shio_rpc_executor = ShioRPCExecutor::new(SuiKeyPair::decode(&args.private_key)?);
            engine.add_executor(map_executor!(shio_rpc_executor, Action::ShioSubmitBid));
        } else {
            // 使用 WebSocket 方式提交 Shio 竞价
            engine.add_executor(map_executor!(shio_executor, Action::ShioSubmitBid));
        }
    } else {
        // 如果没有配置 Shio，则使用公共交易收集器
        let public_tx_collector = PublicTxCollector::new(&tx_socket_path);
        engine.add_collector(Box::new(public_tx_collector));
    }

    // 添加公共交易执行器
    // 负责将套利交易提交到 Sui 网络
    engine.add_executor(map_executor!(
        PublicTxExecutor::new(&rpc_url, SuiKeyPair::decode(&args.private_key)?).await?,
        Action::ExecutePublicTx
    ));

    // 配置私有交易收集器（可选）
    if let Some(ref relay_ws_url) = args.collector_config.relay_ws_url {
        // 如果配置了中继 WebSocket URL，则添加私有交易收集器
        // 用于监听来自中继服务的私有交易
        let private_tx_collector = PrivateTxCollector::new(relay_ws_url);
        engine.add_collector(Box::new(private_tx_collector));
    }

    // 创建模拟器池
    // 模拟器池提供并发的交易模拟能力，是套利算法的核心依赖
    let simulator_pool: ObjectPool<Box<dyn Simulator>> = match args.db_sim_config.use_db_simulator {
        true => {
            // 使用数据库模拟器（推荐）
            // 直接读取 Sui 节点数据库，提供最高性能的模拟能力
            let db_path = db_path.to_string();
            let config_path = config_path.to_string();
            let update_cache_socket = update_cache_socket.to_string();
            let preload_path = preload_path.to_string();
            ObjectPool::new(args.worker_config.num_simulators, move || {
                tokio::runtime::Runtime::new().unwrap().block_on(async {
                    let start = Instant::now();
                    let simulator = Box::new(
                        DBSimulator::new_slow(&db_path, &config_path, Some(&update_cache_socket), Some(&preload_path))
                            .await,
                    ) as Box<dyn Simulator>;
                    info!(elapsed = ?start.elapsed(), "DBSimulator initialized");
                    simulator
                })
            })
        }
        false => {
            // 使用 HTTP 模拟器（已弃用，仅用于测试）
            // 通过 RPC 调用进行模拟，性能较低
            warn!("http simulator is deprecated. use only for testing");

            let rpc_url = rpc_url.to_string();
            let ipc_path = args.http_config.ipc_path.clone();

            ObjectPool::new(args.worker_config.num_simulators, move || {
                let rpc_url = rpc_url.clone();
                let ipc_path = ipc_path.clone();

                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(async { Box::new(HttpSimulator::new(&rpc_url, &ipc_path).await) as Box<dyn Simulator> })
            })
        }
    };

    // 创建专用模拟器
    // 用于套利策略的专用模拟，与模拟器池分离以避免资源竞争
    // TODO: 当有中继（大量未执行交易）时，可能需要使用模拟器池
    let own_simulator = if args.db_sim_config.use_db_simulator {
        Arc::new(DBSimulator::new_slow(&db_path, &config_path, Some(&update_cache_socket), Some(&preload_path)).await)
            as Arc<dyn Simulator>
    } else {
        warn!("http simulator is deprecated. use only for testing");
        let ipc_path = args.http_config.ipc_path;
        Arc::new(HttpSimulator::new(&rpc_url, &ipc_path).await) as Arc<dyn Simulator>
    };

    // 创建重放模拟器（仅在使用数据库模拟器时）
    // 重放模拟器用于高精度的历史交易重放和分析
    let dedicated_simulator = if args.db_sim_config.use_db_simulator {
        Some(Arc::new(
            ReplaySimulator::new_slow(
                &db_path,
                &config_path,
                Duration::from_millis(args.worker_config.dedicated_long_interval),
                Duration::from_millis(args.worker_config.dedicated_short_interval),
            )
            .await,
        ))
    } else {
        None
    };

    // 记录模拟器池初始化完成
    info!("simulator_pool initialized: {:?}", simulator_pool);

    // 创建并添加套利策略
    // 套利策略是整个系统的核心，负责发现和执行套利机会
    let arb_strategy = ArbStrategy::new(
        attacker,                                    // 套利者地址
        Arc::new(simulator_pool),                   // 模拟器池
        own_simulator,                              // 专用模拟器
        args.worker_config.max_recent_arbs,         // 最近套利记录数量
        &rpc_url,                                   // RPC URL
        args.worker_config.workers,                 // 工作线程数
        dedicated_simulator,                        // 重放模拟器
    )
    .await;
    engine.add_strategy(Box::new(arb_strategy));

    // 添加 Telegram 消息执行器
    // 用于发送套利结果和系统状态通知
    engine.add_executor(map_executor!(
        TelegramMessageDispatcher::new_without_error_report(),
        Action::NotifyViaTelegram
    ));

    // 启动心跳监控
    // 每 30 秒发送一次心跳信号，用于监控系统健康状态
    heartbeat::start("sui-arb", Duration::from_secs(30));

    // 启动事件处理引擎并等待完成
    // 这里会阻塞直到程序退出
    engine.run_and_join().await.unwrap();

    Ok(())
}
