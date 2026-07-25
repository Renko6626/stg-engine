extends Node
## 状态机 + 每帧回路(spec §6)。宿主暂停 = 不调 step_frame(世界时间线零帧)。

enum S { PLAYING, PAUSED, STAGE_CLEAR }

var state: int = S.PLAYING
var bridge: WorldBridge
var stg_input: StgInput
var playfield: Playfield
var dispatcher: Dispatcher
var hud: Hud
var effects: Effects
var smoke := false

const SMOKE_SRC := "sub main() { bgm(3); loop { wait(60); } }"

func _ready() -> void:
	smoke = "--smoke" in OS.get_cmdline_user_args()
	bridge = WorldBridge.new()
	add_child(bridge)
	stg_input = StgInput.new()
	add_child(stg_input)
	playfield = Playfield.new()
	add_child(playfield)
	effects = Effects.new()
	playfield.world_root.get_node("FxRoot").add_child(effects)
	hud = Hud.new()
	add_child(hud)
	dispatcher = Dispatcher.new()
	add_child(dispatcher)
	_wire_requests()
	if smoke:
		_run_smoke() # async,自行 quit
	else:
		if not _boot(0):
			push_error("[stg] 开局失败")
			get_tree().quit(1)

func _wire_requests() -> void:
	dispatcher.register(Dispatcher.REQ_ENEMY_DEATH, func(a):
		effects.explosion(Vector2(a[0] / 65536.0, a[1] / 65536.0), int(a[3])))
	dispatcher.register(Dispatcher.REQ_SPELL_DECLARE, func(a):
		hud.show_banner(ContentTables.SPELL_NAMES.get(int(a[0]), "Spell #%d" % int(a[0])), 2.5))
	dispatcher.register(Dispatcher.REQ_SPELL_RESULT, func(a):
		hud.show_banner("取得!" if int(a[1]) == 1 else "失敗…", 2.0))
	dispatcher.register(Dispatcher.REQ_STAGE_CLEAR, func(_a): _on_stage_clear())
	dispatcher.register(Dispatcher.REQ_BGM, func(a):
		hud.set_bgm_label(ContentTables.BGM_NAMES.get(int(a[0]), "BGM #%d" % int(a[0]))))
	dispatcher.register(Dispatcher.REQ_BG, func(a): playfield.bg.set_bg(int(a[0])))
	dispatcher.register(Dispatcher.REQ_BG_PHASE, func(a): playfield.bg.set_phase(int(a[0])))

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
		if not playfield.setup(bridge):
			return false
		playfield.update_view(bridge, 0) # 首帧闪位修:免自机在 (0,0) 停一帧才对上真位置
	return ok

## 双表示规矩:电平追平(开机/读档后一次性对表 hud/bg),含中段开机/读档。真正不可替代
## 的场合是 load_state(读档口未建,T7 follow-up)——new_game_at 路径本身也会在随后
## 经 REQ_BGM/REQ_BG 边沿事件追平,这里只是免去开局瞬间的占位闪烁。
func _sync_anchors() -> void:
	var a := bridge.anchors()
	if a.is_empty():
		push_error("[stg] anchors 空(未开局?)")
		return
	# new_game_at 后锚点字段要到第 2 个 step 帧才被脚本写入,此刻读到的 0 是引擎侧
	# 默认初值(0 = 保留无效值,render-contract id 分区),不是真实 bgm/bg 号——跳过
	# 写标签/换色,免打一帧假 "BGM #0"/0 号底色;bg_phase 的 0(=滚动)是合法态,照设。
	var bgm_id := int(a["bgm"])
	if bgm_id != 0:
		hud.set_bgm_label(ContentTables.BGM_NAMES.get(bgm_id, "BGM #%d" % bgm_id))
	var bg_id := int(a["bg"])
	if bg_id != 0:
		playfield.bg.set_bg(bg_id)
	playfield.bg.set_phase(int(a["bg_phase"]))

func _physics_process(_dt: float) -> void:
	if state == S.PAUSED:
		hud.show_banner("PAUSE", 0.1) # 每帧续,判据只有一条:PAUSED 不 step
		return
	if state != S.PLAYING:
		return
	var buttons := stg_input.mask() # 算一次,step_frame/update_view 共用(避免帧内读两次输入分叉)
	bridge.step_frame(buttons)
	_after_step(buttons)

func _after_step(buttons: int) -> void:
	dispatcher.drain(bridge.take_requests())
	hud.refresh(bridge)
	playfield.update_view(bridge, buttons)

func _on_stage_clear() -> void:
	state = S.STAGE_CLEAR
	var p := bridge.hud_player()
	hud.show_banner("STAGE CLEAR  Score %d  (Z restart)" % int(p.get("score", 0)), 3600.0)

func _unhandled_input(ev: InputEvent) -> void:
	if ev.is_action_pressed("ui_cancel"):
		if state == S.PLAYING:
			state = S.PAUSED
		elif state == S.PAUSED:
			state = S.PLAYING
	elif state == S.STAGE_CLEAR and ev.is_action_pressed("stg_shot"):
		if _boot(0):
			hud.hide_banner()
			state = S.PLAYING
		else:
			push_error("[stg] 重开失败")

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
	var mm: MultiMesh = playfield.layer_nodes[WorldBridge.LAYER_BULLETS].multimesh
	var buf := RenderingServer.multimesh_get_buffer(mm.get_rid())
	fails += _chk(buf.size() == 8192 * 12, "bullets 缓冲尺寸")
	if fails == 0:
		print("SMOKE OK")
	get_tree().quit(0 if fails == 0 else 1)

func _chk(cond: bool, msg: String) -> int:
	if not cond:
		print("SMOKE FAIL: ", msg)
		return 1
	return 0
