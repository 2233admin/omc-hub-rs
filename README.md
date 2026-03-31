# omc-hub-rs

[English](#english) | [中文](#中文)

---

<a id="english"></a>

Lightweight MCP hub for Claude Code. Drop-in replacement for [oh-my-claudecode](https://github.com/Yeachan-Heo/oh-my-claudecode)'s MCP backend — a 2.5MB Rust binary replacing 663MB of bun + haiku subprocess overhead.

## The Problem

OMC's MCP infrastructure runs **three processes** just to provide tools to Claude Code:

```
bun (hub.mjs)           225 MB   MCP tool multiplexer (454 lines of JS)
claude.exe --model haiku 438 MB   Full LLM subprocess for "skill matching"
node (bridge)           ~100 MB   33 tools (state, notepad, LSP, etc.)
────────────────────────────────
Total                   ~763 MB   For JSON forwarding + file I/O + keyword lookup
```

The haiku subprocess uses a **full Claude instance** (438MB) to match user input against ~50 keywords. That's a HashMap lookup.

## The Fix

```
                    Before                          After
              ┌─────────────────┐           ┌─────────────────┐
              │   bun (hub.mjs) │ 225 MB    │                 │
              │   skill proxy   │           │  omc-hub-rs     │ 7.4 MB
              ├─────────────────┤           │  26 tools       │
              │ claude.exe      │ 438 MB    │  2.5 MB binary  │
              │ haiku subprocess│           │                 │
              ├─────────────────┤           ├─────────────────┤
              │ node bridge     │ ~100 MB   │ node bridge     │ ~100 MB
              │ 33 tools        │           │ 13 tools (LSP)  │
              └─────────────────┘           └─────────────────┘
              Total: ~763 MB                Total: ~110 MB (86% less)
```

| Component | Before | After | Savings |
|-----------|--------|-------|---------|
| Hub + skill proxy | 225 MB (bun) | 7.4 MB (Rust, measured) | 95.5% |
| Skill matcher | 438 MB (haiku LLM) | 0 MB (HashMap) | 100% |
| OMC native tools (20) | in node bridge | in Rust hub | moved |
| Node bridge | 33 tools | 13 tools (LSP only) | 60% fewer |
| Binary size | ~50 MB (node_modules) | 2.5 MB | 95% |

## Memory Benchmark

Measured on Windows 11 (Ryzen 9800X3D), idle after startup, no child MCP servers loaded:

| Process | Working Set (RSS) |
|---------|-------------------|
| `node hub.mjs` (OMC default) | **84.5 MB** |
| `omc-hub.exe` (this project) | **7.4 MB** |
| **Reduction** | **11.4x less** |

```powershell
# Reproduce
$n = Start-Process node -ArgumentList "$env:USERPROFILE/.omc/mcp-hub/hub.mjs" -PassThru -WindowStyle Hidden
Start-Sleep 4; (Get-Process -Id $n.Id).WorkingSet64 / 1MB
Stop-Process -Id $n.Id -Force

$r = Start-Process "omc-hub.exe" -PassThru -WindowStyle Hidden
Start-Sleep 4; (Get-Process -Id $r.Id).WorkingSet64 / 1MB
Stop-Process -Id $r.Id -Force
```

## 26 Tools Included

| Category | Tools | Count |
|----------|-------|-------|
| Hub Management | load_skill, unload_skill, list_skills, reload_toolbox, stats | 5 |
| Toolbox | Script execution (bash/python/node) | 1+ |
| State | read, write, clear, list_active, get_status | 5 |
| Notepad | read, write_priority, write_working, write_manual, prune, stats | 6 |
| Project Memory | read, write, add_note, add_directive | 4 |
| Trace | timeline, summary | 2 |
| Session | search | 1 |
| AST | ast_grep_search, ast_grep_replace (via sg CLI) | 2 |

## Install

**Download binary** (recommended):

```bash
# Windows
gh release download v0.2.0 --repo 2233admin/omc-hub-rs --pattern '*windows*'

# macOS (Apple Silicon)
gh release download v0.2.0 --repo 2233admin/omc-hub-rs --pattern '*macos-aarch64*'

# Linux
gh release download v0.2.0 --repo 2233admin/omc-hub-rs --pattern '*linux-x86_64*'
```

Or [download from Releases page](https://github.com/2233admin/omc-hub-rs/releases).

**Build from source:**

```bash
git clone https://github.com/2233admin/omc-hub-rs.git
cd omc-hub-rs
cargo build --release
# Binary: target/release/omc-hub (.exe on Windows)
```

## Setup

Add to `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "omc-mcp-hub": {
      "command": "/path/to/omc-hub",
      "args": ["--config", "~/.omc/mcp-hub"]
    }
  }
}
```

The `--config` path points to your existing OMC mcp-hub directory (containing `skills/` and `toolbox/`). This coexists with OMC's node bridge — no need to disable anything.

## OMC Update Compatibility

**omc-hub-rs survives OMC updates.** Here's why:

- Our hub lives in `settings.json` under key `"omc-mcp-hub"`
- OMC's bridge lives in plugin `.mcp.json` under key `"t"`
- `omc update` only touches the plugin directory, never `settings.json`
- Both servers run in parallel — 20 tools overlap harmlessly, LSP stays in node

When OMC releases a new version, check this repo for compatibility updates.

## Architecture

```
Claude Code session
    |
    |-- "omc-mcp-hub" (settings.json)
    |       |
    |       v
    |   omc-hub-rs (2.5MB Rust binary, 7.4MB runtime (measured))
    |       |-- MCP JSON-RPC 2.0 stdio server
    |       |-- Skill config loader (skills/*.json, lazy-load)
    |       |-- Child MCP proxy (stdio + HTTP transports)
    |       |-- Toolbox script runner (TOOLBOX_ACTION protocol)
    |       |-- State / Notepad / Project Memory (file I/O)
    |       |-- Trace / Session search
    |       +-- AST grep (delegates to sg CLI)
    |
    |-- "t" (OMC plugin .mcp.json)
    |       |
    |       v
    |   node bridge (~100MB, 13 tools)
    |       |-- 12 LSP tools (hover, goto, refs, rename, diagnostics...)
    |       +-- Python REPL (persistent state)
    |
    +-- other MCP servers (gitnexus, tavily, etc.)
```

## Verified

| Test | Result |
|------|--------|
| MCP initialize handshake | PASS |
| tools/list (26 tools) | PASS |
| hub_list_skills (6 skill configs) | PASS |
| hub_stats | PASS |
| Toolbox script execution | PASS |
| state_list_active | PASS |
| notepad_stats | PASS |
| project_memory_read | PASS |
| Skill load error handling | PASS |
| ping / heartbeat | PASS |
| Unknown method error (-32601) | PASS |
| Memory 7.4 MB runtime (measured) | PASS |
| Binary < 3MB | PASS |

## Related

- [cli2skill](https://github.com/2233admin/cli2skill) — Convert any CLI or MCP server into an Agent Skill (zero process overhead)
- [OMC Issue #1878](https://github.com/Yeachan-Heo/oh-my-claudecode/issues/1878) — Memory overhead report with benchmark data

## License

MIT

---

<a id="中文"></a>

# omc-hub-rs (中文)

Claude Code 的轻量级 MCP hub。替换 [oh-my-claudecode](https://github.com/Yeachan-Heo/oh-my-claudecode) 的 MCP 后端 — 用 2.5MB Rust 二进制替掉 663MB 的 bun + haiku 子进程。

## 问题

OMC 的 MCP 基础设施跑了**三个进程**：

```
bun (hub.mjs)            225 MB   MCP 工具多路复用器（454 行 JS）
claude.exe --model haiku  438 MB   完整的 LLM 子进程做"skill 匹配"
node (bridge)            ~100 MB   33 个工具（state, notepad, LSP 等）
────────────────────────────────────
总计                     ~763 MB   就为了 JSON 转发 + 文件读写 + 关键词查找
```

haiku 子进程用了一个**完整的 Claude 实例**（438MB）来匹配 ~50 个关键词。这他妈是 HashMap 就能做的事。

## 解决方案

| 组件 | 替换前 | 替换后 | 节省 |
|------|--------|--------|------|
| Hub + skill 代理 | 225 MB (bun) | 7.4 MB (Rust, measured) | 95.5% |
| Skill 匹配器 | 438 MB (haiku LLM) | 0 MB (HashMap) | 100% |
| OMC 原生工具 (20个) | 在 node bridge 里 | 在 Rust hub 里 | 迁移 |
| Node bridge | 33 工具 | 13 工具 (仅LSP) | 少 60% |
| 二进制大小 | ~50 MB (node_modules) | 2.5 MB | 95% |

## 包含 26 个工具

| 类别 | 工具 | 数量 |
|------|------|------|
| Hub 管理 | load/unload/list/reload/stats | 5 |
| 工具箱 | 脚本执行 (bash/python/node) | 1+ |
| State 状态 | read/write/clear/list_active/get_status | 5 |
| Notepad 记事本 | read/write_priority/write_working/write_manual/prune/stats | 6 |
| 项目记忆 | read/write/add_note/add_directive | 4 |
| Trace 追踪 | timeline/summary | 2 |
| Session 搜索 | search | 1 |
| AST 语法树 | ast_grep_search/replace (通过 sg CLI) | 2 |

## 安装

```bash
# 下载二进制（推荐）
gh release download v0.2.0 --repo 2233admin/omc-hub-rs --pattern '*windows*'

# 或从源码编译
git clone https://github.com/2233admin/omc-hub-rs.git
cd omc-hub-rs && cargo build --release
```

## 配置

在 `~/.claude/settings.json` 加入：

```json
{
  "mcpServers": {
    "omc-mcp-hub": {
      "command": "/path/to/omc-hub",
      "args": ["--config", "~/.omc/mcp-hub"]
    }
  }
}
```

与 OMC 的 node bridge 共存，无需禁用任何东西。

## OMC 更新兼容

**omc-hub-rs 不受 OMC 更新影响。** 我们的 hub 在 settings.json 里（key: `"omc-mcp-hub"`），OMC 的 bridge 在插件目录里（key: `"t"`）。`omc update` 只碰插件目录，不碰 settings.json。

## 相关项目

- [cli2skill](https://github.com/2233admin/cli2skill) — 把任何 CLI 或 MCP server 转成 Agent Skill（零进程开销）
- [OMC Issue #1878](https://github.com/Yeachan-Heo/oh-my-claudecode/issues/1878) — 内存占用报告 + benchmark 数据
