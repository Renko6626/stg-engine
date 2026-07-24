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
	b.free()
	print("SMOKE OK")
	quit(0)
