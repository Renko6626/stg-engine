class_name Play
extends Node
## 游玩页(壳子刀 2026-09-11:原 main.gd 的游玩部分原样搬入;流程归 main.gd 的 GameFlow)。
## 三种模式共用一条 step 回路:NORMAL / PRACTICE 用 step_frame(输入来自玩家),REPLAY 用
## playback_step(输入来自日志)。关间/GAME OVER/结果页是盖在冻结画面上的 Overlay,STOPPED 态
## 不 step;确认/续关/回标题由 GameFlow 经 confirm()/back()/continue_game() 驱动。
## 状态机 + 每帧回路(spec §6)。宿主暂停 = 不调 step_frame(世界时间线零帧)。
## 表现契约 v2(2026-09-07):`_after_step` 顺序 = 请求 → 事件 → vanished → 特效 tick →
## HUD → 自机/木偶;请求 id 与事件 kind 一律取 WorldBridge.REQ_*/EVT_* 常量(不手抄)。

## 时间机制内核刀(2026-09-07,spec §5):REWINDING = 遡行倒放表现态(世界已在落点,壳逐帧
## `view_ring` 往回读到落点再恢复 PLAYING);観測/跳躍两段按键协议见 `_on_observe_key`。
enum S { PLAYING, PAUSED, STOPPED, REWINDING }
enum Mode { NORMAL, PRACTICE, REPLAY }

signal finished(reason: String) # "result" / "practice" / "playback" / "title"


## 开局参数(此前是 `new_game_at(names, sources, 1, 2, start, 0, 0, 3, 3)` 里一串位置魔数——
## demo 一直在跑 Hard 而调用点看不出来)。本刀只让它们可见可改,取值一律维持原样。
##
## 难度档取 `WorldBridge.RANK_*`(转自 stg-core `consts.rs` ①段,值域 `0..=4` 冻结;越界
## `new_game_at` 返 Err → false)。**不在本文件手抄一份**:抄来的镜像与 core 无编译期押运。
## Extra(4) 是预留位不是第五档——Extra 关在现代作品里走自己的脚本,通常不靠 rank 分支。
const BOOT_SEED := 1
const BOOT_RANK := WorldBridge.RANK_HARD
const BOOT_CHARACTER := 0
const BOOT_POWER := 0
const BOOT_LIVES := 3
const BOOT_BOMBS := 3

var state: int = S.PLAYING
var mode: int = Mode.NORMAL
var rank: int = BOOT_RANK
var start_mark: int = 0
var lives: int = BOOT_LIVES
var _units_names := PackedStringArray()   # 空 = 读 res://ecl/game
var _units_sources := PackedStringArray()
var _replay_bytes := PackedByteArray()
var _inject_continue := false
var overlay: Overlay
var bridge: WorldBridge
var stg_input: StgInput
var playfield: Playfield
var dispatcher: Dispatcher
var hud: Hud
var effects: Effects
var smoke := false
var _smoke_saw_bgm := false # 冒烟②侦听 REQ_BGM 用(成员变量,lambda 捕获值类型局部不回写)
## `--shots` 有头目验模式(表现契约 v2 DoD §7.2 第 6 条):脚本化输入(常按射击,第 200 帧放
## bomb)跑 demo,在 SHOT_FRAMES 各帧把 SubViewport 存成 PNG 到 $STG_SHOTS_DIR,最后一张后退出。
## 需要真渲染器(本机走 VNC 桌面 + llvmpipe:`DISPLAY=:2 LIBGL_ALWAYS_SOFTWARE=1 godot
## --rendering-driver opengl3 --path godot -- --shots`)。顺带打印各层 visible_instances
## (B26 ②:headless 恒 0,有头才有真值)。
var shots_mode := false
## 帧 60..75 向左走到 x≈-48 的杂兵正下方(自机每帧约 3px),之后自机弹持续命中:受击闪白/火花
## 可在 95/100 帧看到,杂兵 hp=40 打死后有爆炸环(REQ_ENEMY_DEATH)。第 200 帧放 bomb。
const SHOT_FRAMES := { 95: "hit_a", 100: "hit_b", 130: "zako", 201: "bomb_t1", 206: "bomb_t6", 214: "bomb_t14", 320: "observe", 361: "jump", 620: "boss" }
const SHOT_BOMB_FRAME := 200
const SHOT_LEFT_FRAMES := [60, 76]
var _shots_left := 0
var _death_shot_at := -1 # 首次 REQ_ENEMY_DEATH 后第 4 帧补一张(爆炸环 20 帧寿命的前四分之一)

## ── 時環譜 时间机制(spec §5)────────────────────────────────────────────────
## 観測:按一下 C 进入,窗口 OBSERVE_WINDOW tick 内每 tick 让影子世界跑 JUMP_FRAMES 步并
## 显示影子层;窗口内再按一下 C = 跳躍(把 BTN_JUMP 注入**这一帧**的输入,一帧即沿);
## 窗口到期自动退出。跳躍期间/倒放期间按 C 忽略。
const OBSERVE_WINDOW := 60
const SCRUB_STEP := 3 # 遡行倒放每 tick 退几帧
var observing := false
var _observe_left := 0
var _inject_jump := false
var _scrub_cur := 0
var _scrub_to := 0
var _last_rewind_to := -1 # 冒烟/目验用:最近一次遡行落点
## 目验:第 SHOT_OBSERVE_FRAME 帧按 C(観測),第 SHOT_JUMP_FRAME 帧再按 C(跳躍);
## 跳躍那一 tick 世界一口气走 31 帧,截图帧号取 SHOT_JUMP_FRAME+31。
const SHOT_OBSERVE_FRAME := 300
const SHOT_JUMP_FRAME := 330
const SHOT_FRAME_CAP := 4000 # 目验兜底:固定清单拍完前若到此帧仍未退,强制退

func _ready() -> void:
	smoke = "--smoke" in OS.get_cmdline_user_args()
	shots_mode = "--shots" in OS.get_cmdline_user_args()
	_shots_left = SHOT_FRAMES.size()
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
	overlay = Overlay.new()
	add_child(overlay)
	_wire_requests()
	state = S.STOPPED # 未开局:等 GameFlow 调 start()

## 分类见 dispatcher.gd(即发即忘 / 须确认 / 电平镜像);这里只接处理器。
## 坐标载荷是 Q16.16 raw,`/ 65536.0` 换回浮点世界坐标(render-contract §6 第五处)。
func _wire_requests() -> void:
	dispatcher.register(WorldBridge.REQ_ENEMY_DEATH, func(a):
		effects.explosion(Vector2(a[0] / 65536.0, a[1] / 65536.0), int(a[3]), bridge.frame())
		if shots_mode and _death_shot_at < 0:
			_death_shot_at = bridge.frame() + 4)
	dispatcher.register(WorldBridge.REQ_FX_AT, func(a):
		effects.spawn(int(a[2]), a[0] / 65536.0, a[1] / 65536.0, bridge.frame(), int(a[3])))
	dispatcher.register(WorldBridge.REQ_FX_ATTACHED, func(a):
		effects.spawn_attached(bridge, int(a[2]), int(a[0]), int(a[1]), int(a[3])))
	dispatcher.register(WorldBridge.REQ_SPELL_DECLARE, func(a):
		hud.show_banner(ContentTables.SPELL_NAMES.get(int(a[0]), "Spell #%d" % int(a[0])), 2.5))
	dispatcher.register(WorldBridge.REQ_SPELL_RESULT, func(a):
		hud.show_banner("取得!" if int(a[1]) == 1 else "失敗…", 2.0))
	dispatcher.register(WorldBridge.REQ_BGM, func(a):
		hud.set_bgm_label(ContentTables.BGM_NAMES.get(int(a[0]), "BGM #%d" % int(a[0]))))
	dispatcher.register(WorldBridge.REQ_BG, func(a): playfield.bg.set_bg(int(a[0])))
	dispatcher.register(WorldBridge.REQ_BG_PHASE, func(a): playfield.bg.set_phase(int(a[0])))

## 显隐(GameFlow 切页用):Play 本身是 Node,显隐落在 playfield/hud/overlay 三个可见子节点上。
func set_shown(on: bool) -> void:
	playfield.visible = on
	hud.visible = on
	if not on:
		overlay.hide_page()

## 开局入口(GameFlow 调):三种模式一个口。`names`/`sources` 非空则用它们(冒烟内联脚本),
## 否则整取 res://ecl/game/*.ecl(按名排序)。REPLAY 模式用 `replay_bytes` 走 new_game_from_replay。
func start(p_mode: int, p_rank: int, p_mark: int, p_lives: int = BOOT_LIVES,
		names := PackedStringArray(), sources := PackedStringArray(),
		replay_bytes := PackedByteArray()) -> bool:
	mode = p_mode
	rank = p_rank
	start_mark = p_mark
	lives = p_lives
	_units_names = names
	_units_sources = sources
	_replay_bytes = replay_bytes
	return _boot()

func _load_units() -> bool:
	if not _units_names.is_empty():
		return true
	var dir := DirAccess.open("res://ecl/game")
	if dir == null:
		push_error("[stg] res://ecl/game 目录读取失败")
		return false
	var files: Array[String] = []
	for f in dir.get_files():
		if f.ends_with(".ecl"):
			files.append(f)
	files.sort()
	_units_names = PackedStringArray()
	_units_sources = PackedStringArray()
	for f in files:
		_units_names.append(f)
		_units_sources.append(FileAccess.get_file_as_string("res://ecl/game/" + f))
	return true

func _boot() -> bool:
	if not _load_units():
		return false
	var ok: bool
	if mode == Mode.REPLAY:
		ok = bridge.new_game_from_replay(_units_names, _units_sources, _replay_bytes)
	else:
		# 参数序:names, sources, seed, rank, start, character, power, lives, bombs
		ok = bridge.new_game_at(_units_names, _units_sources, BOOT_SEED, rank, start_mark,
			BOOT_CHARACTER, BOOT_POWER, lives, BOOT_BOMBS)
	if ok:
		# 新世界 = 帧号从 0 起:水位、特效行、木偶记忆全部归零(表现契约 v2 §5.2/§5.4)
		dispatcher.reset()
		effects.clear_all()
		_end_observe()
		_inject_continue = false
		overlay.hide_page()
		hud.hide_banner()
		_sync_anchors() # 双表示规矩:开机后一次性对电平(T5 实装演出)
		if not playfield.setup(bridge):
			return false
		playfield.update_view(bridge, 0) # 首帧闪位修:免自机在 (0,0) 停一帧才对上真位置
		state = S.PLAYING
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
	if state == S.REWINDING:
		_scrub_tick()
		return
	if state != S.PLAYING:
		return
	if mode == Mode.REPLAY:
		_playback_tick()
		return
	var buttons := _scripted_mask(bridge.frame()) if shots_mode else stg_input.mask() # 算一次,step_frame/update_view 共用(避免帧内读两次输入分叉)
	if _inject_continue:
		buttons |= WorldBridge.BTN_CONTINUE
		_inject_continue = false
	# 観測/跳躍两段协议:真人局读 C 键上升沿;目验按脚本帧号;冒烟由 _run_smoke 直接调 _on_observe_key
	if shots_mode:
		if bridge.frame() == SHOT_OBSERVE_FRAME or bridge.frame() == SHOT_JUMP_FRAME:
			_on_observe_key()
	elif not smoke and stg_input.observe_pressed():
		_on_observe_key()
	if _inject_jump:
		buttons |= WorldBridge.BTN_JUMP
		_inject_jump = false
	var g0 := bridge.frame()
	var landed := bridge.step_frame(buttons)
	if landed >= 0:
		_begin_rewind(g0 + 1, landed)
		return
	# 跳躍快进(spec §3.2/§5):缺席的那 N 帧在同一 tick 里走完,中间帧的通道 B 请求与
	# vanished 丢弃——她不在场。上限 JUMP_FRAMES+1 防状态机异常时死循环。
	var n := 0
	while int(bridge.hud_player().get("life_state", 0)) == WorldBridge.LIFE_JUMPING and n <= WorldBridge.JUMP_FRAMES:
		landed = bridge.step_frame(0)
		n += 1
		if landed >= 0: # 缺席不可能被弹,防御性处理
			_begin_rewind(g0 + 1 + n, landed)
			return
	_after_step(buttons)
	if observing:
		_observe_tick()
	if shots_mode and SHOT_FRAMES.has(bridge.frame()):
		_capture(SHOT_FRAMES[bridge.frame()])
	if shots_mode and _death_shot_at == bridge.frame():
		_shots_left += 1
		_capture("death")
	if shots_mode and bridge.frame() >= SHOT_FRAME_CAP:
		print("[shots] frame cap %d reached without stage clear" % SHOT_FRAME_CAP)
		get_tree().quit(0)

## 回放播放的一 tick:输入来自日志;跳躍同样同 tick 快进;录制时的遡行落地只提示不倒放;
## 播完盖「播完」页。GAME OVER / 关底在播放里都不停拍(录制时的停拍贡献零帧)。
func _playback_tick() -> void:
	var r := bridge.playback_step()
	if r == -2:
		_on_playback_done()
		return
	var n := 0
	while r != -2 and int(bridge.hud_player().get("life_state", 0)) == WorldBridge.LIFE_JUMPING and n <= WorldBridge.JUMP_FRAMES:
		r = bridge.playback_step()
		n += 1
	if r >= 0:
		hud.show_banner("遡行", 0.5)
	_after_step(0)

func _on_playback_done() -> void:
	state = S.STOPPED
	var p := bridge.hud_player()
	overlay.show_page(Overlay.Kind.PLAYBACK_DONE, "REPLAY END", [
		"Score  %d" % int(p.get("score", 0)),
		"Frames %d" % bridge.frame(),
	], "Z / X  回标题")

## C 键:未観測 → 进入(只在自机 ALIVE 时);観測中 → 跳躍(注入一帧 BTN_JUMP)并退出観測。
func _on_observe_key() -> void:
	if observing:
		_inject_jump = true
		_end_observe()
	elif int(bridge.hud_player().get("life_state", 0)) == 1: # LIFE_ALIVE
		observing = true
		_observe_left = OBSERVE_WINDOW
		playfield.set_ghost_visible(true)
		bridge.preview(WorldBridge.JUMP_FRAMES)

func _observe_tick() -> void:
	_observe_left -= 1
	if _observe_left <= 0:
		_end_observe()
	else:
		bridge.preview(WorldBridge.JUMP_FRAMES)

func _end_observe() -> void:
	observing = false
	_observe_left = 0
	if playfield != null and playfield.ghost != null:
		playfield.set_ghost_visible(false)

## 遡行落地后的倒放(spec §5):世界已在落点 F,壳从请求帧 G 开始每 tick 退 SCRUB_STEP 帧
## `view_ring` 到 F,期间不 step、不收输入;到 F 后水位重置、fx 清空、锚点对表,回 PLAYING。
func _begin_rewind(g: int, f: int) -> void:
	_end_observe()
	state = S.REWINDING
	_scrub_cur = g
	_scrub_to = f
	_last_rewind_to = f
	hud.set_time_hint("遡行 …")
	_scrub_tick()

func _scrub_tick() -> void:
	_scrub_cur = maxi(_scrub_cur - SCRUB_STEP, _scrub_to)
	if not bridge.view_ring(_scrub_cur):
		_scrub_cur = _scrub_to # 环里没有(被覆写/越界):直接落地
		bridge.view_ring(_scrub_cur)
	playfield.update_view(bridge, 0)
	hud.refresh(bridge)
	if _scrub_cur <= _scrub_to:
		_finish_rewind()

func _finish_rewind() -> void:
	dispatcher.reset_to(_scrub_to)
	effects.clear_after(_scrub_to) # F20:只杀落点之后出生的行,落点前开始的爆炸环继续活
	_sync_anchors() # A7 记的那个"读档后必须对表"场合:遡行落地就是一次读档
	state = S.PLAYING
	_update_time_hint()
	if shots_mode:
		_shots_left += 1
		_capture("rewind_land")

## 时间机制的一行电平提示。
func _update_time_hint() -> void:
	var st := int(bridge.hud_player().get("life_state", 0))
	if observing:
		hud.set_time_hint("観測 %d  (C 跳躍)" % _observe_left)
	elif st == WorldBridge.LIFE_JUMPING:
		hud.set_time_hint("跳躍")
	elif st == WorldBridge.LIFE_DEATHWINDOW:
		hud.set_time_hint("V 遡行")
	else:
		hud.set_time_hint("")

func _after_step(buttons: int) -> void:
	# M3 前确认地平线 = 当前帧(须确认类立即播);接回滚时只改这个实参
	dispatcher.drain(bridge.take_requests(), bridge.frame())
	_drain_events()
	effects.fade_batch(bridge.vanished(), bridge.frame())
	effects.tick(bridge)
	hud.refresh(bridge)
	playfield.update_view(bridge, buttons)
	_update_time_hint()
	# I-1(终审裁定):残机耗尽的最小处置——拦假胜利。life_state==4 = LIFE_GAMEOVER
	# (stg-core player.rs),hud_player 已暴露该键。复用既有三态与 Z 重开路径,不新增状态;
	# continue/计分对齐等深度流程留内容期(follow-ups A8)。
	if state == S.PLAYING and mode != Mode.REPLAY \
			and int(bridge.hud_player().get("life_state", 0)) == WorldBridge.LIFE_GAMEOVER:
		_on_game_over()

## 通道 A 的批量事实流(每帧,下次 step 前必须取走)。与 `take_requests` 的分工:
## 请求 = 脚本/引擎主动发的离散演出指令;事件 = 世界产出的事实,表现层跟着做反应。
## 目前只消费命中火花;敌死/拾取/符卡等其余 kind 仍走各自的请求或 HUD 路径。
func _drain_events() -> void:
	var frame := bridge.frame()
	for ev in bridge.frame_events():
		match int(ev.get("kind", 0)):
			WorldBridge.EVT_SHOT_HIT_ENEMY:
				effects.hit_spark(Vector2(ev.get("x", 0.0), ev.get("y", 0.0)), frame)
			WorldBridge.EVT_STAGE_CLEARED:
				# 流程信号走通道 A 事实流(壳子刀 2026-09-07):脚本 stage_clear(n) 发事件并让出一帧,
				# 宿主停拍;下一关第一帧要等玩家确认(现在 = Z 重开,结算页是壳子刀正文)。
				_on_stage_clear(int(ev.get("data0", 0)))

## 目验模式的脚本化输入:常按射击;第 SHOT_BOMB_FRAME 帧按一帧 bomb(沿检测,按一帧即触发)。
func _scripted_mask(frame: int) -> int:
	var m := WorldBridge.BTN_SHOT
	if frame == SHOT_BOMB_FRAME:
		m |= WorldBridge.BTN_BOMB
	if frame >= SHOT_LEFT_FRAMES[0] and frame < SHOT_LEFT_FRAMES[1]:
		m |= WorldBridge.BTN_LEFT
	return m

func _capture(name: String) -> void:
	var dir := OS.get_environment("STG_SHOTS_DIR")
	if dir.is_empty():
		dir = "user://shots"
	await RenderingServer.frame_post_draw
	var img := playfield.viewport.get_texture().get_image()
	if _shots_left == SHOT_FRAMES.size():
		# 首张顺带 dump 引擎内实际加载的弹图集(核对导入缓存是否与磁盘 PNG 一致)
		var atlas: Texture2D = load(Playfield.TEXTURES[0])
		var aimg := atlas.get_image()
		if aimg != null:
			aimg.save_png(dir.path_join("atlas_as_loaded.png"))
			print("[shots] atlas as loaded: %dx%d fmt=%d mip=%s" % [aimg.get_width(), aimg.get_height(), aimg.get_format(), str(aimg.has_mipmaps())])
	var path := dir.path_join("%s_f%d.png" % [name, bridge.frame()])
	DirAccess.make_dir_recursive_absolute(dir)
	var err := img.save_png(path)
	var vis := []
	for kind in playfield.layer_nodes:
		vis.append([kind, RenderingServer.multimesh_get_visible_instances(playfield.layer_nodes[kind].multimesh.get_rid())])
	vis.append(["ghost", RenderingServer.multimesh_get_visible_instances(playfield.ghost.multimesh.get_rid()), playfield.ghost.visible])
	print("[shots] %s → %s (err %d) visible_instances=%s fx_rows=%d" % [name, path, err, str(vis), effects.n])
	_shots_left -= 1
	if _shots_left <= 0:
		get_tree().quit(0)

func _on_stage_clear(stage: int) -> void:
	if mode == Mode.REPLAY:
		hud.show_banner("STAGE CLEAR" if stage != 0 else "ALL CLEAR", 2.0)
		return # 录制时的停拍贡献零帧,播放照走
	bridge.seal_history() # 关底 = 遡行硬边界:恢复后被弹不能退回上一关
	state = S.STOPPED
	var p := bridge.hud_player()
	if mode == Mode.PRACTICE:
		overlay.show_page(Overlay.Kind.PRACTICE_DONE, "PRACTICE END", [
			"Score  %d" % int(p.get("score", 0)),
			"Graze  %d" % int(p.get("graze", 0)),
		], "Z / X  回练习菜单")
	elif stage == 0:
		var cont := int(p.get("continues", 0))
		var lines := [
			"Score      %d" % int(p.get("score", 0)),
			"Continues  %d" % cont,
		]
		if cont == 0:
			lines.append("EXTRA STAGE UNLOCKED")
		overlay.show_page(Overlay.Kind.RESULT, "ALL CLEAR", lines, "Z  回标题    C  存回放")
	else:
		overlay.show_page(Overlay.Kind.STAGE_CLEAR, "STAGE %d CLEAR" % stage, [
			"Score      %d" % int(p.get("score", 0)),
			"Graze      %d" % int(p.get("graze", 0)),
			"Continues  %d" % int(p.get("continues", 0)),
		], "Z  继续")
	if shots_mode:
		_shots_left += 1
		_capture_root("stage_clear_page")

func _on_game_over() -> void:
	state = S.STOPPED
	var p := bridge.hud_player()
	overlay.show_page(Overlay.Kind.GAME_OVER, "GAME OVER", [
		"Score      %d" % int(p.get("score", 0)),
		"Continues  %d" % int(p.get("continues", 0)),
	], "Z  续关    X  回标题    C  存回放")

## 覆盖层上的确认键(Z):按页面种类分派。GameFlow 与冒烟都走这里。
func confirm() -> void:
	if state != S.STOPPED:
		return
	match overlay.kind:
		Overlay.Kind.STAGE_CLEAR:
			overlay.hide_page()
			state = S.PLAYING
		Overlay.Kind.GAME_OVER:
			continue_game()
		Overlay.Kind.RESULT:
			finished.emit("result")
		Overlay.Kind.PRACTICE_DONE:
			finished.emit("practice")
		Overlay.Kind.PLAYBACK_DONE:
			finished.emit("playback")

## 覆盖层上的返回键(X):回标题(结算页上无效——关底不能弃局)。
func back() -> void:
	if state != S.STOPPED:
		return
	match overlay.kind:
		Overlay.Kind.GAME_OVER, Overlay.Kind.RESULT, Overlay.Kind.PLAYBACK_DONE, Overlay.Kind.PRACTICE_DONE:
			finished.emit("title")

## 续关:注入一帧 BTN_CONTINUE(世界内做续关,可回放),恢复 step。播放态不可续关。
func continue_game() -> void:
	if mode == Mode.REPLAY or state != S.STOPPED:
		return
	overlay.hide_page()
	_inject_continue = true
	state = S.PLAYING

## 存回放到 user://replays/<unix>_r<rank>.stgr;返回路径(失败返空串)。播放态不存。
func save_replay() -> String:
	if mode == Mode.REPLAY:
		return ""
	var bytes := bridge.replay_bytes()
	if bytes.is_empty():
		return ""
	DirAccess.make_dir_recursive_absolute("user://replays")
	var path := "user://replays/%d_r%d.stgr" % [int(Time.get_unix_time_from_system()), rank]
	var f := FileAccess.open(path, FileAccess.WRITE)
	if f == null:
		push_error("[stg] 存回放失败: %s" % path)
		return ""
	f.store_buffer(bytes)
	f.close()
	hud.show_banner("REPLAY SAVED", 1.5)
	return path

## 根视口截图(覆盖层在 SubViewport 之外,`_capture` 拍不到)。
func _capture_root(name: String) -> void:
	var dir := OS.get_environment("STG_SHOTS_DIR")
	if dir.is_empty():
		dir = "user://shots"
	await RenderingServer.frame_post_draw
	var img := get_viewport().get_texture().get_image()
	var path := dir.path_join("%s_f%d.png" % [name, bridge.frame()])
	DirAccess.make_dir_recursive_absolute(dir)
	print("[shots] %s → %s (err %d)" % [name, path, img.save_png(path)])
	_shots_left -= 1
	get_tree().quit(0)

func _unhandled_input(ev: InputEvent) -> void:
	if ev.is_action_pressed("ui_cancel"):
		if state == S.PLAYING:
			state = S.PAUSED
		elif state == S.PAUSED:
			state = S.PLAYING
	elif state == S.STOPPED:
		if ev.is_action_pressed("stg_shot"):
			confirm()
		elif ev.is_action_pressed("stg_bomb"):
			back()
		elif ev.is_action_pressed("stg_observe"):
			if overlay.kind == Overlay.Kind.RESULT or overlay.kind == Overlay.Kind.GAME_OVER:
				save_replay()

## ── 冒烟(T6:demo 两次开机——①正常开局杂兵段真实全链路 ②中段开机垫片补偿)─────────
## 等帧用轮询实际读口(`bridge.frame()`)而非数 `physics_frame` 信号触发次数:实测诊断
## (非简报预判)`physics_frame` 信号在每 tick 的 `_physics_process` **之前**触发(Godot 4.6
## 实况),数信号次数会差一帧,不代表 step_frame 真的"每两 tick 一 step"。轮次上限给
## 目标帧数 2× 余量、并断言实际轮次 ≤ 目标+1——上限防信号真不来时死等,后一条断言防
## "每两 tick 一 step"这类回归被 2× 宽上限悄悄放过(T3 审阅遗留)。
func run_smoke() -> int:
	var fails := 0
	# ① start=0 正常开局:杂兵段跑 240 帧,全链路(编码/分发/HUD)真实走
	if not start(Mode.NORMAL, BOOT_RANK, 0):
		print("SMOKE FAIL: boot(0)")
		return 1
	_smoke_saw_bgm = false
	# 覆盖注册以侦听,链式调回 `_wire_requests` 注册的原 hud 处理器——冒烟不该绕开
	# `hud.set_bgm_label` 那条真实路径(纯加侦听,不替换行为)。
	var orig_bgm_handler: Callable = dispatcher.handlers.get(WorldBridge.REQ_BGM, Callable())
	dispatcher.register(WorldBridge.REQ_BGM, func(a):
		_smoke_saw_bgm = true
		if orig_bgm_handler.is_valid():
			orig_bgm_handler.call(a))
	# I-1(复审裁定):原四断言(frame/checksum/REQ_BGM/player 在场界)对 demo 内容零判别——
	# 即使 stage1 被清空、main 只剩 `bgm(1); loop { wait(600); }`,四条也照绿。表现契约 v2
	# 起敌层退役,改逐帧扫 puppets() 木偶喂料:窗口内出现过非 (0,0) 位置的敌人,证明 stage1
	# 真出过杂兵、真的在动;同时要求对应木偶节点真的可见(壳侧喂料链真接上了)。
	# 选这条而不是侦听 `REQ_ENEMY_DEATH`(复审给的备选②)——headless 下
	# `Input.is_action_pressed` 恒 false,`stg_input.mask()` 每帧恒 0,冒烟一个键都不按
	# (`BTN_LEFT` 是桥级 `crates/stg-godot/smoke/smoke.gd` 自己在 `step_frame` 调用点
	# 显式传的常量,不是这里),`char0_update_shot` 要 `BTN_SHOT` 才发弹,240 帧窗口内
	# 自机打不死杂兵,②在当前冒烟输入下不可达。
	var enemy_seen := false
	var puppet_visible_seen := false
	var waited := 0
	# 时间机制内核刀:第 100 帧按 C(観測)→ 下一 tick 影子层可见;第 130 帧再按 C(跳躍)
	# → 那一 tick 世界一口气走 JUMP_FRAMES+1 帧(缺席快进),影子层随之关闭。
	var ghost_seen := false
	var jump_from := -1
	var jump_ok := false
	while bridge.frame() < 240 and waited < 480:
		await get_tree().physics_frame
		waited += 1
		if jump_from >= 0 and not jump_ok:
			jump_ok = bridge.frame() == jump_from + WorldBridge.JUMP_FRAMES + 1
			jump_from = -1
		if observing and playfield.ghost.visible:
			ghost_seen = true
		if bridge.frame() == 100:
			_on_observe_key()
		elif bridge.frame() == 130 and observing:
			jump_from = bridge.frame()
			_on_observe_key()
		var pp: Dictionary = bridge.puppets()
		if not pp.is_empty() and pp["index"].size() > 0:
			if absf(pp["x"][0]) > 0.01 or absf(pp["y"][0]) > 0.01:
				enemy_seen = true
			if playfield.puppets[pp["index"][0]].visible:
				puppet_visible_seen = true
	fails += _chk(bridge.frame() >= 240, "frame>=240, got %d" % bridge.frame())
	fails += _chk(waited <= 241 - WorldBridge.JUMP_FRAMES, "轮次<=目标+1−跳过帧数(防每两 tick 一 step 回归),got %d" % waited)
	fails += _chk(ghost_seen, "観測期间影子层应可见(两段协议第一下)")
	fails += _chk(jump_ok, "跳躍那一 tick 应快进 JUMP_FRAMES+1 帧(缺席快进)")
	fails += _chk(not observing and not playfield.ghost.visible, "跳躍后観測应已退出")
	# 倒放读口往返:视图切到 5 帧前不动权威帧号,下一 tick 自动切回并 +1。
	var f_now := bridge.frame()
	fails += _chk(bridge.view_ring(f_now - 5), "view_ring(5 帧前) 应在环里")
	fails += _chk(bridge.frame() == f_now, "view_ring 不动权威帧号")
	await get_tree().physics_frame
	fails += _chk(bridge.frame() == f_now + 1, "视图态在下一次 step 后解除")
	fails += _chk(enemy_seen, "puppets() 窗口内出现非默认位置的敌人(demo 杂兵真的在动)")
	fails += _chk(puppet_visible_seen, "木偶节点随 puppets() 变为可见(壳侧喂料链接上)")
	fails += _chk(bridge.checksum() != 0, "checksum!=0")
	fails += _chk(_smoke_saw_bgm, "REQ_BGM 应到达分发器")
	var pp0: Vector2 = bridge.player_pos()
	fails += _chk(pp0.x >= -192.0 and pp0.x <= 192.0 and pp0.y >= 0.0 and pp0.y <= 448.0, "player 在场界")
	# ② start=15 中段开机(第 1 关 boss 段 mark):垫片补偿电平追平(bgm 块内手写 2/bg 注入 1/bg_phase 手写 1)
	if not start(Mode.NORMAL, BOOT_RANK, 15):
		print("SMOKE FAIL: boot(start=15)")
		return fails + 1
	waited = await _wait_frame(10, 20)
	fails += _chk(waited <= 11, "轮次<=目标+1(防每两 tick 一 step 回归),got %d" % waited)
	var a := bridge.anchors()
	fails += _chk(int(a.get("bgm", -1)) == 2, "mid-start bgm==2, got %s" % str(a.get("bgm")))
	fails += _chk(int(a.get("bg", -1)) == 1, "mid-start bg==1")
	fails += _chk(int(a.get("bg_phase", -1)) == 1, "mid-start bg_phase==1")
	# I-2(终审裁定):补招牌断言——boss_main 是 enemy-owned 任务(A5 乙案),中段开机直跳
	# boss_battle() 后应立刻喂到 hud_boss/落池,不是只对表锚点四字段。
	fails += _chk(int(bridge.hud_boss(0).get("active", 0)) == 1, "boss_main 的 boss_set 应已喂到 hud_boss")
	var pb: Dictionary = bridge.puppets()
	var boss_row_ok: bool = not pb.is_empty() and pb["sprite"].size() >= 1 and int(pb["sprite"][0]) == 1 \
		and int(pb["state_age"][0]) >= 1
	fails += _chk(boss_row_ok, "puppets() 首行 sprite==1 且 state_age>=1(boss sprite/anm 两位真落池)")
	return fails

## 轮询等到 `bridge.frame() >= target`(或轮次耗尽);返回实际等待轮数供调用方核验节奏。
func _wait_frame(target: int, cap: int) -> int:
	var waited := 0
	while bridge.frame() < target and waited < cap:
		await get_tree().physics_frame
		waited += 1
	return waited

static func _chk(cond: bool, msg: String) -> int:
	if not cond:
		print("SMOKE FAIL: ", msg)
		return 1
	return 0
