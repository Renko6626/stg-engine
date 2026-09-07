class_name Playfield
extends SubViewportContainer
## 弹幕域:SubViewport 384×448;世界根@(192,0)(世界坐标即本地坐标)。
## z 序(节点序,下→上):Bg < shots < Puppets(敌人节点) < items < Player < bullets < FxRoot。
## 表现契约 v2(2026-09-07):敌层退役,敌人走**节点木偶**(按池索引预分配 256 个 Sprite2D,
## 永不释放,每帧读 puppets() 压缩列);三层 MultiMesh 保留。

# 池容量镜像(值源 crates/stg-core/src/{bullets,shots,items}.rs define_pool! 声明;
# 桥面冻结不出容量口——漂移由 register_layer false + 冒烟兜底,见 setup)
# 2026-09-03(F12):道具层 512→1024,跟 items.rs 的 cap 一起改。这条镜像**没有编译期护栏**,
# 唯一的网就是本工程冒烟——改核心池 cap 而忘了这里,冒烟会以 "register_layer(N) 被拒
# (容量镜像漂移?)" + SMOKE FAIL: boot(0) 的形态报出来(本刀就是这么被抓住的)。
const CAPS := { 0: 8192, 1: 1024, 2: 1024 } # key = WorldBridge.LAYER_*(bullets/shots/items)
# cell 尺寸 = 该层图集的格边长(也是 QuadMesh 边长,1px=1unit)。
# bullets = 16:原作弹片本就是 16×16 网格,取 16 即精确切割、零留白,渲染出来正好是
# 东方在 384×448 场界里的原生比例。将来若补 32×32 大玉,整张图要改按 32 排、小图元
# 居中留白(1:1 不缩放),这里同步改 32——见 docs/render-contract.md §3。
const CELLS := { 0: 16, 1: 32, 2: 32 }
const COLS := { 0: 16, 1: 4, 2: 8 }
const ROWS := { 0: 12, 1: 1, 2: 1 }
const TEXTURES := {
	0: "res://assets/bullets.png", 1: "res://assets/shots.png", 2: "res://assets/items.png",
}
# 弹层出现闪光帧数(layer.gdshader `spawn_flash_frames`;其余层 0 = 关)。
const BULLET_SPAWN_FLASH_FRAMES := 6.0
# 节点序 = 绘制序;bullets 最上(东方惯例)。木偶插在 shots 之后、items 之前(原敌层位置)。
const Z_ORDER := [1, 2, 0] # shots, items, bullets——puppets 插在 shots 后、player 插在 items 后
# 敌人木偶:图集 enemies.png 64px 4×1;cap 镜像 enemy.rs(puppets() 行数永远 ≤ 它)。
const PUPPET_CAP := 256
const PUPPET_TEXTURE := "res://assets/enemies.png"
const PUPPET_HFRAMES := 4
const PUPPET_VFRAMES := 1
# 受击闪白的映射:hit_flash 列(帧计数,核内递减)→ uniform 0..1;≥ 本值即全白。
const HIT_FLASH_FULL := 4.0
# 场界 AABB(含弹的越界回收边距 64):钉死 custom_aabb 绕过 Godot 对全部实例重算 AABB
# (godot-proposals #957 指认的最大性能坑;表现契约 v2 §4.8)。世界根在 (192,0),这里是
# 本地坐标:x ∈ [-256, 256]、y ∈ [-64, 512]。
const FIELD_AABB := AABB(Vector3(-256, -64, -1), Vector3(512, 576, 2))

var viewport: SubViewport
var world_root: Node2D
var bg: Bg
var player: Sprite2D
var hitbox: Sprite2D
var layer_nodes := {}
var puppet_root: Node2D
var puppets: Array[Sprite2D] = []
var puppet_gen := PackedInt32Array()   # 节点记录的 gen(-1 = 空)
var _puppet_seen := PackedByteArray()  # 本帧访问标记(隐藏未出现者用)

func _init() -> void:
	position = Vector2(32, 16)
	stretch = true
	viewport = SubViewport.new()
	viewport.size = Vector2i(384, 448)
	viewport.disable_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	add_child(viewport)
	custom_minimum_size = Vector2(384, 448)

	bg = Bg.new()
	viewport.add_child(bg)
	world_root = Node2D.new()
	world_root.position = Vector2(192, 0)
	viewport.add_child(world_root)

	for kind in Z_ORDER:
		var mmi := _make_layer(kind)
		layer_nodes[kind] = mmi
		world_root.add_child(mmi)
		if kind == 1: # shots 之后插木偶(原敌层位置)
			_make_puppets()
		if kind == 2: # items 之后插 player(player 在 items 上、bullets 下)
			_make_player()

	var fx_root := Node2D.new() # Effects 挂点(T5 用),压 bullets 之上
	fx_root.name = "FxRoot"
	world_root.add_child(fx_root)

func _make_layer(kind: int) -> MultiMeshInstance2D:
	var mmi := MultiMeshInstance2D.new()
	var mm := MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_2D
	mm.use_custom_data = true
	mm.instance_count = CAPS[kind]
	var quad := QuadMesh.new()
	quad.size = Vector2(CELLS[kind], CELLS[kind])
	mm.mesh = quad
	mmi.multimesh = mm
	mmi.texture = load(TEXTURES[kind])
	var mat := ShaderMaterial.new()
	mat.shader = load("res://shaders/layer.gdshader")
	mat.set_shader_parameter("atlas", mmi.texture)
	mat.set_shader_parameter("grid_cols", float(COLS[kind]))
	mat.set_shader_parameter("grid_rows", float(ROWS[kind]))
	if kind == 0:
		mat.set_shader_parameter("spawn_flash_frames", BULLET_SPAWN_FLASH_FRAMES)
	mmi.material = mat
	# 播种定长零缓冲:①headless dummy renderer 下 get_buffer 才可用;②register_layer
	# 的判据就是 buffer 长度==cap×12(bridge.rs 契约注释)
	var buf := PackedFloat32Array()
	buf.resize(CAPS[kind] * 12)
	RenderingServer.multimesh_set_buffer(mm.get_rid(), buf)
	RenderingServer.multimesh_set_custom_aabb(mm.get_rid(), FIELD_AABB)
	mm.visible_instance_count = 0
	return mmi

func _make_puppets() -> void:
	puppet_root = Node2D.new()
	puppet_root.name = "Puppets"
	world_root.add_child(puppet_root)
	var tex: Texture2D = load(PUPPET_TEXTURE)
	var shader: Shader = load("res://shaders/puppet.gdshader")
	puppet_gen.resize(PUPPET_CAP)
	puppet_gen.fill(-1)
	_puppet_seen.resize(PUPPET_CAP)
	for i in PUPPET_CAP:
		var s := Sprite2D.new()
		s.texture = tex
		s.hframes = PUPPET_HFRAMES
		s.vframes = PUPPET_VFRAMES
		s.visible = false
		var mat := ShaderMaterial.new()
		mat.shader = shader
		s.material = mat
		puppet_root.add_child(s)
		puppets.append(s)

func _make_player() -> void:
	player = Sprite2D.new()
	player.texture = load("res://assets/player.png")
	world_root.add_child(player)
	hitbox = Sprite2D.new()
	hitbox.texture = load("res://assets/hitbox.png")
	hitbox.visible = false
	player.add_child(hitbox)

## 三层注册;任何一层失败 → push_error + false(容量镜像漂移在此炸出,冒烟接得住)
func setup(bridge: WorldBridge) -> bool:
	var ok := true
	for kind in layer_nodes:
		var mm: MultiMesh = layer_nodes[kind].multimesh
		if not bridge.register_layer(kind, mm.get_rid()):
			push_error("[stg] register_layer(%d) 被拒(容量镜像漂移?)" % kind)
			ok = false
	reset_puppets()
	return ok

## 重开/读档后清空木偶记忆:所有节点隐藏、gen 归 -1(下一帧按新世界重建)。
func reset_puppets() -> void:
	for i in PUPPET_CAP:
		puppets[i].visible = false
	puppet_gen.fill(-1)

func update_view(bridge: WorldBridge, buttons: int) -> void:
	player.position = bridge.player_pos()
	hitbox.visible = (buttons & WorldBridge.BTN_SLOW) != 0
	# 自机无敌闪烁(电平:hud_player().invuln;按帧号奇偶闪,不走壁钟)
	var p := bridge.hud_player()
	if not p.is_empty() and int(p.get("invuln", 0)) > 0:
		player.modulate.a = 0.35 if (bridge.frame() & 2) == 0 else 1.0
	else:
		player.modulate.a = 1.0
	_update_puppets(bridge)

## 敌人木偶(表现契约 v2 §5.1):每帧读 puppets() 压缩列。节点下标 = 池索引;gen 变了 = 新生
## (重置);按 (sprite, anm_state, state_age) 查 ContentTables 选格——手动设帧,不用壁钟。
func _update_puppets(bridge: WorldBridge) -> void:
	var pp: Dictionary = bridge.puppets()
	_puppet_seen.fill(0)
	if not pp.is_empty():
		var idx: PackedInt32Array = pp["index"]
		var gen: PackedInt32Array = pp["gen"]
		var xs: PackedFloat32Array = pp["x"]
		var ys: PackedFloat32Array = pp["y"]
		var sprites: PackedInt32Array = pp["sprite"]
		var states: PackedInt32Array = pp["anm_state"]
		var ages: PackedInt32Array = pp["state_age"]
		var flashes: PackedInt32Array = pp["hit_flash"]
		for k in idx.size():
			var i := idx[k]
			if i < 0 or i >= PUPPET_CAP:
				continue
			var node := puppets[i]
			if puppet_gen[i] != gen[k]:
				puppet_gen[i] = gen[k]
				node.modulate = Color.WHITE
			_puppet_seen[i] = 1
			node.position = Vector2(xs[k], ys[k])
			var state := states[k]
			if state == ContentTables.ANM_HIDDEN:
				node.visible = false
				continue
			node.frame = ContentTables.enemy_frame(sprites[k], state, ages[k])
			(node.material as ShaderMaterial).set_shader_parameter(
				"flash", clampf(float(flashes[k]) / HIT_FLASH_FULL, 0.0, 1.0))
			node.visible = true
	for i in PUPPET_CAP:
		if _puppet_seen[i] == 0 and puppets[i].visible:
			puppets[i].visible = false
