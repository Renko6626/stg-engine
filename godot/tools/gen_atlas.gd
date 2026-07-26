extends SceneTree
# 占位图集一次性生成(产物 commit,同烘焙表纪律:重跑逐位一致)。
# 用法: $GODOT_BIN --headless --path godot --script res://tools/gen_atlas.gd
const PALETTE := [
	Color(0.95, 0.30, 0.30), Color(0.30, 0.55, 0.95), Color(0.35, 0.85, 0.40),
	Color(0.95, 0.80, 0.25), Color(0.80, 0.40, 0.90), Color(0.30, 0.85, 0.85),
	Color(0.95, 0.55, 0.25), Color(0.75, 0.75, 0.80),
]

# 12 形 × 16 色(值源 crates/stg-core/src/tables.rs build_tables_v0 的
# SHAPE_RADIUS / SHAPE_COLOR_MASK;两边必须同源,改一边要改另一边)
# 注意:下面 SHAPE_RADIUS 是 tables.rs 那组值(世界坐标判定半径,单位 px)的 ×2——
# 32px 格子里画得清楚的"显示半径"与"世界判定半径"是两个独立的量,只要求同步改、
# 不要求数值相等;照抄 tables.rs 新数值时记得再乘 2,否则占位观感会悄悄跟真判定脱节。
const SHAPE_RADIUS := [6.0, 6.0, 8.0, 12.0, 8.0, 8.0, 6.0, 10.0, 14.0, 12.0, 12.0, 8.0]
const SHAPE_COLOR_MASK := [
	0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
	0xFFFF, 0x0FFF, 0x0FFF, 0xFFFF,
]

func _init() -> void:
	_atlas("res://assets/bullets.png", 16, 32, _cell_bullet, 12)
	_atlas("res://assets/shots.png", 4, 32, _cell_shot)
	_atlas("res://assets/enemies.png", 4, 64, _cell_enemy)
	_atlas("res://assets/items.png", 8, 32, _cell_item)
	_atlas("res://assets/player.png", 1, 32, _cell_player)
	_atlas("res://assets/hitbox.png", 1, 16, _cell_hitbox)
	print("ATLAS OK")
	quit(0)

func _atlas(path: String, cols: int, cell: int, painter: Callable, rows: int = 1) -> void:
	var img := Image.create(cols * cell, rows * cell, false, Image.FORMAT_RGBA8)
	img.fill(Color(0, 0, 0, 0))
	for r in rows:
		for c in cols:
			painter.call(img, c * cell, r * cell, cell, r * cols + c)
	var err := img.save_png(path)
	assert(err == OK, "save_png 失败: " + path)

# 距离场画圆:核白心 + 色环(占位弹的通用观感)
func _disc(img: Image, ox: int, oy: int, cell: int, r: float, col: Color) -> void:
	var cx := cell / 2.0
	for y in cell:
		for x in cell:
			var d := Vector2(x + 0.5 - cx, y + 0.5 - cx).length()
			if d < r * 0.55:
				img.set_pixel(ox + x, oy + y, Color(1, 1, 1, 1))
			elif d < r:
				img.set_pixel(ox + x, oy + y, col)
			elif d < r + 1.5:
				var a := clampf(r + 1.5 - d, 0.0, 1.0)
				img.set_pixel(ox + x, oy + y, Color(col.r, col.g, col.b, a))

# 上下有明暗渐变的圆(占位图元必须上下不对称——B23 判决的肉眼靶子,见 spec §8)
func _disc_shaded(img: Image, ox: int, oy: int, cell: int, r: float, col: Color) -> void:
	var cx := cell / 2.0
	for y in cell:
		for x in cell:
			var d := Vector2(x + 0.5 - cx, y + 0.5 - cx).length()
			if d >= r + 1.5:
				continue
			# 上半格亮、下半格暗——**故意上下不对称**(B23 判决的肉眼靶子)
			var k := 1.25 - 0.6 * (float(y) / float(cell))
			var c := Color(col.r * k, col.g * k, col.b * k)
			if d < r * 0.5:
				img.set_pixel(ox + x, oy + y, Color(minf(c.r + 0.5, 1.0), minf(c.g + 0.5, 1.0), minf(c.b + 0.5, 1.0)))
			elif d < r:
				img.set_pixel(ox + x, oy + y, c)
			else:
				img.set_pixel(ox + x, oy + y, Color(c.r, c.g, c.b, clampf(r + 1.5 - d, 0.0, 1.0)))

func _hue_color(i: int) -> Color:
	# 16 色占位:色相环 12 色 + 白/灰/黑/金(与内容包词表 COLOR_* 同序)
	if i == 12: return Color(1, 1, 1)
	if i == 13: return Color(0.6, 0.6, 0.65)
	if i == 14: return Color(0.15, 0.15, 0.2)
	if i == 15: return Color(0.95, 0.8, 0.3)
	return Color.from_hsv(float(i) / 12.0, 0.85, 0.95)

func _cell_bullet(img: Image, ox: int, oy: int, cell: int, i: int) -> void:
	var shape := i / 16
	var color := i % 16
	if SHAPE_COLOR_MASK[shape] >> color & 1 == 0:
		return # 空格:整格透明(踩到它的脚本会在编译期/运行期被拒)
	_disc_shaded(img, ox, oy, cell, SHAPE_RADIUS[shape], _hue_color(color))

func _cell_shot(img: Image, ox: int, oy: int, cell: int, i: int) -> void:
	# 自机弹:竖长针(椭圆度量),号变色
	var col: Color = PALETTE[(i + 1) % PALETTE.size()]
	var cx := cell / 2.0
	for y in cell:
		for x in cell:
			var d := Vector2((x + 0.5 - cx) / 0.35, (y + 0.5 - cx)).length()
			if d < 10.0: img.set_pixel(ox + x, oy + y, Color(col.r, col.g, col.b, 0.9))

func _cell_enemy(img: Image, ox: int, oy: int, cell: int, i: int) -> void:
	# 0=杂兵 1=boss 2/3=备用;大圆身 + 深色描边由 _disc 环体现
	_disc(img, ox, oy, cell, [18.0, 26.0, 20.0, 22.0][i], PALETTE[(i + 4) % PALETTE.size()])

func _cell_item(img: Image, ox: int, oy: int, cell: int, i: int) -> void:
	# 方块图标,号变色(P 点/分点等观感区分交真美术)
	var col: Color = PALETTE[i % PALETTE.size()]
	for y in range(8, cell - 8):
		for x in range(8, cell - 8):
			img.set_pixel(ox + x, oy + y, col)

func _cell_player(img: Image, ox: int, oy: int, cell: int, _i: int) -> void:
	# 上尖三角(自机朝上)
	var cx := cell / 2.0
	for y in cell:
		for x in cell:
			if absf(x + 0.5 - cx) < float(y) * 0.45 and y > 4 and y < cell - 4:
				img.set_pixel(ox + x, oy + y, Color(0.9, 0.9, 1.0, 1.0))

func _cell_hitbox(img: Image, ox: int, oy: int, cell: int, _i: int) -> void:
	_disc(img, ox, oy, cell, 5.0, Color(1.0, 0.2, 0.2))
