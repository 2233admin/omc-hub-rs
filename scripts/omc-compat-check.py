#!/usr/bin/env python3
"""omc-compat-check — Detect OMC plugin changes that might break omc-hub-rs.

Usage:
    python omc-compat-check.py --snapshot   # save current state as baseline
    python omc-compat-check.py              # check against baseline
    python omc-compat-check.py --auto       # snapshot if no baseline, else check
"""

import json
import os
import sys
from pathlib import Path

# ── Paths ──────────────────────────────────────────────────
HOME = Path(os.environ.get("USERPROFILE", os.environ.get("HOME", "~"))).resolve()
OMC_PLUGIN = HOME / ".claude" / "plugins" / "marketplaces" / "omc"
BRIDGE = OMC_PLUGIN / "bridge" / "mcp-server.cjs"
SKILLS_DIR = HOME / ".omc" / "mcp-hub" / "skills"
STATE_DIR = HOME / ".omc" / "state"
SNAPSHOT_DIR = HOME / ".omc" / "mcp-hub" / ".compat-snapshot"

RED = "\033[0;31m"
GREEN = "\033[0;32m"
YELLOW = "\033[1;33m"
NC = "\033[0m"


def get_plugin_version() -> str:
    try:
        p = OMC_PLUGIN / ".claude-plugin" / "plugin.json"
        return json.loads(p.read_text(encoding="utf-8")).get("version", "unknown")
    except Exception:
        return "unknown"


def extract_bridge_tools() -> list[str]:
    """Extract tool names by sending MCP initialize+tools/list to bridge."""
    if not BRIDGE.exists():
        return []
    import subprocess
    stdin_data = (
        '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"capabilities":{}}}\n'
        '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}\n'
    )
    try:
        result = subprocess.run(
            ["node", str(BRIDGE)],
            input=stdin_data, capture_output=True, text=True, timeout=10,
        )
        names = []
        for line in result.stdout.splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                d = json.loads(line)
                if d.get("id") == 1 and "result" in d:
                    for t in d["result"].get("tools", []):
                        names.append(t["name"])
            except json.JSONDecodeError:
                pass
        return sorted(names)
    except Exception:
        return []


def extract_skill_schemas() -> dict:
    """Extract skill config key structure (format fingerprint, not values)."""
    result = {}
    if not SKILLS_DIR.exists():
        return result
    for f in sorted(SKILLS_DIR.glob("*.json")):
        try:
            data = json.loads(f.read_text(encoding="utf-8"))
            result[f.stem] = _key_tree(data)
        except Exception as e:
            result[f.stem] = f"(parse error: {e})"
    for f in sorted(SKILLS_DIR.glob("*/skill.json")):
        try:
            data = json.loads(f.read_text(encoding="utf-8"))
            result[f.parent.name] = _key_tree(data)
        except Exception as e:
            result[f.parent.name] = f"(parse error: {e})"
    return result


def _key_tree(obj, depth=0) -> str:
    """Recursively extract key structure as a string fingerprint."""
    if not isinstance(obj, dict):
        return type(obj).__name__
    lines = []
    for k in sorted(obj.keys()):
        v = obj[k]
        prefix = "  " * depth
        if isinstance(v, dict):
            lines.append(f"{prefix}{k}/")
            lines.append(_key_tree(v, depth + 1))
        elif isinstance(v, list):
            lines.append(f"{prefix}{k}[] ({len(v)} items)")
        else:
            lines.append(f"{prefix}{k}: {type(v).__name__}")
    return "\n".join(lines)


def extract_state_format() -> dict:
    """Extract state file key structure."""
    result = {}
    if not STATE_DIR.exists():
        return result
    for f in sorted(STATE_DIR.glob("*-state.json")):
        try:
            data = json.loads(f.read_text(encoding="utf-8"))
            result[f.name] = _key_tree(data)
        except Exception as e:
            result[f.name] = f"(parse error: {e})"
    return result


def snapshot():
    """Save current state as baseline."""
    SNAPSHOT_DIR.mkdir(parents=True, exist_ok=True)
    ver = get_plugin_version()
    data = {
        "version": ver,
        "bridge_tools": extract_bridge_tools(),
        "skill_schemas": extract_skill_schemas(),
        "state_format": extract_state_format(),
    }
    (SNAPSHOT_DIR / "baseline.json").write_text(
        json.dumps(data, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    print(f"{GREEN}Snapshot saved{NC} (OMC v{ver}) -> {SNAPSHOT_DIR}")


def check():
    """Compare current state against baseline."""
    baseline_path = SNAPSHOT_DIR / "baseline.json"
    if not baseline_path.exists():
        print(f"{YELLOW}No baseline snapshot found. Run with --snapshot first.{NC}")
        sys.exit(1)

    old = json.loads(baseline_path.read_text(encoding="utf-8"))
    old_ver = old.get("version", "?")
    new_ver = get_plugin_version()

    print(f"OMC version: {old_ver} -> {new_ver}")
    print("=" * 50)

    changes = 0

    # Bridge tools diff
    old_tools = set(old.get("bridge_tools", []))
    new_tools = set(extract_bridge_tools())
    added = new_tools - old_tools
    removed = old_tools - new_tools
    if added or removed:
        print(f"\n{RED}! Bridge tools changed:{NC}")
        for t in sorted(added):
            print(f"  {GREEN}+ {t}{NC}")
        for t in sorted(removed):
            print(f"  {RED}- {t}{NC}")
        changes += 1
    else:
        print(f"{GREEN}OK Bridge tools: no change ({len(old_tools)} tools){NC}")

    # Skill schema diff
    old_skills = old.get("skill_schemas", {})
    new_skills = extract_skill_schemas()
    all_skill_names = sorted(set(list(old_skills.keys()) + list(new_skills.keys())))
    skill_changes = []
    for name in all_skill_names:
        o = old_skills.get(name)
        n = new_skills.get(name)
        if o is None:
            skill_changes.append(f"  {GREEN}+ {name} (new skill){NC}")
        elif n is None:
            skill_changes.append(f"  {RED}- {name} (removed){NC}")
        elif o != n:
            skill_changes.append(f"  {YELLOW}~ {name} (schema changed){NC}")
    if skill_changes:
        print(f"\n{RED}! Skill schemas changed:{NC}")
        for line in skill_changes:
            print(line)
        changes += 1
    else:
        print(f"{GREEN}OK Skill schemas: no change ({len(old_skills)} skills){NC}")

    # State format diff
    old_state = old.get("state_format", {})
    new_state = extract_state_format()
    all_state_names = sorted(set(list(old_state.keys()) + list(new_state.keys())))
    state_changes = []
    for name in all_state_names:
        o = old_state.get(name)
        n = new_state.get(name)
        if o is None:
            state_changes.append(f"  {GREEN}+ {name} (new){NC}")
        elif n is None:
            state_changes.append(f"  {YELLOW}- {name} (removed){NC}")
        elif o != n:
            state_changes.append(f"  {YELLOW}~ {name} (format changed){NC}")
    if state_changes:
        print(f"\n{YELLOW}! State format changed:{NC}")
        for line in state_changes:
            print(line)
        changes += 1
    else:
        print(f"{GREEN}OK State format: no change{NC}")

    # Summary
    print()
    if changes > 0:
        print(f"{RED}Found {changes} change(s) — check omc-hub-rs compatibility{NC}")
        print("  https://github.com/2233admin/omc-hub-rs")
        sys.exit(1)
    else:
        print(f"{GREEN}All clear — omc-hub-rs compatible with OMC v{new_ver}{NC}")
        # Auto-refresh snapshot
        snapshot()


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode == "--snapshot":
        snapshot()
    elif mode == "--auto":
        if (SNAPSHOT_DIR / "baseline.json").exists():
            check()
        else:
            snapshot()
    else:
        check()


if __name__ == "__main__":
    main()
