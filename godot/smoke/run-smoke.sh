#!/usr/bin/env bash
# 真工程 headless 冒烟。诊断纪律:不许 set -e + 命令替换吞输出(follow-ups B22)。
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
# 选 Godot(含 ≥4.6 版本闸)+ 超时上限:低版本会让 headless 挂死而非报错,详见该文件头注释
. ../scripts/find-godot.sh
( cd .. && cargo build -p stg-godot ) || exit 1
# 首跑 --import 生成 .godot/ 缓存;冷缓存可能 SIGABRT(坑档 G1),非致命
timeout "$GODOT_TIMEOUT" "$GODOT_BIN" --headless --path . --import >/dev/null 2>&1 || true
out=$(timeout "$GODOT_TIMEOUT" "$GODOT_BIN" --headless --path . -- --smoke 2>&1)
st=$?
echo "$out"
[ "$st" -eq 124 ] && echo "[run-smoke] 超时 ${GODOT_TIMEOUT}s——常见原因:扩展未加载(Godot <4.6 / 产物路径不符),脚本报错后 headless 不自退" >&2
# shader 编译错误只打 ERROR 到 stdout、不改变 Godot 退出码,SMOKE OK 照打——必须单独判失败,
# 否则坏 shader 会静默混过冒烟(2026-09-25 激光池刀 Task 6 复盘)。
if grep -qE "SHADER ERROR|Shader compilation failed" <<<"$out"; then
	echo "[run-smoke] shader 编译失败" >&2
	exit 1
fi
[ "$st" -eq 0 ] && grep -q "SMOKE OK" <<<"$out"
