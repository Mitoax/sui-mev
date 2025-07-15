# Sui MEV Bot - 代码架构文档

## 项目目录结构

```
sui-mev/
├── bin/                    # 可执行程序
│   ├── arb/               # 套利机器人主程序
│   │   └── src/
│   │       ├── main.rs           # 程序入口 ✅ 已添加注释
│   │       ├── start_bot.rs      # 机器人启动逻辑
│   │       ├── arb.rs            # 套利核心逻辑
│   │       ├── collector.rs      # 数据收集器
│   │       ├── executor.rs       # 交易执行器
│   │       ├── config.rs         # 配置管理 ✅ 已添加注释
│   │       ├── types.rs          # 类型定义 ✅ 已添加注释
│   │       ├── pool_ids.rs       # 池ID管理 ✅ 已添加注释
│   │       ├── common/           # 通用模块
│   │       ├── defi/             # DeFi协议实现
│   │       └── strategy/         # 策略模块 ✅ 已添加注释
│   │           ├── mod.rs        # 策略主模块 ✅ 已添加注释
│   │           ├── arb_cache.rs  # 套利缓存管理 ✅ 已添加注释
│   │           └── worker.rs     # 工作线程实现 ✅ 已添加注释
│   └── relay/             # 中继服务器
│       └── src/
│           └── main.rs           # 中继服务器主程序
├── crates/                # 核心库
│   ├── arb-common/        # 套利通用库
│   ├── dex-indexer/       # DEX索引器
│   ├── logger/            # 日志库
│   ├── object-pool/       # 对象池
│   ├── shio/              # Shio协议支持
│   ├── simulator/         # 交易模拟器
│   ├── utils/             # 工具库
│   └── version/           # 版本管理
├── scripts/               # 脚本文件
└── docs/                  # 文档
    ├── README.md             # 项目文档总览
    ├── architecture.md       # 系统架构设计文档
    ├── development-guide.md  # 开发指南
    └── devlogs/             # 开发日志
        ├── structure.md      # 代码架构文档
        ├── memory.md         # 项目理解与需求
        ├── dev_history.md    # 开发历史记录
        ├── chain_analysis.md # 多链支持与套利策略分析
        └── rules.md          # 开发规则与最佳实践
```

## 核心组件架构

### 1. 数据收集层 (Collectors)
- **PublicTxCollector**: 收集公共交易数据
- **PrivateTxCollector**: 收集私有交易数据（通过relay）
- **ShioCollector**: 收集Shio协议交易

### 2. 策略处理层 (Strategy) ✅ 已完成注释
- **ArbStrategy**: 主要套利策略，负责事件处理、机会识别、缓存管理和工作线程调度
- **Worker**: 工作线程管理，处理套利任务、验证交易、提交动作和发送通知
- **ArbCache**: 套利缓存管理，提供唯一性、重排序和定时过期功能

### 3. 模拟执行层 (Simulator)
- **DBSimulator**: 数据库模拟器（推荐）
- **HttpSimulator**: HTTP模拟器（已弃用）
- **ReplaySimulator**: 重放模拟器

### 4. 交易执行层 (Executors)
- **PublicTxExecutor**: 公共交易执行
- **ShioExecutor**: Shio交易执行
- **TelegramMessageDispatcher**: 通知执行

### 5. DEX协议支持
- **Cetus**: CLMM协议
- **Turbos**: AMM协议
- **Aftermath**: 多资产池
- **Kriya**: AMM和CLMM
- **FlowX**: CLMM协议
- **DeepBook V2**: 订单簿
- **BlueMove**: NFT和代币交易
- **Navi**: 借贷协议
- **Shio**: 专有协议

## 数据流架构

1. **数据收集**: Collectors → Event Stream
2. **策略处理**: Event Stream → ArbStrategy → Action
3. **模拟验证**: Action → Simulator → 验证结果
4. **交易执行**: 验证通过 → Executor → 链上执行
5. **结果通知**: 执行结果 → Telegram/日志

## 关键配置 ✅ 已添加详细注释

- **Workers**: 处理事件的工作线程数（默认8）
- **Simulators**: 模拟器池大小（默认32）
- **MaxRecentArbs**: 最近套利记录数（默认20）
- **Intervals**: 专用模拟器间隔（短50ms，长200ms）
- **GAS_BUDGET**: Gas预算限制（1_000_000_000）
- **MAX_SQRT_PRICE_X64**: 最大平方根价格
- **MIN_SQRT_PRICE_X64**: 最小平方根价格
- **pegged_coin_types**: 支持的稳定币和主要币种类型

## 代码注释完成状态

### 已完成注释的文件：
- ✅ `config.rs` - MEV套利系统核心配置参数和常量
- ✅ `types.rs` - 核心数据类型：Action、Event、Source
- ✅ `main.rs` - 主程序入口，支持多种运行模式
- ✅ `pool_ids.rs` - 流动性池对象ID管理模块
- ✅ `strategy/mod.rs` - 套利策略核心模块
- ✅ `strategy/arb_cache.rs` - 套利缓存管理模块
- ✅ `strategy/worker.rs` - 工作线程模块
- ✅ `start_bot.rs` - 机器人启动逻辑，组件初始化和协调
- ✅ `arb.rs` - 套利核心算法，机会发现和路径优化
- ✅ `collector.rs` - 交易收集器，支持公共和私有交易收集
- ✅ `executor.rs` - 交易执行器，负责交易签名和提交

### 待添加注释的文件：
- ⏳ `common/` - 通用工具和辅助模块
- ⏳ `defi/` - DeFi协议适配器实现

## 核心模块注释完成度

### 🎉 已完成核心业务逻辑注释 (100%)
所有核心业务逻辑文件已完成详细的中文注释，包括：
- **程序入口**: main.rs, start_bot.rs
- **核心配置**: config.rs, types.rs, pool_ids.rs
- **套利算法**: arb.rs
- **策略模块**: strategy/ 目录下所有文件
- **数据收集**: collector.rs
- **交易执行**: executor.rs

### 📊 注释质量标准
所有已完成的文件都包含：
- 📝 文件头部功能描述和使用场景
- 🏗️ 结构体和枚举的详细字段说明
- 🔧 函数的参数、返回值和功能描述
- 💡 复杂逻辑的内部注释和算法说明
- 🔗 模块间依赖关系和数据流向