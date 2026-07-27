extends SceneTree
# 占位图集一次性生成(产物 commit,同烘焙表纪律:重跑逐位一致)。
# 用法: $GODOT_BIN --headless --path godot --script res://tools/gen_atlas.gd
#
# 【弹层已不在本脚本】bullets.png 现由真美术切割而来,见 tools/slice_bullet_sheet.gd。
# 本脚本**绝不可**再生成 bullets.png——否则一跑就把真图集覆盖成占位图。
# 其余五层仍是占位,换真美术时照此从本脚本移除对应那一行。
const PALETTE := [
	Color(0.95, 0.30, 0.30), Color(0.30, 0.55, 0.95), Color(0.35, 0.85, 0.40),
	Color(0.95, 0.80, 0.25), Color(0.80, 0.40, 0.90), Color(0.30, 0.85, 0.85),
	Color(0.95, 0.55, 0.25), Color(0.75, 0.75, 0.80),
]

func _init() -> void:
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
