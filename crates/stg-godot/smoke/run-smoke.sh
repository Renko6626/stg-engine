#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
GODOT_BIN="${GODOT_BIN:-/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64}"
( cd ../../.. && cargo build -p stg-godot )
# 首跑生成 .godot/ 资源缓存(含扩展清单);失败不致命,真判定在下一步
"$GODOT_BIN" --headless --path . --import >/dev/null 2>&1 || true
out=$("$GODOT_BIN" --headless --path . --script res://smoke.gd 2>&1)
echo "$out"
grep -q "SMOKE OK" <<<"$out"
