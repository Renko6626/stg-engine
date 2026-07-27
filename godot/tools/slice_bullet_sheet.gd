extends SceneTree
# 从原作弹片 bullet1.png 切出引擎用的弹图集(产物 commit,同烘焙表纪律:重跑逐位一致)。
# 用法: $GODOT_BIN --headless --path godot --script res://tools/slice_bullet_sheet.gd
#
# 【为什么是"切"而不是"拼"】bullet1.png 顶部 256×192 恰好已经是 16 列 × 12 行的
# 16×16 网格(逐格实测:12 行全满 16 色,无空格),与引擎的统一网格契约天然对齐,
# 只需裁掉 192px 以下的杂项区(那里是别的尺寸,不属本层)。
# 将来若要补 32×32 的大玉一类,整张图得改按 32 排、小图元居中留白(1:1 不缩放),
# 并同步 playfield.gd 的 CELLS[0] —— 见 docs/render-contract.md §3。
#
# 行序 = 弹型号(值 = 行 × color_stride),必须与三处保持同源:
#   crates/stg-core/src/tables.rs  build_tables_v0 的 SHAPE_RADIUS/SHAPE_COLOR_MASK
#   godot/ecl/demo/bullets.ecl     内容包词表
#   docs/render-contract.md §3     图集契约
const SRC := "res://assets/bullet/bullet1.png"
const DST := "res://assets/bullets.png"
const CELL := 16
const COLS := 16
const ROWS := 12

# 仅供人读:行序与名字(登记于 bullets.ecl,此处是对照表)
const ROW_NAMES := [
	"laser", "arrowhead", "outline", "ball", "rice", "kunai",
	"shard", "amulet", "bullet", "bacteria", "star", "laserhead",
]

func _init() -> void:
	var src := Image.load_from_file(ProjectSettings.globalize_path(SRC))
	if src == null:
		push_error("读不到源图: " + SRC)
		quit(1)
		return
	if src.get_width() < COLS * CELL or src.get_height() < ROWS * CELL:
		push_error("源图尺寸不足: %dx%d < %dx%d" % [
			src.get_width(), src.get_height(), COLS * CELL, ROWS * CELL])
		quit(1)
		return

	var out := Image.create(COLS * CELL, ROWS * CELL, false, Image.FORMAT_RGBA8)
	out.fill(Color(0, 0, 0, 0))
	out.blit_rect(src, Rect2i(0, 0, COLS * CELL, ROWS * CELL), Vector2i(0, 0))

	var err := out.save_png(ProjectSettings.globalize_path(DST))
	if err != OK:
		push_error("save_png 失败: " + DST)
		quit(1)
		return

	# 逐行自检并打印:每行的非空格数(应恒 16)与不透明包围盒最大边长,
	# 供 tables.rs 的 SHAPE_RADIUS/SHAPE_COLOR_MASK 对账。
	print("SLICE OK  %dx%d  (%d 列 × %d 行 × %dpx)" % [
		out.get_width(), out.get_height(), COLS, ROWS, CELL])
	for r in ROWS:
		var mask := 0
		var mw := 0
		var mh := 0
		for c in COLS:
			var lo := Vector2i(CELL, CELL)
			var hi := Vector2i(-1, -1)
			for y in CELL:
				for x in CELL:
					if out.get_pixel(c * CELL + x, r * CELL + y).a > 0.03:
						lo = Vector2i(mini(lo.x, x), mini(lo.y, y))
						hi = Vector2i(maxi(hi.x, x), maxi(hi.y, y))
			if hi.x >= 0:
				mask |= 1 << c
				mw = maxi(mw, hi.x - lo.x + 1)
				mh = maxi(mh, hi.y - lo.y + 1)
		print("  row %2d %-10s 非空格 %2d  掩码 0x%04X  最大 %2dx%-2d" % [
			r, ROW_NAMES[r], _popcount(mask), mask, mw, mh])
	quit(0)

func _popcount(v: int) -> int:
	var n := 0
	while v != 0:
		n += v & 1
		v >>= 1
	return n
