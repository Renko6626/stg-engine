class_name Bg
extends Node2D
## 占位背景:底色 + 滚动网格线。bg 号→底色;bg_phase 硬编码 0=滚 1=停(A4 mini-VM 接管前)。
## 表现层自有时钟(_process float dt)——断层线上,无纪律负担。

const BG_COLORS := { 0: Color(0.06, 0.06, 0.10), 1: Color(0.05, 0.10, 0.08), 2: Color(0.10, 0.05, 0.10) }

var base_color: Color = BG_COLORS[0]
var scrolling := true
var offset := 0.0

func set_bg(id: int) -> void:
	base_color = BG_COLORS.get(id, BG_COLORS[0])
	queue_redraw()

func set_phase(phase: int) -> void:
	scrolling = phase == 0

func _process(dt: float) -> void:
	if scrolling:
		offset = fmod(offset + 40.0 * dt, 32.0)
		queue_redraw()

func _draw() -> void:
	draw_rect(Rect2(0, 0, 384, 448), base_color)
	var line := Color(1, 1, 1, 0.05)
	var y := offset - 32.0
	while y < 448.0:
		draw_line(Vector2(0, y), Vector2(384, y), line)
		y += 32.0
	for x in range(0, 385, 32):
		draw_line(Vector2(x, 0), Vector2(x, 448), line)
