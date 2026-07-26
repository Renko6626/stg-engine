class_name Playfield
extends SubViewportContainer
## 弹幕域:SubViewport 384×448;世界根@(192,0)(世界坐标即本地坐标)。
## z 序(节点序,下→上):Bg < shots < enemies < items < Player < bullets < Effects(T5 挂)。

# 池容量镜像(值源 crates/stg-core/src/{bullets,shots,enemy,items}.rs define_pool! 声明;
# 桥面冻结不出容量口——漂移由 register_layer false + 冒烟兜底,见 setup)
const CAPS := { 0: 8192, 1: 1024, 2: 256, 3: 512 } # key = WorldBridge.LAYER_*
const CELLS := { 0: 32, 1: 32, 2: 64, 3: 32 }
const COLS := { 0: 8, 1: 4, 2: 4, 3: 8 }
const TEXTURES := {
	0: "res://assets/bullets.png", 1: "res://assets/shots.png",
	2: "res://assets/enemies.png", 3: "res://assets/items.png",
}
# 节点序 = 绘制序;bullets 最上(东方惯例)
const Z_ORDER := [1, 2, 3, 0] # shots, enemies, items, bullets——player 插在 items 后

var viewport: SubViewport
var world_root: Node2D
var bg: Bg
var player: Sprite2D
var hitbox: Sprite2D
var layer_nodes := {}

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
		if kind == 3: # items 之后插 player(player 在 items 上、bullets 下)
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
	mat.set_shader_parameter("grid_cols", float(COLS[kind]))
	mat.set_shader_parameter("grid_rows", 1.0)
	mmi.material = mat
	# 播种定长零缓冲:①headless dummy renderer 下 get_buffer 才可用;②register_layer
	# 的判据就是 buffer 长度==cap×12(bridge.rs 契约注释)
	var buf := PackedFloat32Array()
	buf.resize(CAPS[kind] * 12)
	RenderingServer.multimesh_set_buffer(mm.get_rid(), buf)
	mm.visible_instance_count = 0
	return mmi

func _make_player() -> void:
	player = Sprite2D.new()
	player.texture = load("res://assets/player.png")
	world_root.add_child(player)
	hitbox = Sprite2D.new()
	hitbox.texture = load("res://assets/hitbox.png")
	hitbox.visible = false
	player.add_child(hitbox)

## 四层注册;任何一层失败 → push_error + false(容量镜像漂移在此炸出,冒烟接得住)
func setup(bridge: WorldBridge) -> bool:
	var ok := true
	for kind in layer_nodes:
		var mm: MultiMesh = layer_nodes[kind].multimesh
		if not bridge.register_layer(kind, mm.get_rid()):
			push_error("[stg] register_layer(%d) 被拒(容量镜像漂移?)" % kind)
			ok = false
	return ok

func update_view(bridge: WorldBridge, buttons: int) -> void:
	player.position = bridge.player_pos()
	hitbox.visible = (buttons & WorldBridge.BTN_SLOW) != 0
