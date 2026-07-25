extends Node
## 状态机 + 每帧回路(spec §6)。宿主暂停 = 不调 step_frame(世界时间线零帧)。

enum S { PLAYING, PAUSED, STAGE_CLEAR }

var state: int = S.PLAYING
var bridge: WorldBridge
var stg_input: StgInput
var smoke := false

const SMOKE_SRC := "sub main() { bgm(3); loop { wait(60); } }"

func _ready() -> void:
	smoke = "--smoke" in OS.get_cmdline_user_args()
	bridge = WorldBridge.new()
	add_child(bridge)
	stg_input = StgInput.new()
	add_child(stg_input)
	if smoke:
		_run_smoke() # async,自行 quit
	else:
		if not _boot(0):
			push_error("[stg] 开局失败")
			get_tree().quit(1)

## 读 res://ecl/demo/*.ecl(按名排序)开局;T3 期目录还没有内容 → 回退内置最小源。
func _boot(start: int) -> bool:
	var names := PackedStringArray()
	var sources := PackedStringArray()
	var dir := DirAccess.open("res://ecl/demo")
	if dir != null:
		var files: Array[String] = []
		for f in dir.get_files():
			if f.ends_with(".ecl"):
				files.append(f)
		files.sort()
		for f in files:
			names.append(f)
			sources.append(FileAccess.get_file_as_string("res://ecl/demo/" + f))
	if names.is_empty():
		names.append("inline.ecl")
		sources.append(SMOKE_SRC)
	var ok := bridge.new_game_at(names, sources, 1, 2, start, 0, 0, 3, 3)
	if ok:
		_sync_anchors() # 双表示规矩:开机后一次性对电平(T5 实装演出)
	return ok

func _sync_anchors() -> void:
	var a := bridge.anchors() # T5 起喂给 hud/bg;T3 只留读口热身
	if a.is_empty():
		push_error("[stg] anchors 空(未开局?)")

func _physics_process(_dt: float) -> void:
	if state != S.PLAYING:
		return
	bridge.step_frame(stg_input.mask())
	_after_step()

## T4(渲染)/T5(分发器 HUD)在此挂逐帧消费;T3 空置。
func _after_step() -> void:
	pass

func _unhandled_input(ev: InputEvent) -> void:
	if ev.is_action_pressed("ui_cancel"):
		if state == S.PLAYING:
			state = S.PAUSED
		elif state == S.PAUSED:
			state = S.PLAYING

## ── 冒烟(v0:内置源;T6 换 demo 两次开机)────────────────────────────
func _run_smoke() -> void:
	var fails := 0
	if not _boot(0):
		print("SMOKE FAIL: boot")
		get_tree().quit(1)
		return
	# 实测诊断(非简报预判):`physics_frame` 信号在每 tick 的 `_physics_process` 调用**之前**
	# 触发(Godot 4.6 实况),故简报原版 `for i in 60: await ...physics_frame` 只兑现 59 次
	# 真实 step_frame(差一帧,非"物理 tick 不走")。改轮询实际读口而非数信号次数,连锁免疫
	# 同类差一错;120 轮上限防信号真不来时死等。
	var waited := 0
	while bridge.frame() < 60 and waited < 120:
		await get_tree().physics_frame
		waited += 1
	fails += _chk(bridge.frame() >= 60, "frame>=60, got %d" % bridge.frame())
	fails += _chk(bridge.checksum() != 0, "checksum!=0")
	fails += _chk(int(bridge.anchors().get("bgm", -1)) == 3, "anchors.bgm==3")
	if fails == 0:
		print("SMOKE OK")
	get_tree().quit(0 if fails == 0 else 1)

func _chk(cond: bool, msg: String) -> int:
	if not cond:
		print("SMOKE FAIL: ", msg)
		return 1
	return 0
