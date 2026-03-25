# omc-hub-rs

Lightweight MCP hub for Claude Code. Drop-in replacement for [oh-my-claudecode](https://github.com/Yeachan-Heo/oh-my-claudecode)'s `mcp-hub` — a 2.5MB Rust binary that replaces 663MB of bun + haiku subprocess overhead.

## The Problem

OMC's MCP hub runs as:
- **bun process** (225MB) — `hub.mjs`, a 454-line MCP tool multiplexer
- **claude.exe --model haiku** (438MB) — a full Claude subprocess for "skill matching"
- **Total: 663MB** for what is essentially JSON forwarding + keyword lookup

## The Fix

| | Old (OMC) | New (omc-hub-rs) | Savings |
|---|-----------|-------------------|---------|
| Hub process | 225 MB (bun) | ~10 MB (Rust) | 95.5% |
| Skill matcher | 438 MB (haiku) | 0 MB (HashMap) | 100% |
| Binary size | ~50 MB (node) | 2.5 MB | 95% |
| **Total** | **663 MB** | **~10 MB** | **97%** |

## Features

- MCP JSON-RPC 2.0 over stdio (hand-rolled, no SDK bloat)
- Lazy-loading skill configs from `skills/*.json`
- Child MCP proxy (stdio + HTTP transports)
- Toolbox script execution (bash/python/node via `TOOLBOX_ACTION` protocol)
- Hub management tools: `hub_load_skill`, `hub_unload_skill`, `hub_list_skills`, `hub_reload_toolbox`, `hub_stats`
- Namespace isolation: `skill__<name>__<tool>`, `toolbox__<name>`
- Full compatibility with existing OMC skill configs

## Install

Download the binary for your platform from [Releases](https://github.com/2233admin/omc-hub-rs/releases).

Or build from source:

```bash
cargo build --release
# Binary at target/release/omc-hub (or omc-hub.exe on Windows)
```

## Usage

Replace the `omc-mcp-hub` entry in `~/.claude/settings.json`:

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

The `--config` path should point to your existing OMC mcp-hub directory (containing `skills/` and `toolbox/`).

## Verified

| Test | Result |
|------|--------|
| MCP initialize handshake | PASS |
| tools/list (6 hub tools + toolbox) | PASS |
| hub_list_skills (6 skill configs) | PASS |
| hub_stats | PASS |
| Toolbox script execution | PASS |
| ping / heartbeat | PASS |
| Unknown method error (-32601) | PASS |
| Skill load error handling | PASS |
| Memory < 10MB runtime | PASS |
| Binary < 3MB | PASS |

## Architecture

```
claude.exe (main session)
    | stdio JSON-RPC
    v
omc-hub-rs (~10MB)
    |-- Skill Config Loader (skills/*.json)
    |-- Tool Multiplexer (child MCP proxy)
    |-- Toolbox Runner (script execution)
    +-- Hub Management (load/unload/list/stats)
```

## Why Not Just Fix OMC?

OMC is a JS/TS plugin ecosystem. The 663MB overhead comes from fundamental architecture choices:
1. Running on bun/node runtime (225MB baseline)
2. Spawning a full `claude.exe` subprocess for LLM-based skill matching when a HashMap suffices

This can't be fixed within the JS ecosystem — it needs a native binary. This project provides that binary as an optional drop-in replacement.

## License

MIT
