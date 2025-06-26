# Sui MEV Bot - 项目文档总览

## 📋 文档导航

本目录包含了 Sui MEV Bot 项目的完整技术文档，帮助开发者快速理解项目架构和开发流程。

### 🏗️ 架构设计
- **[架构设计文档](./architecture.md)** - 系统整体架构、组件关系和数据流
- **[开发指南](./development-guide.md)** - 开发流程、最佳实践和故障排查

### 📝 开发日志
- **[项目理解](./devlogs/memory.md)** - 项目需求、功能特性和用户优先级
- **[代码结构](./devlogs/structure.md)** - 目录结构、组件架构和配置说明
- **[开发历史](./devlogs/dev_history.md)** - 开发记录、问题解决和经验总结

## 🚀 快速开始

### 系统概述
Sui MEV Bot 是一个高性能的套利机器人系统，专门设计用于在 Sui 区块链上捕获 MEV（Maximal Extractable Value）机会。

### 核心特性
- ✅ **多协议支持**：支持 9+ 主流 DEX 协议
- ✅ **实时监控**：多数据源实时交易监控
- ✅ **智能模拟**：高性能交易模拟验证
- ✅ **自动执行**：智能套利策略自动执行
- ✅ **风险控制**：完善的风险管理机制
- ✅ **监控告警**：Telegram 实时通知

### 支持的 DEX 协议
```
🔹 Cetus (CLMM)     🔹 Turbos (AMM)     🔹 Aftermath
🔹 Kriya (AMM/CLMM) 🔹 FlowX (CLMM)     🔹 DeepBook V2
🔹 BlueMove          🔹 Navi             🔹 Shio
```

## 📊 系统架构概览

### 整体架构
```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   数据收集层     │    │   策略处理层     │    │   执行监控层     │
├─────────────────┤    ├─────────────────┤    ├─────────────────┤
│ • 公共交易收集   │───▶│ • 套利机会识别   │───▶│ • 交易执行       │
│ • 私有交易收集   │    │ • 风险评估       │    │ • 结果监控       │
│ • Shio 协议     │    │ • 策略优化       │    │ • 告警通知       │
└─────────────────┘    └─────────────────┘    └─────────────────┘
           │                       │                       │
           ▼                       ▼                       ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   模拟验证层     │    │   协议适配层     │    │   基础设施层     │
├─────────────────┤    ├─────────────────┤    ├─────────────────┤
│ • DB 模拟器     │    │ • DEX 协议适配   │    │ • 对象池管理     │
│ • HTTP 模拟器   │    │ • 价格计算       │    │ • 日志系统       │
│ • 重放模拟器     │    │ • 交易构建       │    │ • 工具库         │
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

### 核心组件

#### 🔄 数据流处理
```
数据源 → 收集器 → 事件流 → 策略引擎 → 模拟器 → 执行器 → 区块链
   ↓        ↓        ↓         ↓         ↓        ↓        ↓
验证器   解析器   过滤器   分析器   验证器   签名器   确认器
   ↓        ↓        ↓         ↓         ↓        ↓        ↓
中继器   缓存器   路由器   优化器   池管理   通知器   监控器
```

#### ⚡ 性能优化
- **并发处理**：多线程工作池（默认 8 个 workers）
- **对象池**：模拟器对象池（默认 32 个实例）
- **智能缓存**：套利机会缓存和去重
- **连接复用**：WebSocket 和 RPC 连接复用

## 🛠️ 开发环境

### 系统要求
```bash
# 操作系统
Ubuntu 22.04+

# 开发工具
Rust 1.70+
Node.js 22
Python 3.11

# 包管理器
uv (Python)
pnpm (Node.js)
cargo (Rust)
```

### 快速启动
```bash
# 1. 克隆项目
git clone <repository-url>
cd sui-mev

# 2. 构建项目
cargo build --release

# 3. 启动套利机器人
cargo run -r --bin arb start-bot -- --private-key <YOUR_PRIVATE_KEY>

# 4. 启动中继服务器（可选）
cargo run -r --bin relay
```

## 📈 监控和运维

### 关键指标
- **交易延迟**：从发现机会到执行完成的时间
- **成功率**：套利交易的成功执行率
- **利润统计**：实时和历史盈利数据
- **Gas 消耗**：交易成本分析
- **系统资源**：CPU、内存使用情况

### 告警机制
- 🔔 **Telegram 通知**：实时交易结果和系统状态
- 📊 **心跳监控**：30秒间隔的系统健康检查
- 🚨 **异常告警**：错误率超阈值自动告警
- 📝 **日志记录**：结构化日志便于问题排查

## 🔧 配置说明

### 主要配置参数
```bash
# 基础配置
SUI_RPC_URL=http://localhost:9000          # Sui RPC 节点地址
SUI_PRIVATE_KEY=<your_private_key>         # 交易签名私钥

# 数据源配置
SUI_TX_SOCKET_PATH=/tmp/sui_tx.sock       # 公共交易 Socket
relay-ws-url=ws://localhost:9001           # 私有交易 WebSocket
shio-ws-url=ws://shio.example.com          # Shio 协议 WebSocket

# 性能配置
workers=8                                  # 工作线程数
num-simulators=32                          # 模拟器池大小
max-recent-arbs=20                         # 最近套利记录数

# 数据库模拟器配置
SUI_DB_PATH=/home/ubuntu/sui/db/live/store # 数据库路径
SUI_CONFIG_PATH=/home/ubuntu/sui/fullnode.yaml # 配置文件路径
use-db-simulator=true                      # 启用数据库模拟器
```

## 🔍 故障排查

### 常见问题

#### 1. 连接问题
```bash
# 检查网络连接
ping <sui-rpc-host>
telnet <relay-host> <relay-port>

# 检查 Socket 文件
ls -la /tmp/sui_tx.sock
```

#### 2. 性能问题
```bash
# 检查系统资源
top -p $(pgrep arb)
free -h
df -h

# 检查日志
tail -f /var/log/sui-arb.log
```

#### 3. 配置问题
```bash
# 验证配置
cargo run --bin arb -- --help
echo $SUI_PRIVATE_KEY | wc -c  # 应该是 44 字符
```

### 日志分析
```bash
# 查看套利成功记录
grep "Executed tx" /var/log/sui-arb.log

# 查看错误信息
grep "ERROR" /var/log/sui-arb.log

# 查看性能指标
grep "elapsed" /var/log/sui-arb.log
```

## 📚 扩展开发

### 添加新的 DEX 协议
1. 在 `crates/dex-indexer/src/protocols/` 添加协议实现
2. 在 `bin/arb/src/defi/` 添加协议适配器
3. 更新 `supported_protocols()` 函数
4. 添加测试用例和文档

### 开发新的策略
1. 在 `bin/arb/src/strategy/` 添加策略实现
2. 实现 `Strategy` trait
3. 添加配置参数
4. 编写单元测试和集成测试

### 性能优化
1. 使用 `cargo flamegraph` 进行性能分析
2. 优化热点代码路径
3. 调整并发参数
4. 优化内存使用

## 🤝 贡献指南

### 代码规范
- 遵循 Rust 官方代码风格
- 使用 `cargo fmt` 格式化代码
- 使用 `cargo clippy` 检查代码质量
- 编写充分的单元测试

### 提交流程
1. Fork 项目并创建功能分支
2. 实现功能并添加测试
3. 确保所有测试通过
4. 提交 Pull Request
5. 代码审查和合并

## 📞 支持和联系

- **文档问题**：查看 [开发指南](./development-guide.md)
- **架构问题**：查看 [架构设计](./architecture.md)
- **历史记录**：查看 [开发日志](./devlogs/)
- **技术支持**：提交 GitHub Issue

---

> 💡 **提示**：建议开发者按照文档顺序阅读，先了解整体架构，再深入具体实现细节。

> ⚠️ **注意**：生产环境部署前请仔细阅读安全配置和风险控制相关文档。