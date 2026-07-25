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
	var reqs_seen := 0
	for i in range(120):
		b.step_frame(0)
		reqs_seen += b.take_requests().size()
	if b.frame() != 120: fail("frame120"); return
	if reqs_seen == 0: fail("通道 B 零请求——emit_req 没到达"); return
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
	# 中段启动 + 锚点读口冒烟(task-7):godot_smoke.ecl main 顶层依次是
	# `bgm(3); mark(9);`——start=9 跳入落点直接执行 mark 自动补偿(注入 bgm=3,
	# spawn_enemy/初次 bgm(3) 声明本身都被 landing 跨过,补偿值仍是 3);
	# bg/bg_phase 全程未声明,应保持初值 0。复用既有 `src`(脚本内容不变)。
	var names := PackedStringArray(["smoke.ecl"])
	var srcs := PackedStringArray([src])
	if not b.new_game_at(names, srcs, 7, 2, 9, 0, 400, 3, 3): fail("new_game_at"); return
	if b.frame() != 0: fail("new_game_at 应从 frame 0 起"); return
	b.step_frame(0); b.step_frame(0)
	if b.frame() != 2: fail("new_game_at 后 step 计数错"); return
	var a: Dictionary = b.anchors()
	if a["bgm"] != 3: fail("anchors bgm 应为 mark(9) 垫片补偿值 3"); return
	if a["bg"] != 0: fail("anchors bg 应保持初值 0(全程未声明)"); return
	if a["bg_phase"] != 0: fail("anchors bg_phase 应保持初值 0(全程未声明)"); return
	if b.hud_player().get("power", -1) != 400: fail("new_game_at loadout power 未生效"); return
	b.free()
	print("SMOKE OK")
	quit(0)
