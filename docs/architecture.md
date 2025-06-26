# Sui MEV Bot - 架构设计文档

## 概述

Sui MEV Bot 是一个高性能的套利机器人系统，专门设计用于在 Sui 区块链上捕获 MEV（Maximal Extractable Value）机会。系统采用模块化架构，支持多种 DEX 协议和数据源。

## 系统架构图

```mermaid
graph TB
    subgraph "数据源层"
        A[Sui 验证器] --> B[Relay 服务器]
        C[公共交易池] --> D[Unix Socket]
        E[Shio 协议] --> F[WebSocket]
    end
    
    subgraph "数据收集层"
        B --> G[PrivateTxCollector]
        D --> H[PublicTxCollector]
        F --> I[ShioCollector]
    end
    
    subgraph "事件处理引擎"
        G --> J[Event Stream]
        H --> J
        I --> J
        J --> K[ArbStrategy]
    end
    
    subgraph "模拟执行层"
        K --> L[DBSimulator Pool]
        K --> M[ReplaySimulator]
        L --> N[交易模拟验证]
        M --> N
    end
    
    subgraph "执行层"
        N --> O[PublicTxExecutor]
        N --> P[ShioExecutor]
        O --> Q[Sui 区块链]
        P --> Q
    end
    
    subgraph "监控通知层"
        Q --> R[TelegramDispatcher]
        Q --> S[日志系统]
        R --> T[Telegram 通知]
        S --> U[系统日志]
    end
```

## 程序流程图

```mermaid
flowchart TD
    A[启动 MEV Bot] --> B[初始化配置]
    B --> C[创建数据收集器]
    C --> D[创建模拟器池]
    D --> E[创建策略处理器]
    E --> F[创建执行器]
    F --> G[启动事件引擎]
    
    G --> H[监听交易事件]
    H --> I{事件类型}
    
    I -->|公共交易| J[PublicTx 处理]
    I -->|私有交易| K[PrivateTx 处理]
    I -->|Shio交易| L[Shio 处理]
    
    J --> M[套利机会识别]
    K --> M
    L --> M
    
    M --> N{发现套利机会?}
    N -->|否| H
    N -->|是| O[获取模拟器]
    
    O --> P[模拟交易执行]
    P --> Q{模拟成功?}
    Q -->|否| R[释放模拟器]
    Q -->|是| S[计算预期利润]
    
    S --> T{利润 > 阈值?}
    T -->|否| R
    T -->|是| U[构建套利交易]
    
    U --> V[签名交易]
    V --> W[提交到区块链]
    W --> X[等待确认]
    X --> Y{交易成功?}
    
    Y -->|是| Z[记录成功套利]
    Y -->|否| AA[记录失败原因]
    
    Z --> BB[发送通知]
    AA --> BB
    BB --> R
    R --> H
```

## 组件交互图

```mermaid
sequenceDiagram
    participant V as Validator
    participant R as Relay
    participant C as Collector
    participant S as Strategy
    participant Sim as Simulator
    participant E as Executor
    participant B as Blockchain
    participant T as Telegram
    
    V->>R: 发送内存池交易
    R->>C: 转发交易数据
    C->>S: 生成交易事件
    
    S->>S: 分析套利机会
    alt 发现套利机会
        S->>Sim: 请求模拟执行
        Sim->>Sim: 模拟交易
        Sim->>S: 返回模拟结果
        
        alt 模拟成功且有利润
            S->>E: 提交套利交易
            E->>B: 执行交易
            B->>E: 返回执行结果
            E->>T: 发送通知
            E->>S: 返回执行状态
        end
    end
    
    S->>C: 继续监听
```

## DEX 协议支持架构

```mermaid
graph LR
    subgraph "DEX 协议层"
        A[Cetus CLMM]
        B[Turbos AMM]
        C[Aftermath]
        D[Kriya AMM/CLMM]
        E[FlowX CLMM]
        F[DeepBook V2]
        G[BlueMove]
        H[Navi]
        I[Shio]
    end
    
    subgraph "协议适配器"
        J[Cetus Adapter]
        K[Turbos Adapter]
        L[Aftermath Adapter]
        M[Kriya Adapter]
        N[FlowX Adapter]
        O[DeepBook Adapter]
        P[BlueMove Adapter]
        Q[Navi Adapter]
        R[Shio Adapter]
    end
    
    subgraph "统一接口"
        S[Trade Interface]
        T[Pool Interface]
        U[Price Interface]
    end
    
    A --> J
    B --> K
    C --> L
    D --> M
    E --> N
    F --> O
    G --> P
    H --> Q
    I --> R
    
    J --> S
    K --> S
    L --> S
    M --> S
    N --> S
    O --> S
    P --> S
    Q --> S
    R --> S
    
    J --> T
    K --> T
    L --> T
    M --> T
    N --> T
    O --> T
    P --> T
    Q --> T
    R --> T
    
    J --> U
    K --> U
    L --> U
    M --> U
    N --> U
    O --> U
    P --> U
    Q --> U
    R --> U
```

## 依赖关系图

```mermaid
graph TD
    subgraph "应用层"
        A[arb 主程序]
        B[relay 中继]
    end
    
    subgraph "业务逻辑层"
        C[dex-indexer]
        D[simulator]
        E[shio]
        F[object-pool]
    end
    
    subgraph "基础设施层"
        G[utils]
        H[logger]
        I[version]
        J[arb-common]
    end
    
    subgraph "外部依赖"
        K[sui-sdk]
        L[sui-types]
        M[fastcrypto]
        N[tokio]
        O[serde]
        P[eyre]
    end
    
    A --> C
    A --> D
    A --> E
    A --> F
    A --> G
    A --> H
    A --> J
    
    B --> G
    B --> H
    
    C --> G
    C --> H
    D --> G
    E --> G
    E --> H
    F --> G
    
    A --> K
    A --> L
    A --> M
    A --> N
    A --> O
    A --> P
    
    B --> K
    B --> L
    B --> M
    B --> N
    B --> O
    B --> P
```

## 数据流图

```mermaid
flowchart LR
    subgraph "输入数据"
        A[内存池交易]
        B[链上事件]
        C[价格数据]
    end
    
    subgraph "数据处理"
        D[事件解析]
        E[套利识别]
        F[风险评估]
    end
    
    subgraph "决策执行"
        G[交易构建]
        H[模拟验证]
        I[链上执行]
    end
    
    subgraph "输出结果"
        J[执行结果]
        K[利润统计]
        L[监控告警]
    end
    
    A --> D
    B --> D
    C --> D
    
    D --> E
    E --> F
    F --> G
    
    G --> H
    H --> I
    
    I --> J
    J --> K
    J --> L
```

## 部署架构图

```mermaid
graph TB
    subgraph "Sui 网络"
        A[Sui 验证器节点]
        B[Sui 全节点]
    end
    
    subgraph "MEV Bot 服务器"
        C[Relay 服务]
        D[Arb Bot 主程序]
        E[数据库]
        F[日志文件]
    end
    
    subgraph "监控系统"
        G[Telegram Bot]
        H[监控面板]
        I[告警系统]
    end
    
    A -->|内存池交易| C
    B -->|RPC 调用| D
    C -->|WebSocket| D
    D -->|交易提交| B
    D -->|状态存储| E
    D -->|日志记录| F
    D -->|通知| G
    D -->|指标| H
    H -->|告警| I
```

## 性能优化架构

```mermaid
graph TD
    subgraph "并发处理"
        A[多线程 Workers]
        B[异步 I/O]
        C[事件驱动]
    end
    
    subgraph "缓存优化"
        D[对象池]
        E[内存缓存]
        F[预加载数据]
    end
    
    subgraph "网络优化"
        G[连接复用]
        H[批量处理]
        I[压缩传输]
    end
    
    subgraph "算法优化"
        J[快速路径查找]
        K[并行模拟]
        L[智能过滤]
    end
    
    A --> D
    B --> E
    C --> F
    
    D --> G
    E --> H
    F --> I
    
    G --> J
    H --> K
    I --> L
```

## 总结

本架构文档详细描述了 Sui MEV Bot 的各个组件和它们之间的关系。系统采用分层架构设计，具有良好的模块化、可扩展性和高性能特征。通过这些图表，开发者可以快速理解系统的整体架构和工作原理，便于后续的开发和维护工作。