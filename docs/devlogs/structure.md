# Sui MEV Bot - 代码架构文档

## 项目目录结构

```
sui-mev/
├── bin/                    # 可执行程序
│   ├── arb/               # 套利机器人主程序
│   │   └── src/
│   │       ├── main.rs           # 程序入口
│   │       ├── start_bot.rs      # 机器人启动逻辑
│   │       ├── arb.rs            # 套利核心逻辑
│   │       ├── collector.rs      # 数据收集器
│   │       ├── executor.rs       # 交易执行器
│   │       ├── config.rs         # 配置管理
│   │       ├── types.rs          # 类型定义
│   │       ├── pool_ids.rs       # 池ID管理
│   │       ├── common/           # 通用模块
│   │       ├── defi/             # DeFi协议实现
│   │       └── strategy/         # 策略模块
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
    └── devlogs/          # 开发日志
```

## 核心组件架构

### 1. 数据收集层 (Collectors)
- **PublicTxCollector**: 收集公共交易数据
- **PrivateTxCollector**: 收集私有交易数据（通过relay）
- **ShioCollector**: 收集Shio协议交易

### 2. 策略处理层 (Strategy)
- **ArbStrategy**: 主要套利策略
- **Worker**: 工作线程管理
- **ArbCache**: 套利缓存管理

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

## 关键配置

- **Workers**: 处理事件的工作线程数（默认8）
- **Simulators**: 模拟器池大小（默认32）
- **MaxRecentArbs**: 最近套利记录数（默认20）
- **Intervals**: 专用模拟器间隔（短50ms，长200ms）