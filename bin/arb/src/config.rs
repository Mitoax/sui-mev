//! 配置模块
//!
//! 本模块定义了 MEV 套利系统的核心配置参数和常量：
//! - 交易费用配置：Gas 预算限制
//! - 价格边界配置：最大/最小平方根价格（用于 AMM 计算）
//! - 稳定币类型：系统支持的锚定币种列表
//! - 测试配置：测试环境专用参数
//!
//! 这些配置直接影响套利策略的执行效果和风险控制。

use std::collections::HashSet;

use sui_sdk::SUI_COIN_TYPE;

/// Gas 预算限制（单位：MIST）
/// 
/// 设置为 10 SUI，这是单笔交易的最大 Gas 消耗限制。
/// 套利交易通常涉及多个 DEX 调用，需要较高的 Gas 预算。
pub const GAS_BUDGET: u64 = 10_000_000_000;

/// 最大平方根价格（X64 格式）
/// 
/// 用于 AMM（自动做市商）价格计算的上限。
/// X64 格式表示价格乘以 2^64，提供高精度的定点数运算。
/// 这个值对应于极高的价格比率，用于防止价格溢出。
pub const MAX_SQRT_PRICE_X64: u128 = 79226673515401279992447579055;

/// 最小平方根价格（X64 格式）
/// 
/// 用于 AMM 价格计算的下限。
/// 防止价格接近零时的数值计算问题。
pub const MIN_SQRT_PRICE_X64: u128 = 4295048016;

/// 获取锚定币种类型集合
/// 
/// 返回系统支持的稳定币和主要币种列表。这些币种通常具有：
/// - 相对稳定的价值（如稳定币）
/// - 高流动性和广泛接受度
/// - 适合作为套利路径的中间币种
/// 
/// # 返回
/// * `HashSet<&'static str>` - 币种类型标识符集合
/// 
/// # 包含的币种
/// - SUI: 原生代币
/// - USDC: 多个版本的美元稳定币
/// - USDT: 泰达币
/// - WETH: 包装以太坊
/// - BUCK: Bucket 协议的稳定币
pub fn pegged_coin_types() -> HashSet<&'static str> {
    HashSet::from_iter([
        SUI_COIN_TYPE,  // SUI 原生代币
        
        // USDC (多个合约版本)
        "0x5d4b302506645c37ff133b98c4b50a5ae14841659738d6d733d59d0d217a93bf::coin::COIN",
        "0xb231fcda8bbddb31f2ef02e6161444aec64a514e2c89279584ac9806ce9cf037::coin::COIN",
        "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC",
        
        // USDT (泰达币)
        "0xc060006111016b8a020ad5b33834984a437aaa7d3c74c18e09a95d48aceab08c::coin::COIN",
        
        // WETH (包装以太坊)
        "0xaf8cd5edc19c4512f4259f0bee101a40d41ebed738ade5874359610ef8eeced5::coin::COIN",
        
        // Bucket USD (Bucket 协议稳定币)
        "0xce7ff77a83ea0cb6fd39bd8748e2ec89a3f41e8efdc3f4eb123e0ca37b184db2::buck::BUCK",
    ])
}

/// 测试配置模块
/// 
/// 包含测试环境专用的配置参数。
/// 在实际部署时，这些值应该被具体的测试环境参数替换。
#[cfg(test)]
pub mod tests {
    /// 测试环境的 HTTP RPC URL
    /// 
    /// 用于连接测试网络或本地节点的 RPC 端点。
    /// 在运行测试前需要设置为有效的 URL。
    pub const TEST_HTTP_URL: &str = "";
    
    /// 测试用的攻击者地址
    /// 
    /// 用于测试套利交易的发送方地址。
    /// 在运行测试前需要设置为有效的 Sui 地址。
    pub const TEST_ATTACKER: &str = "";
}
