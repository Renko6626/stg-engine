extends SceneTree

func fail(msg: String):
	push_error("SMOKE FAIL: " + msg)
	quit(1)

func _init():
	if not ClassDB.class_exists("WorldBridge"):
		fail("WorldBridge 未注册"); return
	var b = ClassDB.instantiate("WorldBridge")
	if b.ping() != 42: fail("ping"); return
	# 未开局违约路径:no-op 不炸
	b.step_frame(0)
	if b.frame() != -1: fail("未开局 frame 应为 -1"); return
	if b.anchors().size() != 0: fail("未开局 anchors 应为空字典(同 hud_* 口径)"); return
	var src := FileAccess.get_file_as_string("res://godot_smoke.ecl")
	if not b.new_game(src, 7, 2): fail("new_game"); return
	if b.frame() != 0: fail("frame0"); return

	# register_layer 三路(task-4:B18 可达面——坏 kind/坏尺寸/合法注册全覆盖,用 LAYER_ENEMIES
	# 因为 godot_smoke.ecl 会出一只敌;cap=EnemyPool::CAP=256,播种 256×12=3072 浮点很便宜)。
	# 坏 kind:层号越界(LAYER_COUNT==4,99 显然越界),no-op 直接返 false。
	var rid_bad_kind := RenderingServer.multimesh_create()
	if b.register_layer(99, rid_bad_kind): fail("rl bad kind"); return
	RenderingServer.free_rid(rid_bad_kind)
	# 坏尺寸:播种 100×12 浮点 ≠ 需要的 256×12,register_layer 的缓冲尺寸校验拒绝(bridge.rs
	# register_layer 判据:got==cap×12 才收)。
	var bad := RenderingServer.multimesh_create()
	var seed_bad := PackedFloat32Array(); seed_bad.resize(100 * 12)
	RenderingServer.multimesh_set_buffer(bad, seed_bad)
	if b.register_layer(b.LAYER_ENEMIES, bad): fail("rl bad size"); return
	RenderingServer.free_rid(bad)
	# 合法路径:allocate_data 走生产同款调用,再 set_buffer 播种一次定长零缓冲——headless
	# dummy renderer 下 get_buffer 只在 set_buffer 之后才有完整往返(已验证实验事实),不播种
	# 这条会被坏尺寸判据一并拒收。
	var mm := RenderingServer.multimesh_create()
	RenderingServer.multimesh_allocate_data(mm, 256, RenderingServer.MULTIMESH_TRANSFORM_2D, false, true)
	var seed_ok := PackedFloat32Array(); seed_ok.resize(256 * 12)
	RenderingServer.multimesh_set_buffer(mm, seed_ok)
	if not b.register_layer(b.LAYER_ENEMIES, mm): fail("rl ok path"); return

	var reqs_seen := 0
	for i in range(120):
		b.step_frame(0)
		reqs_seen += b.take_requests().size()
	if b.frame() != 120: fail("frame120"); return
	if reqs_seen == 0: fail("通道 B 零请求——emit_req 没到达"); return

	# 编码→上传链回读判别(task-4):`register_layer` 在上面的 120 步循环之前就注册了,故
	# 每一步 `step_frame` 都会编码+上传;godot_smoke.ecl 的敌 `spawn_enemy(0,-160,...)` 全程
	# 无 move_to/xform,vx=vy=0/mv_active=0(world.rs spawn_enemy 语法糖)——120 步后仍静止
	# 在出生点。布局 12 float/实例:[xx,yx,0,ox, xy,yy,0,oy, custom×4](frame.rs
	# write_instance);无旋转 → cos=1/sin=0,故 xx=1.0、ox=0.0、oy=-160.0。
	# (multimesh_get_visible_instances 在 headless dummy renderer 下恒 0,已实验判决,
	# 不可测——这里改用 get_buffer 回读实数据判别,不断言可见数。)
	var back := RenderingServer.multimesh_get_buffer(mm)
	if absf(back[0] - 1.0) > 0.0001: fail("mm xx"); return       # 无旋转 cos=1,非零判别
	if absf(back[7] - (-160.0)) > 0.0001: fail("mm oy"); return  # 出生 y,非默认判别

	var c120 = b.checksum()
	if c120 == 0: fail("checksum 0"); return
	var sav = b.save_state()
	if sav.size() == 0: fail("save 空"); return
	for i in range(30): b.step_frame(WorldBridge.BTN_LEFT)
	var c150 = b.checksum()
	if c150 == c120: fail("步进未改变校验和"); return
	if not b.load_state(sav): fail("load"); return
	if b.checksum() != c120: fail("载入未回到 c120"); return
	for i in range(30): b.step_frame(WorldBridge.BTN_LEFT)
	if b.checksum() != c150: fail("恢复重演 ≠ 未离开(确定性破)"); return
	if b.hud_player().get("lives", -1) < 0: fail("hud_player"); return

	# hud_boss 判别(task-4):godot_smoke.ecl 在 mark(9) 之前顶层直写
	# `boss_set(0, 1.0fx, 7, 3600, 2, 1);`——正常路径(本 game,start=0 隐式)会直接执行到它
	# (main 首次真正运行是第 2 次 step_frame,frame==1 那一次,详见下方 anchors 推导),此后
	# 主循环只 fire/emit_req/wait,从不二次覆写、也未开任何符卡(符卡 active 期才会被引擎
	# 逐帧自动覆写 enemy/spell_id/timer_frames/active/hp_ratio)——故步进多轮之后读到的值
	# 应该还是 boss_set 写入时的原样,验证"设过就一直在"而非只在设置当帧可见。
	var hb: Dictionary = b.hud_boss(0)
	if hb.get("active", -1) != 1: fail("hud_boss active"); return
	if hb.get("spell_id", -1) != 7: fail("hud_boss spell_id"); return
	if hb.get("timer_frames", -1) != 3600: fail("hud_boss timer_frames"); return
	if hb.get("phase_left", -1) != 2: fail("hud_boss phase_left"); return
	if absf(hb.get("hp_ratio", 0.0) - 1.0) > 0.0001: fail("hud_boss hp_ratio"); return

	# player_pos 判别(task-4):自机出生点非零(PlayerState::spawn 硬编码 y=384,player.rs)。
	var p: Vector2 = b.player_pos()
	if absf(p.y - 384.0) > 0.0001: fail("player_pos y"); return

	var hp: Dictionary = b.hud_player()
	if hp.get("lives", -1) != 3: fail("hud_player lives"); return
	if hp.get("bombs", -1) != 3: fail("hud_player bombs"); return

	# anchors(正常路径,task-4):main 顶层依次 `bgm(3); bg(2); bg_phase(1); boss_set(...);
	# mark(9);`——本 game 走 start=0 隐式正常流,顺序执行到这四行,mark(9) 只是 `JMP after`
	# 跨过垫片(不重跑)。bg_phase_frame 推导:`set_bg_phase` 写 `bg_phase_frame = self.frame`
	# (world.rs)，而 `self.frame` 只在每次 `step_frame` 调用末尾的 `advance()`(相位10)才
	# 加一——main 的 born_frame 门禁令它第 1 次 step_frame 调用(frame 0→1)整体跳过不跑,
	# 第 2 次调用(frame 仍是 1,尚未 advance)才真正跑到 `bg_phase(1)`,此刻 `self.frame==1`。
	# 本 game 的"第 2 次 step_frame 调用"就是上面 120 步循环的第 2 次迭代(i==1),早已跑完,
	# 故此处 bg_phase_frame 应为 1。
	var anc: Dictionary = b.anchors()
	if anc["bgm"] != 3: fail("anchors bgm(正常路径)"); return
	if anc["bg"] != 2: fail("anchors bg(正常路径)"); return
	if anc["bg_phase"] != 1: fail("anchors bg_phase(正常路径)"); return
	if anc["bg_phase_frame"] != 1: fail("anchors bg_phase_frame(正常路径,推导见上)"); return

	# 中段启动 + 锚点读口冒烟(task-7 续 task-4 连锁):godot_smoke.ecl main 顶层依次是
	# `spawn_enemy(...); bgm(3); bg(2); bg_phase(1); boss_set(...); mark(9);`——start=9
	# 跳入落点直接执行 mark 自动补偿:编译器按顶层线性位扫描,记录到 bgm=3/bg=2/bg_phase=1
	# (bg_phase 声明位置严格晚于最近一次 bg 声明,补偿生效),按固定顺序 `bgm→bg→bg_phase`
	# 各插一条 sys 调用落在 landing 首(mark(9) 本身无作者手写补偿块)——三值因此与正常路径
	# 相同。`bg_phase_frame` 同上推导:landing 的 sys 调用同样发生在"第 2 次 step_frame"
	# (frame 仍是 1,尚未 advance),故也为 1。`spawn_enemy`/`boss_set` 均不在自动补偿覆盖
	# 名单内(boss_set 更不是三类锚点之一)——这条路径没有敌、hud_boss 仍是初值,故不在此处
	# 断言两者。复用既有 `src`(脚本内容不变)。
	var names := PackedStringArray(["smoke.ecl"])
	var srcs := PackedStringArray([src])
	if not b.new_game_at(names, srcs, 7, 2, 9, 0, 400, 3, 3): fail("new_game_at"); return
	if b.frame() != 0: fail("new_game_at 应从 frame 0 起"); return
	b.step_frame(0); b.step_frame(0)
	if b.frame() != 2: fail("new_game_at 后 step 计数错"); return
	var a: Dictionary = b.anchors()
	if a["bgm"] != 3: fail("anchors bgm 应为 mark(9) 垫片补偿值 3"); return
	if a["bg"] != 2: fail("anchors bg 应为 mark(9) 垫片补偿值 2(task-4 连锁:bg(2) 现声明于 mark 之前,自动补偿会注入)"); return
	if a["bg_phase"] != 1: fail("anchors bg_phase 应为 mark(9) 垫片补偿值 1(同上连锁)"); return
	if a["bg_phase_frame"] != 1: fail("anchors bg_phase_frame 应为 1(推导见上,mid-start 与正常路径同值)"); return
	if b.hud_player().get("power", -1) != 400: fail("new_game_at loadout power 未生效"); return
	RenderingServer.free_rid(mm)
	b.free()
	print("SMOKE OK")
	quit(0)
