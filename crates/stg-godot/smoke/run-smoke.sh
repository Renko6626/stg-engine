#!/usr/bin/env bash
# B22 修复:set -e + 命令替换会在 godot 非零退出时吞掉全部诊断——改为先落地再判定。
set -uo pipefail
cd "$(dirname "$0")"
GODOT_BIN="${GODOT_BIN:-/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64}"
( cd ../../.. && cargo build -p stg-godot ) || exit 1
"$GODOT_BIN" --headless --path . --import >/dev/null 2>&1 || true
out=$("$GODOT_BIN" --headless --path . --script res://smoke.gd 2>&1)
st=$?
echo "$out"
[ "$st" -eq 0 ] && grep -q "SMOKE OK" <<<"$out"
