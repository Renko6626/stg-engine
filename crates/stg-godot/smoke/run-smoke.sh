#!/usr/bin/env bash
# B22 修复:set -e + 命令替换会在 godot 非零退出时吞掉全部诊断——改为先落地再判定。
set -uo pipefail
cd "$(dirname "$0")" || exit 1
# 选 Godot(含 ≥4.6 版本闸)+ 超时上限:低版本会让 headless 挂死而非报错,详见该文件头注释
. ../../../scripts/find-godot.sh
( cd ../../.. && cargo build -p stg-godot ) || exit 1
timeout "$GODOT_TIMEOUT" "$GODOT_BIN" --headless --path . --import >/dev/null 2>&1 || true
out=$(timeout "$GODOT_TIMEOUT" "$GODOT_BIN" --headless --path . --script res://smoke.gd 2>&1)
st=$?
echo "$out"
[ "$st" -eq 124 ] && echo "[run-smoke] 超时 ${GODOT_TIMEOUT}s——常见原因:扩展未加载(Godot <4.6 / 产物路径不符),脚本报错后 headless 不自退" >&2
[ "$st" -eq 0 ] && grep -q "SMOKE OK" <<<"$out"
