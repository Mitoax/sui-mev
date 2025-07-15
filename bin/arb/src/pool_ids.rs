//! # 流动性池对象 ID 管理模块
//!
//! 负责扫描、收集和管理所有 DEX 协议中流动性池的对象 ID。
//! 主要功能：
//! - 扫描支持的 DEX 协议（Cetus、Turbos、DeepBook 等）
//! - 收集池相关的所有对象 ID（池对象、代币对象、配置对象等）
//! - 生成完整的对象 ID 列表文件，用于模拟器预加载
//! - 提供测试功能验证交易路径和模拟器配置

use std::collections::HashSet;
use std::fs;
use std::str::FromStr;
use std::sync::Arc;

use clap::Parser;
use dex_indexer::{types::Protocol, DexIndexer};
use eyre::Result;
use mev_logger::LevelFilter;
use object_pool::ObjectPool;
use simulator::{DBSimulator, SimulateCtx, Simulator};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use sui_sdk::types::{
    BRIDGE_PACKAGE_ID, DEEPBOOK_PACKAGE_ID, MOVE_STDLIB_PACKAGE_ID, SUI_AUTHENTICATOR_STATE_OBJECT_ID,
    SUI_BRIDGE_OBJECT_ID, SUI_CLOCK_OBJECT_ID, SUI_DENY_LIST_OBJECT_ID, SUI_FRAMEWORK_PACKAGE_ID,
    SUI_RANDOMNESS_STATE_OBJECT_ID, SUI_SYSTEM_PACKAGE_ID, SUI_SYSTEM_STATE_OBJECT_ID,
};
use sui_sdk::SuiClientBuilder;
use sui_types::base_types::{ObjectID, SuiAddress};
use sui_types::object::{Object, Owner};
use sui_types::transaction::{InputObjectKind, ObjectReadResult};
use tracing::info;

use crate::common::get_latest_epoch;
use crate::defi::{DexSearcher, IndexerDexSearcher, TradeType, Trader};
use crate::HttpConfig;

/// 池对象 ID 生成器命令行参数
/// 
/// 支持两种运行模式：
/// 1. 生成模式：扫描所有 DEX 协议并生成完整的对象 ID 列表
/// 2. 测试模式：验证特定交易路径的模拟执行
#[derive(Clone, Debug, Parser)]
pub struct Args {
    /// 输出文件路径
    /// 
    /// 生成的对象 ID 列表将写入此文件，每行一个对象 ID
    #[clap(long, default_value = "./pool_related_ids.txt")]
    pub result_path: String,

    /// HTTP 连接配置
    #[command(flatten)]
    pub http_config: HttpConfig,

    /// 仅运行测试模式
    /// 
    /// 启用后将执行交易路径测试而不是生成对象 ID 列表
    #[clap(long, help = "Run test only")]
    pub test: bool,

    /// 使用回退模拟器
    /// 
    /// 在测试模式下，当主模拟器失败时是否使用 HTTP 回退
    #[clap(long, help = "Simulate with fallback")]
    pub with_fallback: bool,

    /// 测试交易输入金额
    /// 
    /// 用于测试模式的交易金额（单位：最小单位）
    #[clap(long, default_value = "10000000")]
    pub amount_in: u64,

    /// 测试交易路径
    /// 
    /// 逗号分隔的对象 ID 列表，定义测试交易的路径
    #[clap(
        long,
        default_value = "0x3c3dd05e348fba5d8bf6958369cc3b33c8e8be85c96e10b1ca6413ad1b2d7787,0xe356c686eb19972e076b6906de12354a1a7ce1b09691416e9d852b04fd21b9a6,0xade90c3bc407eaa34068129d63bba5d1cf7889a2dbaabe5eb9b3efbbf53891ea,0xda49f921560e39f15d801493becf79d47c89fb6db81e0cbbe7bf6d3318117a00"
    )]
    pub path: String,

    /// 测试前删除的对象
    /// 
    /// 逗号分隔的对象 ID 列表，这些对象将在模拟前被删除
    #[clap(long, help = "Delete objects before simulation")]
    pub delete_objects: Option<String>,
}

/// 获取支持的 DEX 协议列表
/// 
/// 返回系统支持的所有 DEX 协议，用于扫描流动性池。
/// 包含主流的 AMM 和 CLMM 协议。
/// 
/// # 返回
/// * `Vec<Protocol>` - 支持的协议列表
fn supported_protocols() -> Vec<Protocol> {
    vec![
        Protocol::Cetus,      // Cetus CLMM 协议
        Protocol::Turbos,     // Turbos CLMM 协议
        Protocol::KriyaAmm,   // Kriya AMM 协议
        Protocol::BlueMove,   // BlueMove AMM 协议
        Protocol::KriyaClmm,  // Kriya CLMM 协议
        Protocol::FlowxClmm,  // FlowX CLMM 协议
        Protocol::Navi,       // Navi 借贷协议
        Protocol::Aftermath,  // Aftermath AMM 协议
    ]
}

/// 主执行函数：生成池相关对象 ID 列表
/// 
/// 根据参数选择运行模式：
/// - 生成模式：扫描所有支持的 DEX 协议，收集池和相关对象的 ID
/// - 测试模式：验证特定交易路径的模拟执行
/// 
/// # 参数
/// * `args` - 命令行参数
/// 
/// # 返回
/// * `Result<()>` - 执行结果
pub async fn run(args: Args) -> Result<()> {
    // 初始化日志系统，设置适当的日志级别
    mev_logger::init_console_logger_with_directives(
        Some(LevelFilter::INFO),
        &[
            "arb=debug",              // 套利模块调试信息
            // "dex_indexer=warn",     // DEX 索引器警告
            // "simulator=trace",      // 模拟器详细跟踪
            // "sui_types=trace",      // Sui 类型跟踪
            // "sui_move_natives_latest=trace", // Move 原生函数跟踪
            // "sui_execution=warn",   // 执行引擎警告
        ],
    );
    
    // 如果是测试模式，执行测试函数
    if args.test {
        return test_pool_related_objects(args).await;
    }

    let result_path = args.result_path;
    let rpc_url = args.http_config.rpc_url;

    // 初始化 DEX 索引器，用于获取池信息
    let dex_indexer = DexIndexer::new(&rpc_url).await?;
    // 初始化数据库模拟器，用于获取对象详细信息
    let simulator: Arc<dyn Simulator> = Arc::new(DBSimulator::new_default_slow().await);

    // 清理旧的结果文件并创建新文件
    let _ = fs::remove_file(&result_path);
    let file = File::create(&result_path)?;
    let mut writer = BufWriter::new(file);

    // 加载已存在的对象 ID（如果文件存在）
    let mut object_ids: HashSet<String> = fs::read_to_string(&result_path)
        .unwrap_or_default()
        .lines()
        .map(|line| line.to_string())
        .collect();

    // 为每个支持的协议添加相关对象 ID
    for protocol in supported_protocols() {
        // 添加协议级别的相关对象 ID（如包 ID、配置对象等）
        object_ids.extend(protocol.related_object_ids().await?);
        
        if protocol == Protocol::Navi {
            // Navi 协议的池不在索引器中，跳过池扫描
            continue;
        }

        // 添加每个池的相关对象 ID
        for pool in dex_indexer.get_all_pools(&protocol)? {
            // 获取池相关的所有对象（代币、配置、状态等）
            object_ids.extend(pool.related_object_ids(simulator.clone()).await);
        }
    }

    // 添加全局系统对象 ID
    object_ids.extend(global_ids());

    // 将所有对象 ID 写入文件
    let all_ids: Vec<String> = object_ids.into_iter().collect();
    writeln!(writer, "{}", all_ids.join("\n"))?;

    // 确保数据写入磁盘
    writer.flush()?;

    info!("🎉 write pool and related object ids to {}", result_path);

    Ok(())
}

/// 获取全局系统对象 ID 集合
/// 
/// 返回 Sui 系统中的核心对象 ID，这些对象在所有交易中都可能被使用。
/// 包括系统包、全局状态对象和跨链桥相关对象。
/// 
/// # 返回
/// * `HashSet<String>` - 全局对象 ID 集合
fn global_ids() -> HashSet<String> {
    // Sui 系统核心对象 ID
    let mut result = vec![
        MOVE_STDLIB_PACKAGE_ID,           // Move 标准库包
        SUI_FRAMEWORK_PACKAGE_ID,         // Sui 框架包
        SUI_SYSTEM_PACKAGE_ID,            // Sui 系统包
        BRIDGE_PACKAGE_ID,                // 跨链桥包
        DEEPBOOK_PACKAGE_ID,              // DeepBook 包
        SUI_SYSTEM_STATE_OBJECT_ID,       // 系统状态对象
        SUI_CLOCK_OBJECT_ID,              // 时钟对象
        SUI_AUTHENTICATOR_STATE_OBJECT_ID, // 认证器状态对象
        SUI_RANDOMNESS_STATE_OBJECT_ID,   // 随机数状态对象
        SUI_BRIDGE_OBJECT_ID,             // 跨链桥对象
        SUI_DENY_LIST_OBJECT_ID,          // 拒绝列表对象
    ]
    .into_iter()
    .map(|id| id.to_string())
    .collect::<HashSet<String>>();

    // 添加其他重要的全局对象 ID
    result.insert("0x5306f64e312b581766351c07af79c72fcb1cd25147157fdc2f8ad76de9a3fb6a".to_string()); // Wormhole 主对象
    result.insert("0x26efee2b51c911237888e5dc6702868abca3c7ac12c53f76ef8eba0697695e3d".to_string()); // Wormhole 辅助对象

    result
}

/// 测试池相关对象的模拟交易
/// 
/// 使用指定的交易路径和参数执行模拟交易，验证对象 ID 列表的完整性。
/// 主要用于调试和验证模拟器配置。
/// 
/// # 参数
/// * `args` - 测试参数
/// 
/// # 返回
/// * `Result<()>` - 测试结果
async fn test_pool_related_objects(args: Args) -> Result<()> {
    // ==================== 测试数据准备 ====================
    // 固定的测试发送者地址
    let sender = SuiAddress::from_str("0xac5bceec1b789ff840d7d4e6ce4ce61c90d190a7f8c4f4ddf0bff6ee2413c33c").unwrap();
    let amount_in = args.amount_in;

    // 解析交易路径（逗号分隔的对象 ID）
    let path = args
        .path
        .split(',')
        .map(|obj_id| ObjectID::from_hex_literal(obj_id).unwrap())
        .collect::<Vec<_>>();

    let with_fallback = args.with_fallback;
    let rpc_url = args.http_config.rpc_url;

    // 创建模拟器对象池（单个模拟器实例）
    let simulator_pool = Arc::new(ObjectPool::new(1, move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { Box::new(DBSimulator::new_test(with_fallback).await) as Box<dyn Simulator> })
    }));

    // 初始化 DEX 搜索器和交易路径
    let dex_searcher: Arc<dyn DexSearcher> = Arc::new(IndexerDexSearcher::new(&rpc_url, simulator_pool.clone()).await?);
    let path = dex_searcher.find_test_path(&path).await?;
    info!(?with_fallback, ?amount_in, ?path, ?args.delete_objects, "test data");
    // ==================== 测试数据准备完成 ====================

    // 连接 Sui 客户端并获取当前 epoch
    let sui = SuiClientBuilder::default().build(&rpc_url).await?;
    let epoch = get_latest_epoch(&sui).await?;

    // 加载池相关对象用于模拟
    let mut override_objects = pool_related_objects(&args.result_path).await?;
    
    // 如果指定了要删除的对象，从模拟上下文中移除它们
    if let Some(delete_objects) = args.delete_objects {
        let delete_objects = delete_objects
            .split(',')
            .map(|obj_id| ObjectID::from_hex_literal(obj_id).unwrap())
            .collect::<Vec<_>>();
        override_objects.retain(|obj| !delete_objects.contains(&obj.id()));
    }

    // 创建模拟上下文
    let sim_ctx = SimulateCtx::new(epoch, override_objects);

    // 执行模拟交易
    let trader = Trader::new(simulator_pool).await?;
    let result = trader
        .get_trade_result(&path, sender, amount_in, TradeType::Flashloan, vec![], sim_ctx)
        .await?;
    info!(?result, "trade result");

    Ok(())
}

/// 从文件加载池相关对象
/// 
/// 读取对象 ID 列表文件，并从模拟器中获取对应的对象数据。
/// 根据对象的所有权类型（共享或拥有）创建适当的输入对象类型。
/// 
/// # 参数
/// * `file_path` - 对象 ID 列表文件路径
/// 
/// # 返回
/// * `Result<Vec<ObjectReadResult>>` - 对象读取结果列表
async fn pool_related_objects(file_path: &str) -> Result<Vec<ObjectReadResult>> {
    // 初始化测试模拟器（启用回退）
    let simulator: Arc<dyn Simulator> = Arc::new(DBSimulator::new_test(true).await);
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let mut res = vec![];
    
    // 逐行读取对象 ID 并获取对象数据
    for line in reader.lines() {
        let line = line?;
        let object_id = ObjectID::from_hex_literal(&line)?;
        
        // 从模拟器获取对象，如果不存在则跳过
        let object: Object = if let Some(obj) = simulator.get_object(&object_id).await {
            obj
        } else {
            continue;
        };

        // 根据对象所有权类型创建输入对象类型
        let input_object_kind = match object.owner() {
            Owner::Shared { initial_shared_version } => {
                // 共享对象：需要指定初始共享版本
                InputObjectKind::SharedMoveObject {
                    id: object_id,
                    initial_shared_version: *initial_shared_version,
                    mutable: true,
                }
            }
            _ => {
                // 不可变或拥有对象：使用对象引用
                InputObjectKind::ImmOrOwnedMoveObject(object.compute_object_reference())
            }
        };

        res.push(ObjectReadResult::new(input_object_kind, object.into()));
    }

    Ok(res)
}
