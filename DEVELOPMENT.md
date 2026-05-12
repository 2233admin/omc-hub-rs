# Development Guide

[English](#english) | [中文](#中文)

---

<a id="english"></a>

## Prerequisites

- **Rust** 1.75+ (edition 2024, tested on 1.85+)
- **cargo** (comes with Rust)
- **git** for version control

## Project Structure

```
omc-hub-rs/
├── src/
│   ├── main.rs          # Entry point, async main, message loop
│   ├── hub.rs           # Hub lifecycle, skill registry, tool dispatch
│   ├── child.rs         # Child MCP client (stdio + HTTP transports)
│   ├── config.rs        # Skill config loading from skills/*.json
│   ├── protocol.rs      # JSON-RPC 2.0 types, MCP tool schema
│   ├── omc_tools.rs     # OMC native tools (state, notepad, memory)
│   ├── toolbox.rs       # Toolbox script runner (bash/python/node)
│   └── ...
├── tests/
│   └── mcp_compliance.rs  # MCP protocol + process verification tests
├── scripts/               # Build + release scripts
├── Cargo.toml
└── Cargo.lock
```

### Core Modules

| Module | Purpose |
|--------|---------|
| `main.rs` | Stdin reader, tokio runtime, signal handling, main loop |
| `hub.rs` | Skill lifecycle, tool registry, dispatch routing, stats |
| `child.rs` | Stdio subprocess + HTTP client for child MCP servers |
| `config.rs` | Load `skills/*.json` and `skills/*/skill.json` |
| `protocol.rs` | JSON-RPC 2.0 types, `ToolDef`, `ToolResult` |
| `omc_tools.rs` | Built-in tools: state, notepad, project memory, trace |
| `toolbox.rs` | Execute scripts from `toolbox/` directory |

## Building

```bash
# Development build (fast, debug symbols)
cargo build

# Release build (optimized, ~2.5MB binary)
cargo build --release

# Check without building
cargo check --all-targets

# Run tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Lint
cargo clippy -- -D warnings
```

## Binary Output

| Build | Path | Size |
|-------|------|------|
| Debug | `target/debug/omc-hub.exe` | ~10 MB |
| Release | `target/release/omc-hub.exe` | ~2.5 MB |

## Architecture

### Message Flow

```
Claude Code
    │
    ▼
main.rs: stdin reader (std::sync::mpsc → tokio channel)
    │
    ▼
handle_message() → JsonRpcRequest
    │
    ├─ "initialize"  → return server info
    ├─ "tools/list"   → hub.list_tools()
    ├─ "tools/call"  → hub.call_tool()
    └─ "ping"         → heartbeat
    │
    ▼
hub.call_tool() dispatch:
    │
    ├─ hub_* tools  → management handlers
    ├─ omc_* tools  → omc_tools.rs
    ├─ toolbox tools → toolbox.rs script runner
    └─ skill_* tools → child.rs → child MCP server
```

### Hub Lifecycle

```
Hub::new(base_dir, state_dir)
    │
    ├─ load_skill_configs()      // scan skills/*.json
    ├─ load_stats()              // restore stats.json
    └─ scan_toolbox()            // discover toolbox scripts

hub.call_tool(name, args)
    │
    ├─ hub_load_skill   → spawn child processes, register tools
    ├─ hub_unload_skill → terminate children, clean registry
    └─ tool dispatch    → OMC / toolbox / skill proxy

hub.shutdown()
    │
    ├─ unload all skills (close children)
    └─ flush_stats()   // write stats.json
```

### Skill Loading

```
hub_load_skill("skill-name")
    │
    ├─ lookup SkillConfig from skill_configs
    ├─ for each mcp_server in config:
    │   ├─ ChildMcp::connect()  → spawn stdio subprocess OR create HTTP client
    │   ├─ send initialize handshake
    │   └─ tools/list → register tools with namespace prefix
    └─ scan skill_dir/toolbox (if present) → register scripts
```

### State Persistence

| Data | Location | Format |
|------|----------|--------|
| Skill configs | `{config}/skills/*.json` | JSON |
| Toolbox scripts | `{config}/toolbox/*` | Shell/Python/Node |
| OMC state | `{state}/state.json` | JSON |
| Notepad | `{state}/notepad/` | Text files |
| Project memory | `{state}/memory/` | Markdown |
| Call stats | `{config}/stats.json` | JSON |

## Testing

### Test Categories

| Test File | Coverage |
|-----------|----------|
| `tests/mcp_compliance.rs` | MCP protocol, process lifecycle, memory |

### Run All Tests

```bash
cargo test
```

### Test Output

```
running 16 tests
    test_initialize_response_format       ... ok
    test_tools_list_response_format     ... ok
    test_tools_call_success              ... ok
    test_tools_call_unknown_tool_returns_error_in_content ... ok
    test_ping_response                   ... ok
    test_unknown_method_returns_error    ... ok
    test_parse_error_returns_32700       ... ok
    test_notification_no_response         ... ok
    test_id_types_string_and_number      ... ok
    test_tools_list_changed_notification_after_load ... ok
    test_omc_state_roundtrip            ... ok
    test_binary_size_under_15mb         ... ok
    test_startup_time_under_2s          ... ok
    test_graceful_shutdown_on_stdin_close ... ok
    test_multiple_rapid_requests         ... ok
    test_hub_stats_tracking             ... ok

test result: ok. 16 passed; 0 failed
```

### Manual Verification

```powershell
# Memory benchmark
$n = Start-Process node -ArgumentList "$env:USERPROFILE/.omc/mcp-hub/hub.mjs" -PassThru
Start-Sleep 4; (Get-Process -Id $n.Id).WorkingSet64 / 1MB  # ~84 MB

$r = Start-Process "target/release/omc-hub.exe" -PassThru
Start-Sleep 4; (Get-Process -Id $r.Id).WorkingSet64 / 1MB  # ~7 MB
```

## Debugging

### Enable Trace Logging

```bash
# Via environment variable
RUST_LOG=omc_hub=debug ./target/release/omc-hub --config ~/.omc/mcp-hub

# Via command line (uses stderr for logs)
# Logs go to stderr by default
```

### Trace JSON-RPC Traffic

Add `tracing::info!("-> {:?}", req)` in `handle_message()` to log incoming requests.

### Common Issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| "Method not found" | Wrong method name | Check MCP spec 2024-11-05 |
| Timeout on skill load | Child process hang | Check child stderr, increase timeout |
| Tools not showing | Skill config parse error | Validate JSON schema |
| Memory leak | Children not closed | Ensure `child.close()` in unload |

## Release Process

```bash
# 1. Update version in Cargo.toml
# 2. Update version in src/main.rs (serverInfo)
# 3. Update tests/mcp_compliance.rs (clientInfo)

# 4. Build all targets
cargo build --release
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target aarch64-apple-darwin

# 5. Run tests
cargo test --release

# 6. Create GitHub release
gh release create vX.Y.Z \
  --title "omc-hub-rs vX.Y.Z" \
  --notes "Changes..." \
  target/release/omc-hub.exe \
  target/release/omc-hub
```

## Contributing

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Make changes with tests
4. Run `cargo clippy -- -D warnings` and `cargo test`
5. Commit using conventional commits: `git commit -m "feat: add something"`
6. Push and create a PR

---

<a id="中文"></a>

## 环境要求

- **Rust** 1.75+ (edition 2024, 测试于 1.85+)
- **cargo** (随 Rust 安装)
- **git** 版本控制

## 项目结构

```
omc-hub-rs/
├── src/
│   ├── main.rs          # 入口点, async main, 消息循环
│   ├── hub.rs           # Hub 生命周期, skill 注册表, 工具分发
│   ├── child.rs         # 子 MCP 客户端 (stdio + HTTP)
│   ├── config.rs        # 从 skills/*.json 加载 skill 配置
│   ├── protocol.rs      # JSON-RPC 2.0 类型, MCP tool schema
│   ├── omc_tools.rs     # OMC 原生工具 (state, notepad, memory)
│   ├── toolbox.rs       # Toolbox 脚本执行器 (bash/python/node)
│   └── ...
├── tests/
│   └── mcp_compliance.rs  # MCP 协议 + 进程验证测试
├── scripts/               # 构建 + 发布脚本
├── Cargo.toml
└── Cargo.lock
```

### 核心模块

| 模块 | 职责 |
|------|------|
| `main.rs` | stdin 读取器, tokio runtime, 信号处理, 主循环 |
| `hub.rs` | Skill 生命周期, 工具注册表, 分发放路由, 统计 |
| `child.rs` | Stdio 子进程 + HTTP 客户端连接子 MCP 服务器 |
| `config.rs` | 加载 `skills/*.json` 和 `skills/*/skill.json` |
| `protocol.rs` | JSON-RPC 2.0 类型, `ToolDef`, `ToolResult` |
| `omc_tools.rs` | 内置工具: state, notepad, project memory, trace |
| `toolbox.rs` | 执行 `toolbox/` 目录下的脚本 |

## 构建

```bash
# 开发构建 (快速, 包含调试符号)
cargo build

# 发布构建 (优化, ~2.5MB 二进制)
cargo build --release

# 仅检查不构建
cargo check --all-targets

# 运行测试
cargo test

# 带输出运行测试
cargo test -- --nocapture

# 代码检查
cargo clippy -- -D warnings
```

## 二进制输出

| 构建类型 | 路径 | 大小 |
|----------|------|------|
| Debug | `target/debug/omc-hub.exe` | ~10 MB |
| Release | `target/release/omc-hub.exe` | ~2.5 MB |

## 架构

### 消息流程

```
Claude Code
    │
    ▼
main.rs: stdin 读取器 (std::sync::mpsc → tokio channel)
    │
    ▼
handle_message() → JsonRpcRequest
    │
    ├─ "initialize"  → 返回服务器信息
    ├─ "tools/list"  → hub.list_tools()
    ├─ "tools/call"  → hub.call_tool()
    └─ "ping"        → 心跳
    │
    ▼
hub.call_tool(name, args) 分发:
    │
    ├─ hub_* 工具  → 管理处理器
    ├─ omc_* 工具  → omc_tools.rs
    ├─ toolbox 工具 → toolbox.rs 脚本执行器
    └─ skill_* 工具 → child.rs → 子 MCP 服务器
```

### Hub 生命周期

```
Hub::new(base_dir, state_dir)
    │
    ├─ load_skill_configs()      // 扫描 skills/*.json
    ├─ load_stats()              // 恢复 stats.json
    └─ scan_toolbox()            // 发现 toolbox 脚本

hub.call_tool(name, args)
    │
    ├─ hub_load_skill   → 启动子进程, 注册工具
    ├─ hub_unload_skill → 终止子进程, 清理注册表
    └─ 工具分发          → OMC / toolbox / skill 代理

hub.shutdown()
    │
    ├─ 卸载所有 skills (关闭子进程)
    └─ flush_stats()   // 写入 stats.json
```

### Skill 加载

```
hub_load_skill("skill-name")
    │
    ├─ 从 skill_configs 查找 SkillConfig
    ├─ 遍历 config 中的每个 mcp_server:
    │   ├─ ChildMcp::connect()  → 启动 stdio 子进程 或 创建 HTTP 客户端
    │   ├─ 发送 initialize 握手
    │   └─ tools/list → 注册工具并添加命名空间前缀
    └─ 扫描 skill_dir/toolbox (如果有) → 注册脚本
```

### 状态持久化

| 数据 | 位置 | 格式 |
|------|------|------|
| Skill 配置 | `{config}/skills/*.json` | JSON |
| Toolbox 脚本 | `{config}/toolbox/*` | Shell/Python/Node |
| OMC 状态 | `{state}/state.json` | JSON |
| 记事本 | `{state}/notepad/` | 文本文件 |
| 项目记忆 | `{state}/memory/` | Markdown |
| 调用统计 | `{config}/stats.json` | JSON |

## 测试

### 测试类别

| 测试文件 | 覆盖范围 |
|----------|----------|
| `tests/mcp_compliance.rs` | MCP 协议, 进程生命周期, 内存 |

### 运行所有测试

```bash
cargo test
```

### 测试输出

```
running 16 tests
    test_initialize_response_format       ... ok
    test_tools_list_response_format       ... ok
    test_tools_call_success               ... ok
    test_tools_call_unknown_tool_returns_error_in_content ... ok
    test_ping_response                   ... ok
    test_unknown_method_returns_error    ... ok
    test_parse_error_returns_32700       ... ok
    test_notification_no_response         ... ok
    test_id_types_string_and_number      ... ok
    test_tools_list_changed_notification_after_load ... ok
    test_omc_state_roundtrip            ... ok
    test_binary_size_under_15mb          ... ok
    test_startup_time_under_2s          ... ok
    test_graceful_shutdown_on_stdin_close ... ok
    test_multiple_rapid_requests         ... ok
    test_hub_stats_tracking              ... ok

test result: ok. 16 passed; 0 failed
```

### 手动验证

```powershell
# 内存基准测试
$n = Start-Process node -ArgumentList "$env:USERPROFILE/.omc/mcp-hub/hub.mjs" -PassThru
Start-Sleep 4; (Get-Process -Id $n.Id).WorkingSet64 / 1MB  # ~84 MB

$r = Start-Process "target/release/omc-hub.exe" -PassThru
Start-Sleep 4; (Get-Process -Id $r.Id).WorkingSet64 / 1MB  # ~7 MB
```

## 调试

### 启用跟踪日志

```bash
# 通过环境变量
RUST_LOG=omc_hub=debug ./target/release/omc-hub --config ~/.omc/mcp-hub

# 通过命令行 (日志输出到 stderr)
```

### 跟踪 JSON-RPC 流量

在 `handle_message()` 中添加 `tracing::info!("-> {:?}", req)` 来记录传入请求。

### 常见问题

| 症状 | 原因 | 修复 |
|------|------|------|
| "Method not found" | 方法名错误 | 检查 MCP spec 2024-11-05 |
| Skill 加载超时 | 子进程挂起 | 检查子进程 stderr, 增加超时时间 |
| 工具不显示 | Skill 配置解析错误 | 验证 JSON schema |
| 内存泄漏 | 子进程未关闭 | 确保 unload 时调用 `child.close()` |

## 发布流程

```bash
# 1. 更新 Cargo.toml 中的版本号
# 2. 更新 src/main.rs 中的 serverInfo 版本
# 3. 更新 tests/mcp_compliance.rs 中的 clientInfo 版本

# 4. 构建所有目标平台
cargo build --release
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target aarch64-apple-darwin

# 5. 运行测试
cargo test --release

# 6. 创建 GitHub release
gh release create vX.Y.Z \
  --title "omc-hub-rs vX.Y.Z" \
  --notes "Changes..." \
  target/release/omc-hub.exe \
  target/release/omc-hub
```

## 贡献指南

1. Fork 仓库
2. 创建功能分支: `git checkout -b feature/my-feature`
3. 添加更改和测试
4. 运行 `cargo clippy -- -D warnings` 和 `cargo test`
5. 使用 conventional commits 提交: `git commit -m "feat: add something"`
6. 推送并创建 PR
