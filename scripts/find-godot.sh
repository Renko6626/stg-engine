#!/usr/bin/env bash
# 两个冒烟脚本共用:挑一个【可用】的 Godot 二进制并导出 GODOT_BIN / GODOT_TIMEOUT。
# 用法:在冒烟脚本里 `source "<repo>/scripts/find-godot.sh"`,失败会自行 exit 1。
#
# 选取顺序:显式 GODOT_BIN(尊重调用方,只提醒不否决)→ PATH 里的 godot → 本开发机绝对路径。
#
# 【为什么必须校验版本】godot/stg_godot.gdextension 写死 compatibility_minimum = 4.6。
# 低于 4.6 的 Godot 不加载扩展 → WorldBridge 类不存在 → main.gd/smoke.gd 脚本报错 →
# headless 进程【永不退出】(冒烟里那句 quit() 根本没跑到),表现是【挂死】而不是失败。
# 本机实测(2026-07-26):PATH 上的是 4.5.1、开发钉的是 4.6.3——不校验就会自动选中前者,
# 冒烟挂死 18 分钟无任何输出。故:自动发现的候选一律要过 ≥4.6 闸,过不了就换下一个。
GODOT_FALLBACK="${GODOT_FALLBACK:-/data/sunyunbo/playground/godot/Godot_v4.6.3-stable_linux.x86_64}"
# 单条 godot 调用的墙钟上限:挂死时把"无限等"变成"可诊断的失败"(见上,扩展没加载即挂死)
GODOT_TIMEOUT="${GODOT_TIMEOUT:-300}"

# 打印 $1 的版本号首行;取不到则空
_godot_version() { "$1" --version 2>/dev/null | head -1; }

# $1 版本号是否 ≥ 4.6(含 4.6-4.9 / 4.10+ / 5+)
_godot_version_ok() {
	case "$1" in
	4.[6-9]* | 4.[1-9][0-9]* | [5-9].*) return 0 ;;
	*) return 1 ;;
	esac
}

if [ -n "${GODOT_BIN:-}" ]; then
	# 显式指定:照用,只在版本不达标时提醒(调用方可能在有意测旧版行为)
	_v=$(_godot_version "$GODOT_BIN")
	if ! _godot_version_ok "$_v"; then
		echo "[find-godot] 警告:显式 GODOT_BIN=$GODOT_BIN 版本为 '${_v:-取不到}',低于扩展要求的 4.6;" >&2
		echo "[find-godot] 扩展将不会加载,冒烟大概率超时失败(见本文件头注释)。" >&2
	fi
else
	GODOT_BIN=""
	for _cand in godot "$GODOT_FALLBACK"; do
		command -v "$_cand" >/dev/null 2>&1 || [ -x "$_cand" ] || continue
		_v=$(_godot_version "$_cand")
		if _godot_version_ok "$_v"; then
			GODOT_BIN="$_cand"
			break
		fi
		echo "[find-godot] 跳过 $_cand(版本 '${_v:-取不到}' < 4.6)" >&2
	done
	if [ -z "$GODOT_BIN" ]; then
		echo "[find-godot] 找不到 ≥4.6 的 Godot:PATH 里没有合用的 godot,回落路径 $GODOT_FALLBACK 也不可用。" >&2
		echo "[find-godot] 装一个 Godot ≥4.6 并放进 PATH,或 GODOT_BIN=/path/to/godot 显式指定。" >&2
		exit 1
	fi
fi
export GODOT_BIN GODOT_TIMEOUT
