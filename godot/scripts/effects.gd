class_name Effects
extends Node2D
## 一次性演出(挂 Playfield.world_root/FxRoot 下,世界坐标)。表现契约 v2(2026-09-07)重写:
## **fx 层 = 一张 MultiMesh + 程序化 shader,行由这里的池管理**,时间基是 step 后的帧号
## (age = frame − born),不走壁钟——宿主暂停 = 帧不走 = 特效原地停;回滚/读档后
## `clear_all()` 即可。此前每个火花一个带 _process/_draw 的节点,擦弹一接就炸(B28)。
##
## 行 = (kind, x, y, born, param, sprite, index, gen)。`index >= 0` 的是依附行(fx_on):
## 每帧经 entity_pos 跟随,句柄失效即回收。缓冲布局同各实体层:stride 12,
## [1,0,0,x, 0,1,0,y, kind,age,param,sprite](缩放在 shader 里按 kind 算)。
## 飘字仍是 Label(数量小)。

const CAP := 2048
const QUAD_PX := 64.0

var mmi: MultiMeshInstance2D
var mm: MultiMesh
var buf := PackedFloat32Array()
# 行池(并行数组;活行前缀无序,回收用 swap-remove)
var n := 0
var kind := PackedInt32Array()
var xs := PackedFloat32Array()
var ys := PackedFloat32Array()
var born := PackedInt32Array()
var param := PackedInt32Array()
var sprite := PackedInt32Array()
var ent_index := PackedInt32Array()
var ent_gen := PackedInt32Array()
var _labels: Array[Label] = []

func _init() -> void:
	for a in [kind, born, param, sprite, ent_index, ent_gen]:
		a.resize(CAP)
	xs.resize(CAP); ys.resize(CAP)
	buf.resize(CAP * 12)
	mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_2D
	mm.use_custom_data = true
	mm.instance_count = CAP
	var quad := QuadMesh.new()
	quad.size = Vector2(QUAD_PX, QUAD_PX)
	mm.mesh = quad
	mmi = MultiMeshInstance2D.new()
	mmi.multimesh = mm
	var mat := ShaderMaterial.new()
	mat.shader = load("res://shaders/fx.gdshader")
	mat.set_shader_parameter("bullet_atlas", load(Playfield.TEXTURES[0]))
	mat.set_shader_parameter("grid_cols", float(Playfield.COLS[0]))
	mat.set_shader_parameter("grid_rows", float(Playfield.ROWS[0]))
	mat.set_shader_parameter("quad_px", QUAD_PX)
	mat.set_shader_parameter("life_0_3", Vector4(
		ContentTables.FX_LIFE[0], ContentTables.FX_LIFE[1],
		ContentTables.FX_LIFE[2], ContentTables.FX_LIFE[3]))
	mat.set_shader_parameter("life_4", float(ContentTables.FX_LIFE[4]))
	mmi.material = mat
	RenderingServer.multimesh_set_buffer(mm.get_rid(), buf)
	RenderingServer.multimesh_set_custom_aabb(mm.get_rid(), Playfield.FIELD_AABB)
	mm.visible_instance_count = 0
	add_child(mmi)

## 起一行。满了丢最老的(池内无序,取"活行 0"近似最老——即发即忘特效可掉)。
func spawn(k: int, x: float, y: float, frame: int, p: int = 0, spr: int = 0, ei: int = -1, eg: int = 0) -> void:
	var i := n
	if n >= CAP:
		i = 0
	else:
		n += 1
	kind[i] = k; xs[i] = x; ys[i] = y; born[i] = frame
	param[i] = p; sprite[i] = spr; ent_index[i] = ei; ent_gen[i] = eg

## 依附特效(fx_on → REQ_FX_ATTACHED):出生位置由 entity_pos 取,取不到(句柄已死)就不起。
func spawn_attached(bridge: WorldBridge, k: int, ei: int, eg: int, p: int) -> void:
	var pos = bridge.entity_pos(0, ei, eg)
	if pos == null:
		return
	spawn(k, pos.x, pos.y, bridge.frame(), p, 0, ei, eg)

## 消弹淡出批:vanished() 的四列 → 每行一条 FX_BULLET_FADE(custom.w = 弹格号)。
func fade_batch(vn: Dictionary, frame: int) -> void:
	if vn.is_empty():
		return
	var vx: PackedFloat32Array = vn["x"]
	var vy: PackedFloat32Array = vn["y"]
	var vs: PackedInt32Array = vn["sprite"]
	for j in vx.size():
		spawn(ContentTables.FX_BULLET_FADE, vx[j], vy[j], frame, 0, vs[j])

func explosion(pos: Vector2, score: int, frame: int) -> void:
	spawn(ContentTables.FX_EXPLOSION, pos.x, pos.y, frame)
	if score > 0:
		_popup(pos, str(score))

func hit_spark(pos: Vector2, frame: int) -> void:
	spawn(ContentTables.FX_SPARK, pos.x, pos.y, frame)

## 每帧(step 后):回收到期/句柄失效的行,依附行跟随,重写缓冲前缀并上传。
func tick(bridge: WorldBridge) -> void:
	var frame := bridge.frame()
	var i := 0
	while i < n:
		var age := frame - born[i]
		var dead := age >= ContentTables.fx_life(kind[i], param[i]) or age < 0
		if not dead and ent_index[i] >= 0:
			var pos = bridge.entity_pos(0, ent_index[i], ent_gen[i])
			if pos == null:
				dead = true
			else:
				xs[i] = pos.x; ys[i] = pos.y
		if dead:
			_swap_remove(i)
			continue
		var o := i * 12
		buf[o] = 1.0; buf[o + 1] = 0.0; buf[o + 2] = 0.0; buf[o + 3] = xs[i]
		buf[o + 4] = 0.0; buf[o + 5] = 1.0; buf[o + 6] = 0.0; buf[o + 7] = ys[i]
		buf[o + 8] = float(kind[i]); buf[o + 9] = float(age)
		buf[o + 10] = float(param[i]); buf[o + 11] = float(sprite[i])
		i += 1
	RenderingServer.multimesh_set_buffer(mm.get_rid(), buf)
	RenderingServer.multimesh_set_visible_instances(mm.get_rid(), n)

func _swap_remove(i: int) -> void:
	n -= 1
	if i != n:
		kind[i] = kind[n]; xs[i] = xs[n]; ys[i] = ys[n]; born[i] = born[n]
		param[i] = param[n]; sprite[i] = sprite[n]
		ent_index[i] = ent_index[n]; ent_gen[i] = ent_gen[n]

## 重开/读档:清空全部行与飘字(否则旧世界坐标的残留会等自己到期才消失,A9 ⑤)。
## 时间跳变清理(F20,render-contract §0.5):只杀出生帧 > frame 的行——遡行落点之前开始的
## 演出继续活;依附行一并按出生帧处理(其宿主若不在落点世界里,tick 时 entity_pos 返 null 自会回收)。
func clear_after(frame: int) -> void:
	var i := 0
	while i < n:
		if born[i] > frame:
			_swap_remove(i)
		else:
			i += 1
	RenderingServer.multimesh_set_visible_instances(mm.get_rid(), n)
	for l in _labels:
		if is_instance_valid(l):
			l.queue_free()
	_labels.clear()

func clear_all() -> void:
	n = 0
	RenderingServer.multimesh_set_visible_instances(mm.get_rid(), 0)
	for l in _labels:
		if is_instance_valid(l):
			l.queue_free()
	_labels.clear()

func _popup(pos: Vector2, text: String) -> void:
	var l := Label.new()
	l.text = text
	l.position = pos + Vector2(-12, -20)
	l.add_theme_font_size_override("font_size", 10)
	add_child(l)
	_labels.append(l)
	var tw := l.create_tween()
	tw.tween_property(l, "position:y", l.position.y - 24.0, 0.6)
	tw.parallel().tween_property(l, "modulate:a", 0.0, 0.6)
	tw.tween_callback(func():
		_labels.erase(l)
		l.queue_free())
