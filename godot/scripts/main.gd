extends Node
## GameFlow(壳子刀 2026-09-11,spec §5):流程状态机,只管切页。
## TITLE → DIFFICULTY → PLAY(NORMAL) / PRACTICE → DIFFICULTY → PLAY(PRACTICE) / REPLAYS → PLAY(REPLAY)。
## 游玩页 Play 常驻不销毁(World 跨关活着),菜单是代码生成的 Menu 层,按流程显隐。
## `--smoke`:先跑 Play.run_smoke()(既有游玩断言),再跑本文件的流程冒烟;`--shots`:直接开局。

enum F { TITLE, DIFFICULTY, PRACTICE, REPLAYS, PLAY }

const DIFFICULTIES := ["Easy", "Normal", "Hard", "Lunatic"] # 下标即 RANK_*(0..=3)
const REPLAY_DIR := "user://replays"

var flow: int = F.TITLE
var play: Play
var menu: Menu
var stg_input: StgInput
var _pending_mode: int = Play.Mode.NORMAL
var _pending_mark := 0
var _replay_files: Array[String] = []
var _smoke_reason := "" # lambda 捕获值类型局部不回写(同 play.gd 的 _smoke_saw_bgm),用成员

func _ready() -> void:
	stg_input = StgInput.new() # 注册 stg_* 动作(菜单也用 Z/X)
	add_child(stg_input)
	play = Play.new()
	add_child(play)
	play.finished.connect(_on_play_finished)
	play.set_shown(false)
	menu = Menu.new()
	add_child(menu)
	menu.chosen.connect(_on_menu_chosen)
	menu.cancelled.connect(_on_menu_cancelled)
	if "--smoke" in OS.get_cmdline_user_args():
		_run_smoke()
	elif "--shots" in OS.get_cmdline_user_args():
		_start_play(Play.Mode.NORMAL, WorldBridge.RANK_HARD, 0)
	else:
		_show_title()

# ── 页面 ─────────────────────────────────────────────────────────────────────

func _show_title() -> void:
	flow = F.TITLE
	play.set_shown(false)
	menu.visible = true
	menu.setup("東方時環譜", ["Start", "Practice", "Replay", "Quit"])

func _show_difficulty(mode: int, mark: int) -> void:
	flow = F.DIFFICULTY
	_pending_mode = mode
	_pending_mark = mark
	menu.visible = true
	menu.setup("Difficulty", DIFFICULTIES)

func _show_practice() -> void:
	flow = F.PRACTICE
	menu.visible = true
	var names: Array = []
	for e in ContentTables.PRACTICE:
		names.append(e["name"])
	menu.setup("Practice", names)

func _show_replays() -> void:
	flow = F.REPLAYS
	_replay_files = _list_replays()
	menu.visible = true
	menu.setup("Replay", _replay_files if not _replay_files.is_empty() else ["(no replays)"])

func _start_play(mode: int, rank: int, mark: int, bytes := PackedByteArray()) -> bool:
	menu.visible = false
	play.set_shown(true)
	var ok := play.start(mode, rank, mark, Play.BOOT_LIVES, PackedStringArray(), PackedStringArray(), bytes)
	if ok:
		flow = F.PLAY
	else:
		push_error("[stg] 开局失败")
		_show_title()
	return ok

# ── 菜单事件 ───────────────────────────────────────────────────────────────────

func _on_menu_chosen(i: int) -> void:
	match flow:
		F.TITLE:
			match i:
				0: _show_difficulty(Play.Mode.NORMAL, 0)
				1: _show_practice()
				2: _show_replays()
				3: get_tree().quit(0)
		F.DIFFICULTY:
			_start_play(_pending_mode, i, _pending_mark)
		F.PRACTICE:
			var e: Dictionary = ContentTables.PRACTICE[i]
			_show_difficulty(Play.Mode.PRACTICE, int(e["mark"]))
		F.REPLAYS:
			if _replay_files.is_empty():
				return
			var bytes := FileAccess.get_file_as_bytes(REPLAY_DIR.path_join(_replay_files[i]))
			if bytes.is_empty():
				push_error("[stg] 回放文件读取失败")
				return
			_start_play(Play.Mode.REPLAY, 0, 0, bytes)

func _on_menu_cancelled() -> void:
	match flow:
		F.DIFFICULTY, F.PRACTICE, F.REPLAYS:
			_show_title()

func _on_play_finished(_reason: String) -> void:
	# result / practice / playback / title 都回标题(练习回练习菜单)
	if _reason == "practice":
		_show_practice()
		play.set_shown(false)
	else:
		_show_title()

func _list_replays() -> Array[String]:
	var out: Array[String] = []
	var dir := DirAccess.open(REPLAY_DIR)
	if dir == null:
		return out
	for f in dir.get_files():
		if f.ends_with(".stgr"):
			out.append(f)
	out.sort()
	out.reverse() # 新的在前
	return out

# ── 冒烟 ──────────────────────────────────────────────────────────────────────

## 内联脚本:第 5 帧 stage_clear(1) → 结算页;确认后再 5 帧 stage_clear(0) → 结果页;
## 确认回标题;再开一局(1 条命)让一颗弹压在自机出生点 → GAME OVER → 续关 → ALIVE;存回放;
## 从文件播放到播完页。
const SMOKE_ECL := """
const BALL: int = 48;
const COLOR_CYAN: int = 8;
sub main() {
    bgm(1);
    wait(5);
    add_score(5);
    stage_clear(1);
    wait(5);
    stage_clear(0);
    loop { wait(60); }
}
"""
const SMOKE_ECL_DIE := """
const BALL: int = 48;
const COLOR_CYAN: int = 8;
sub main() {
    bgm(1);
    wait(3);
    _ = fire(BALL, COLOR_CYAN, 0.0fx, 384.0fx, 0.0fx, 0deg, none, none);
    loop { wait(60); }
}
"""

func _run_smoke() -> void:
	play.set_shown(true)
	var fails := await play.run_smoke()
	# ── 流程冒烟 ──
	var names := PackedStringArray(["smoke.ecl"])
	var srcs := PackedStringArray([SMOKE_ECL])
	fails += Play._chk(play.start(Play.Mode.NORMAL, WorldBridge.RANK_NORMAL, 0, 3, names, srcs), "flow: 内联开局")
	var w := await _wait_until(func(): return play.state == Play.S.STOPPED, 60)
	fails += Play._chk(play.state == Play.S.STOPPED and play.overlay.kind == Overlay.Kind.STAGE_CLEAR, "flow: stage_clear(1) 应盖结算页并停拍(轮次 %d)" % w)
	var f_stop := play.bridge.frame()
	await get_tree().physics_frame
	await get_tree().physics_frame
	fails += Play._chk(play.bridge.frame() == f_stop, "flow: 结算页期间不 step")
	play.confirm()
	fails += Play._chk(play.state == Play.S.PLAYING and not play.overlay.visible, "flow: 确认后恢复")
	w = await _wait_until(func(): return play.state == Play.S.STOPPED, 60)
	fails += Play._chk(play.overlay.kind == Overlay.Kind.RESULT, "flow: stage_clear(0) 应盖结果页")
	_smoke_reason = ""
	var conn := func(r: String): _smoke_reason = r
	play.finished.connect(conn)
	play.confirm()
	fails += Play._chk(_smoke_reason == "result", "flow: 结果页确认 → finished(result)")
	play.finished.disconnect(conn)
	# GAME OVER → 续关 → 存回放 → 播放
	srcs = PackedStringArray([SMOKE_ECL_DIE])
	fails += Play._chk(play.start(Play.Mode.NORMAL, WorldBridge.RANK_NORMAL, 0, 1, names, srcs), "flow: 1 条命开局")
	w = await _wait_until(func(): return play.state == Play.S.STOPPED, 120)
	fails += Play._chk(play.overlay.kind == Overlay.Kind.GAME_OVER, "flow: 1 条命被弹 → GAME OVER 页(轮次 %d)" % w)
	play.continue_game()
	await get_tree().physics_frame
	await get_tree().physics_frame
	var hp: Dictionary = play.bridge.hud_player()
	fails += Play._chk(int(hp.get("continues", 0)) == 1 and int(hp.get("lives", 0)) == 3, "flow: 续关后 continues==1、残机回 3")
	fails += Play._chk(play.state == Play.S.PLAYING, "flow: 续关后恢复 step")
	# 续关后自机重生在 (0,384) 且无敌 120 帧;弹还在原地,无敌耗尽会再死——在那之前存回放并切走
	var path := play.save_replay()
	fails += Play._chk(not path.is_empty() and FileAccess.file_exists(path), "flow: 存回放落盘 %s" % path)
	var f_rec := play.bridge.frame()
	var c_rec: int = play.bridge.checksum()
	var bytes := FileAccess.get_file_as_bytes(path)
	fails += Play._chk(play.start(Play.Mode.REPLAY, 0, 0, 3, names, srcs, bytes), "flow: 从文件开回放")
	fails += Play._chk(play.bridge.is_playback(), "flow: 播放态")
	w = await _wait_until(func(): return play.state == Play.S.STOPPED, f_rec + 60)
	fails += Play._chk(play.overlay.kind == Overlay.Kind.PLAYBACK_DONE, "flow: 播完盖播完页(轮次 %d)" % w)
	fails += Play._chk(play.bridge.frame() == f_rec and play.bridge.checksum() == c_rec, "flow: 播完帧号/校验和 == 录制时")
	DirAccess.remove_absolute(path)
	if fails == 0:
		print("SMOKE OK")
	get_tree().quit(0 if fails == 0 else 1)

func _wait_until(pred: Callable, cap: int) -> int:
	var n := 0
	while not pred.call() and n < cap:
		await get_tree().physics_frame
		n += 1
	return n
