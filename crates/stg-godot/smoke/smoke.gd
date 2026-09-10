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

	# register_layer 三路(task-4:B18 可达面——坏 kind/坏尺寸/合法注册全覆盖)。表现契约 v2
	# 起敌层退役(敌人走 puppets() 木偶喂料),改用 LAYER_BULLETS:godot_smoke.ecl 每 30 帧发一颗
	# 弹;cap=BulletPool::CAP=8192,播种 8192×12=98304 浮点。
	# 坏 kind:层号越界(LAYER_COUNT==3,99 显然越界),no-op 直接返 false。
	var rid_bad_kind := RenderingServer.multimesh_create()
	if b.register_layer(99, rid_bad_kind): fail("rl bad kind"); return
	RenderingServer.free_rid(rid_bad_kind)
	# 坏尺寸:播种 100×12 浮点 ≠ 需要的 8192×12,register_layer 的缓冲尺寸校验拒绝(bridge.rs
	# register_layer 判据:got==cap×12 才收)。
	var bad := RenderingServer.multimesh_create()
	var seed_bad := PackedFloat32Array(); seed_bad.resize(100 * 12)
	RenderingServer.multimesh_set_buffer(bad, seed_bad)
	if b.register_layer(b.LAYER_BULLETS, bad): fail("rl bad size"); return
	RenderingServer.free_rid(bad)
	# 合法路径:allocate_data 走生产同款调用,再 set_buffer 播种一次定长零缓冲——headless
	# dummy renderer 下 get_buffer 只在 set_buffer 之后才有完整往返(已验证实验事实),不播种
	# 这条会被坏尺寸判据一并拒收。
	var mm := RenderingServer.multimesh_create()
	RenderingServer.multimesh_allocate_data(mm, 8192, RenderingServer.MULTIMESH_TRANSFORM_2D, false, true)
	var seed_ok := PackedFloat32Array(); seed_ok.resize(8192 * 12)
	RenderingServer.multimesh_set_buffer(mm, seed_ok)
	if not b.register_layer(b.LAYER_BULLETS, mm): fail("rl ok path"); return

	var reqs_seen := 0
	# B18 余量:hud_spell 判别(active/spell_id=7/bonus>0/frames_left>0)+ fields_info 非空
	# 至少一帧(符卡清弹 field)。godot_smoke.ecl 的 smoke_boss 符卡挂槽 1(不是 0,避让
	# main() 顶层手写的 boss_ui 槽 0——见 .ecl 注释,task-4 hud_boss 断言读的就是槽 0),
	# time_limit=8(远小于原型 3600)——boss hp=8888 全程不受伤,HP 路径的 hp_threshold=0
	# 永不命中,只有超时路径会在早期几帧内自然结算;结算(settle_one_spell,spell.rs)才
	# 铺一帧清弹 field(life=1,只活一帧)。头 20 步(远宽于超时点,给 born-frame 调度余量)
	# 逐帧扫,先摸到 active 期的 hud_spell,再摸到结算那一帧的 field;把这 20 步的总数从
	# 下方主循环的 120 步预算里扣除(20+100=120),`frame120`/`reqs_seen` 两条既有断言分毫
	# 不动。
	var spell_seen := false
	var bonus_floor_seen := false
	var field_seen := false
	# 表现契约 v2:符卡结算铺的清弹 field 会把 frame 1 发的那颗场内弹(y=300)消掉——
	# 核内第四条纯输出缓冲 `vanished` 当帧必有一行 reason==VANISH_CLEARED、y==300。
	# 这是 vanished 读口在真桥面上的唯一可达判别(其余弹都是越界消失、越界不记)。
	var vanished_cleared_seen := false
	for i in range(20):
		b.step_frame(0)
		reqs_seen += b.take_requests().size()
		var vn0: Dictionary = b.vanished()
		for j in range(vn0["reason"].size()):
			if int(vn0["reason"][j]) == b.VANISH_CLEARED and absf(vn0["y"][j] - 300.0) < 0.0001:
				vanished_cleared_seen = true
		var s: Dictionary = b.hud_spell(1)
		if not s.is_empty() and int(s.get("active", 0)) == 1 and int(s.get("spell_id", 0)) == 7 \
				and int(s.get("bonus_now", 0)) > 0 and int(s.get("frames_left", 0)) > 0:
			spell_seen = true
		# 复审裁定:`bonus_now>0` 收紧成衰减公式的真判别式——`bonus_floor = bonus0/10 =
		# 50000/10 = 5000`(spell_begin 当帧一次整除定格,spell.rs)。取"窗口内出现过 5000"
		# 而非"与上面 active/frames_left>0 同帧同断言"这个更强形态:`dec_per_frame =
		# (bonus0-floor)/time_limit = 45000/8 = 5625` 整除无余数,bonus_now 与 frames_left
		# 每次 settle_spells 同步各减一步,数学上 bonus_now 首次摸到地板 5000 的那一次
		# settle 调用,恰好也是 frames_left 减到 0 的那一次——`frames_left>0` 在那一读
		# 恒假,两个条件在同一帧本就互斥,合并写会让这条断言永远假,故拆成独立标志。
		if not s.is_empty() and int(s.get("bonus_now", 0)) == 5000:
			bonus_floor_seen = true
		if b.fields_info().size() > 0:
			field_seen = true
	for i in range(100):
		b.step_frame(0)
		reqs_seen += b.take_requests().size()
	if b.frame() != 120: fail("frame120"); return
	if reqs_seen == 0: fail("通道 B 零请求——emit_req 没到达"); return
	if not spell_seen: fail("hud_spell 判别(active/spell_id=7/bonus>0/frames_left>0)"); return
	if not bonus_floor_seen: fail("hud_spell bonus_now 触底=bonus0/10=5000(衰减公式判别)"); return
	if not field_seen: fail("fields_info 非空(符卡清弹 field 可达)"); return
	if not vanished_cleared_seen: fail("vanished 应在清弹帧记下 y=300 那颗弹(reason=CLEARED)"); return

	# 编码→上传链回读判别(task-4,表现契约 v2 改弹层):`register_layer` 在上面的 120 步循环
	# 之前就注册了,故每一步 `step_frame` 都会编码+上传。godot_smoke.ecl 主循环每 30 帧在
	# (-150,300) 朝 0deg(+x)发一颗 speed 0.5 的场内弹(另一颗 y=-160 的出生即越界回收,
	# 见 .ecl 注释):frame 1/31/61/91 各一颗。**frame 1 那颗活不到 120**——槽 1 的符卡
	# time_limit=8,约 frame 10 超时结算铺一帧全屏清弹 field,把它消掉(上面 20 步循环里的
	# vanished 断言吃的正是这一行)。故 frame 120 时实例 0 = frame 31 那颗:
	# x = -150 + 0.5×89 = -105.5(raw 精确,f32 无舍入)、y 恒 300(vy 精确为 0)。
	# 布局 12 float/实例:[xx,yx,0,ox, xy,yy,0,oy, sprite,age,0,0](frame.rs write_instance);
	# 弹层旋转 = 速度方向 + 90°:BAM 0 → cos=0/sin=1 → xx=0、xy=1。
	# age = 120 − 31 = 89(custom.y,表现契约 v2 §4.2)。
	# (multimesh_get_visible_instances 在 headless dummy renderer 下恒 0,已实验判决,
	# 不可测——这里改用 get_buffer 回读实数据判别,不断言可见数。)
	var back := RenderingServer.multimesh_get_buffer(mm)
	print("[smoke] bullets instance0 = ", back.slice(0, 12), " instance1 = ", back.slice(12, 24))
	if absf(back[0]) > 0.0001: fail("mm xx(朝右飞的弹应转 90°:cos=0)"); return
	if absf(back[4] - 1.0) > 0.0001: fail("mm xy(sin=1)"); return
	if absf(back[3] - (-105.5)) > 0.0001: fail("mm ox 应为 -105.5,得 %f" % back[3]); return
	if absf(back[7] - 300.0) > 0.0001: fail("mm oy 应为 300,得 %f" % back[7]); return
	if absf(back[9] - 89.0) > 0.0001: fail("mm custom.y 弹龄应为 89,得 %f" % back[9]); return

	# puppets() 木偶喂料判别(表现契约 v2 §4.4):godot_smoke.ecl 三只敌里 boss(索引 1)约
	# 10 帧就自燃退场(见 .ecl 注释),frame 120 时只剩索引 0(y=-160)与 2(y=200)。压实序 =
	# 池索引升序;state_age = 120 − 出生帧 1 = 119;gen 首次分配为 1。
	var pp: Dictionary = b.puppets()
	for k in ["index", "gen", "x", "y", "sprite", "anm_state", "state_age", "hit_flash"]:
		if not pp.has(k): fail("puppets 缺列 " + k); return
	if pp["index"].size() != 2: fail("puppets 应两行(boss 已退场),得 %d" % pp["index"].size()); return
	if pp["index"][0] != 0 or pp["index"][1] != 2: fail("puppets 压实序应为 [0,2]"); return
	if absf(pp["y"][0] - (-160.0)) > 0.0001 or absf(pp["y"][1] - 200.0) > 0.0001: fail("puppets y"); return
	if pp["gen"][0] != 1: fail("puppets gen 首次分配应为 1"); return
	if pp["state_age"][0] != 119: fail("puppets state_age 应为 119,得 %d" % pp["state_age"][0]); return
	# entity_pos:活句柄 → Vector2;死 boss 的旧句柄(索引 1,gen 1)与坏 gen → null。
	var ep = b.entity_pos(0, 0, pp["gen"][0])
	if ep == null or absf(ep.y - (-160.0)) > 0.0001: fail("entity_pos 活句柄"); return
	if b.entity_pos(0, 1, 1) != null: fail("entity_pos 已退场 boss 应为 null"); return
	if b.entity_pos(0, 0, 9999) != null: fail("entity_pos 坏 gen 应为 null"); return
	# vanished 形状(本窗口弹只会越界消失、越界不记 → 空列,但四列必须在)。
	var vn: Dictionary = b.vanished()
	for k in ["x", "y", "sprite", "reason"]:
		if not vn.has(k): fail("vanished 缺列 " + k); return

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

	# frame_events 读口(通道 A 批量事实出口):按住射击键推进,自机弹飞上去打中
	# godot_smoke.ecl 那只 hp=9999 的敌(x=0,与自机同一列),应观察到 EVT_SHOT_HIT_ENEMY。
	# 这条不是形状断言——它要求读口真的把 core 侧的事件流透出来,且字段齐全。
	const EVT_SHOT_HIT_ENEMY := 9
	# 先把自机送回中列:上面 load_state 之后又按了 30 帧 BTN_LEFT,自机此刻在 x≈-135,
	# 不在靶敌(x=0)那一列上。左右速度与钳制对称,故同样 30 帧 BTN_RIGHT 正好回到 x≈0。
	for i in range(30): b.step_frame(WorldBridge.BTN_RIGHT)
	var saw_hit := false
	var bad_shape := false
	for i in range(180):
		b.step_frame(WorldBridge.BTN_SHOT)
		for ev in b.frame_events():
			for k in ["kind", "x", "y", "a_index", "a_gen", "data0", "data1"]:
				if not ev.has(k): bad_shape = true
			if int(ev.get("kind", 0)) == EVT_SHOT_HIT_ENEMY:
				saw_hit = true
				if int(ev.get("data0", 0)) <= 0: fail("命中事件 data0(damage) 应 >0"); return
	if bad_shape: fail("frame_events 条目字段不全"); return
	if not saw_hit: fail("按住射击 180 帧应观察到 EVT_SHOT_HIT_ENEMY"); return

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

	# ── rank 值域:核内校验 + 壳层饱和(难度档具名化刀) ──────────────────────
	# 两条负例都断言"返 false 且世界没被动"(P4-b:违约 = no-op)。第二条是判别式的关键:
	# 4294967296 = 2³²,`as i32` 截断后恰好是 **0 = RANK_EASY** 这个合法档——壳层若不饱和
	# 就会静默以 Easy 开局并返 true,核内那道 `RankOutOfRange` 根本轮不到执行。
	for bad_rank in [5, 4294967296, -4294967297]:
		if b.new_game_at(names, srcs, 7, bad_rank, 9, 0, 400, 3, 3):
			fail("越界 rank %d 应被拒(壳层饱和 + 核内校验)" % bad_rank); return
		if b.frame() != 2: fail("rank 被拒时世界不应被动(frame 变了)"); return
	if b.hud_player().get("power", -1) != 400: fail("rank 被拒时世界不应被动(power 变了)"); return
	# 上沿正例:4 = RANK_EXTRA 是合法档(只测越界会让判据写成 `0..=3` 也照过)。
	if not b.new_game_at(names, srcs, 7, WorldBridge.RANK_EXTRA, 9, 0, 400, 3, 3):
		fail("RANK_EXTRA(4) 是合法档,不该被拒"); return

	# ── 时间机制内核刀(2026-09-07,spec §4 桥级三条)──────────────────────────
	# 从头再开一局(单单元 new_game),用 godot_smoke.ecl 的弹流当靶:每 30 帧从 (-150,300)
	# 朝右发一颗 0.5px/帧 的弹,y 恒 300——自机走到 y≈300 再一路向左就会撞上。
	if not b.new_game(src, 7, 2): fail("time: new_game"); return
	var mm_ghost := RenderingServer.multimesh_create()
	RenderingServer.multimesh_allocate_data(mm_ghost, 8192, RenderingServer.MULTIMESH_TRANSFORM_2D, false, true)
	var seed_g := PackedFloat32Array(); seed_g.resize(8192 * 12)
	RenderingServer.multimesh_set_buffer(mm_ghost, seed_g)
	if b.preview(30): fail("time: 未注册影子层时 preview 应返 false"); return
	if not b.register_ghost_layer(mm_ghost): fail("time: register_ghost_layer"); return
	if not b.register_layer(b.LAYER_BULLETS, mm): fail("time: rl bullets"); return
	for i in range(40): b.step_frame(0) # 让场上有几颗弹
	# ① 跳躍:按一帧 BTN_JUMP → LIFE_JUMPING;第 29 帧仍在跳,第 30 帧回 ALIVE(N±1 判别)。
	if b.step_frame(WorldBridge.BTN_JUMP) != -1: fail("time: 跳躍不该返回落点"); return
	if int(b.hud_player()["life_state"]) != WorldBridge.LIFE_JUMPING: fail("time: 按跳躍后应为 LIFE_JUMPING"); return
	for i in range(WorldBridge.JUMP_FRAMES - 1): b.step_frame(0)
	if int(b.hud_player()["life_state"]) != WorldBridge.LIFE_JUMPING: fail("time: 第 N−1 帧仍应在跳"); return
	b.step_frame(0)
	if int(b.hud_player()["life_state"]) != 1: fail("time: 第 N 帧应回 ALIVE"); return
	# ② 影子:preview 后影子缓冲有实弹行(位置非零)、且不动权威世界(frame/checksum 不变)。
	var f_before: int = b.frame()
	var c_before: int = b.checksum()
	if not b.preview(WorldBridge.JUMP_FRAMES): fail("time: preview"); return
	if b.frame() != f_before or b.checksum() != c_before: fail("time: preview 动了权威世界"); return
	var gb := RenderingServer.multimesh_get_buffer(mm_ghost)
	if absf(gb[3]) < 0.0001 and absf(gb[7]) < 0.0001: fail("time: 影子层实例 0 应有弹(位置非零)"); return
	# 影子的弹比现在的弹多飞了 31 帧(0.5px/帧 → +15.5px):同一颗弹(压实序首行)x 差 15.5。
	var rb := RenderingServer.multimesh_get_buffer(mm)
	if absf((gb[3] - rb[3]) - 15.5) > 0.0001: fail("time: 影子首弹应比实弹多飞 15.5px,得 %f" % (gb[3] - rb[3])); return
	# ③ 遡行:先上到 y≈300,再向左撞弹流;进决死窗口按 V → step_frame 返回落点 F ==
	#    max(hit_frame−30, 环最老帧);frame() 回到 F;落地后 invuln 非零。
	var moved := 0
	while b.player_pos().y > 300.5 and moved < 120:
		b.step_frame(WorldBridge.BTN_UP); moved += 1
	if absf(b.player_pos().y - 300.0) > 4.0: fail("time: 自机未到 y≈300,得 %f" % b.player_pos().y); return
	var ring_oldest := 0 # new_game 后环首帧 = 0
	var hit_frame := -1
	var walked := 0
	while hit_frame < 0 and walked < 900:
		b.step_frame(WorldBridge.BTN_LEFT); walked += 1
		if int(b.hud_player()["life_state"]) == WorldBridge.LIFE_DEATHWINDOW:
			hit_frame = int(b.hud_player()["hit_frame"])
	if hit_frame < 0: fail("time: 向左走 900 帧没撞上弹流"); return
	var g0: int = b.frame()
	var landed: int = b.step_frame(WorldBridge.BTN_REWIND)
	var expect_to := maxi(hit_frame - WorldBridge.REWIND_DEPTH, ring_oldest)
	if landed != expect_to: fail("time: 遡行落点应为 %d,得 %d(hit_frame %d)" % [expect_to, landed, hit_frame]); return
	if b.frame() != expect_to: fail("time: frame() 应回到落点"); return
	if int(b.hud_player()["life_state"]) != 1: fail("time: 落地应为 ALIVE"); return
	if int(b.hud_player()["invuln"]) <= 0: fail("time: 落地应有无敌帧"); return
	# 倒放读口:被丢弃的请求帧 g0+1 在下一次 step 前仍可读,且切走视图不动权威帧号;
	# 落点本身也可读;step 后视图自动切回。
	if not b.view_ring(g0 + 1): fail("time: view_ring(被丢弃的请求帧)"); return
	if int(b.hud_player()["life_state"]) != WorldBridge.LIFE_DEATHWINDOW: fail("time: 视图帧应是决死窗口那一帧"); return
	if b.frame() != expect_to: fail("time: view_ring 不该动权威帧号"); return
	if not b.view_ring(expect_to): fail("time: view_ring(落点)"); return
	if b.view_ring(expect_to + 100000): fail("time: 不在环里的帧应返 false"); return
	b.step_frame(0)
	if b.frame() != expect_to + 1: fail("time: step 后应从落点继续"); return
	# replay_bytes:非空且魔数 STGR。
	var rb_bytes: PackedByteArray = b.replay_bytes()
	if rb_bytes.size() < 8 or rb_bytes.slice(0, 4).get_string_from_ascii() != "STGR": fail("time: replay_bytes 魔数"); return
	RenderingServer.free_rid(mm_ghost)

	# ── 壳子刀(2026-09-11,spec §7 桥级两条)────────────────────────────────────
	# ① 续关:1 条命开局,走进弹流 → 决死窗口耗尽 → GAMEOVER → 按 BTN_CONTINUE →
	#    RESPAWNING、continues==1、残机回默认 3、score==1。
	var one_name := PackedStringArray(["smoke.ecl"])
	var one_src := PackedStringArray([src])
	if not b.new_game_at(one_name, one_src, 7, 2, 0, 0, 0, 1, 3): fail("shell: new_game_at lives=1"); return
	var mv := 0
	while b.player_pos().y > 300.5 and mv < 120:
		b.step_frame(WorldBridge.BTN_UP); mv += 1
	var died := false
	var wk := 0
	while not died and wk < 1200:
		b.step_frame(WorldBridge.BTN_LEFT); wk += 1
		died = int(b.hud_player()["life_state"]) == WorldBridge.LIFE_GAMEOVER
	if not died: fail("shell: 1 条命走进弹流 1200 帧内应 GAMEOVER"); return
	var x_go: float = b.player_pos().x
	b.step_frame(WorldBridge.BTN_RIGHT) # GAMEOVER 下方向键零效
	if absf(b.player_pos().x - x_go) > 0.0001: fail("shell: GAMEOVER 下不该移动"); return
	if int(b.hud_player()["life_state"]) != WorldBridge.LIFE_GAMEOVER: fail("shell: GAMEOVER 应保持"); return
	b.step_frame(WorldBridge.BTN_CONTINUE)
	var hp2: Dictionary = b.hud_player()
	if int(hp2["life_state"]) != 3: fail("shell: 续关后应 RESPAWNING(3),得 %d" % int(hp2["life_state"])); return
	if int(hp2["continues"]) != 1: fail("shell: continues 应为 1"); return
	if int(hp2["lives"]) != 3: fail("shell: 续关残机回默认 3"); return
	if int(hp2["score"]) != 1: fail("shell: 续关后 score = 续关次数"); return
	for i in range(20): b.step_frame(0)
	var c_end: int = b.checksum()
	var f_end: int = b.frame()
	# ② 回放播放:倒出 log → new_game_from_replay 逐帧播完 → 帧号/校验和与录制末态相同;
	#    播放态 step 用 playback_step,is_playback 为真,播完返 -2 且再调仍 -2。
	var log_bytes: PackedByteArray = b.replay_bytes()
	if not b.new_game_from_replay(one_name, one_src, log_bytes): fail("shell: new_game_from_replay"); return
	if not b.is_playback(): fail("shell: is_playback 应为真"); return
	if b.playback_total() != f_end: fail("shell: playback_total 应 == 录制帧数 %d,得 %d" % [f_end, b.playback_total()]); return
	var guard := 0
	var last := -1
	while guard < 5000:
		last = b.playback_step()
		guard += 1
		if last == -2: break
	if last != -2: fail("shell: 播放应以 -2 结束"); return
	if b.frame() != f_end: fail("shell: 播完帧号应 == 录制末帧"); return
	if b.checksum() != c_end: fail("shell: 播完校验和应 == 录制末态"); return
	if b.playback_step() != -2: fail("shell: 播完再调仍 -2"); return
	var bad_log := log_bytes.duplicate()
	bad_log[12] ^= 1
	if b.new_game_from_replay(one_name, one_src, bad_log): fail("shell: 坏日志应被拒"); return
	if b.frame() != f_end: fail("shell: 拒收不该动世界"); return

	RenderingServer.free_rid(mm)
	b.free()
	print("SMOKE OK")
	quit(0)
