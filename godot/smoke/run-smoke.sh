#!/usr/bin/env bash
# 真工程 headless 冒烟。诊断纪律:不许 set -e + 命令替换吞输出(follow-ups B22)。
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
GODOT_BIN="${GODOT_BIN:-/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64}"
( cd .. && cargo build -p stg-godot ) || exit 1
# 首跑 --import 生成 .godot/ 缓存;冷缓存可能 SIGABRT(坑档 G1),非致命
"$GODOT_BIN" --headless --path . --import >/dev/null 2>&1 || true
out=$("$GODOT_BIN" --headless --path . -- --smoke 2>&1)
st=$?
echo "$out"
[ "$st" -eq 0 ] && grep -q "SMOKE OK" <<<"$out"
