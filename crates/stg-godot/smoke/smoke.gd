extends SceneTree

func _init():
	if not ClassDB.class_exists("WorldBridge"):
		push_error("WorldBridge 未注册——扩展没加载")
		quit(1)
		return
	var b = ClassDB.instantiate("WorldBridge")
	if b.ping() != 42:
		push_error("ping != 42")
		quit(1)
		return
	b.free()
	print("SMOKE OK")
	quit(0)
