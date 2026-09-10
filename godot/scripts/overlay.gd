class_name Overlay
extends CanvasLayer
## 盖在冻结画面上的页(壳子刀 2026-09-11):关间结算 / GAME OVER / 结果 / 练习结束 / 播完。
## 纯展示:数据由 Play 在 show_page 时一次性塞入(读口取自 hud_player),按键由 Play 分派
## (confirm/back/save_replay),这里不读输入。层号压在 HUD 之上。

enum Kind { NONE, STAGE_CLEAR, GAME_OVER, RESULT, PRACTICE_DONE, PLAYBACK_DONE }

var kind: int = Kind.NONE
var _dim: ColorRect
var _title: Label
var _body: Label
var _hint: Label

func _init() -> void:
	layer = 5
	_dim = ColorRect.new()
	_dim.color = Color(0, 0, 0, 0.55)
	_dim.position = Vector2(32, 16)
	_dim.size = Vector2(384, 448)
	add_child(_dim)
	_title = Label.new()
	_title.position = Vector2(64, 150)
	_title.add_theme_font_size_override("font_size", 22)
	add_child(_title)
	_body = Label.new()
	_body.position = Vector2(80, 200)
	_body.add_theme_font_size_override("font_size", 13)
	add_child(_body)
	_hint = Label.new()
	_hint.position = Vector2(64, 400)
	_hint.add_theme_font_size_override("font_size", 11)
	_hint.modulate = Color(0.8, 0.9, 1.0)
	add_child(_hint)
	visible = false

func show_page(p_kind: int, title: String, lines: Array, hint: String) -> void:
	kind = p_kind
	_title.text = title
	_body.text = "\n".join(PackedStringArray(lines))
	_hint.text = hint
	visible = true

func hide_page() -> void:
	kind = Kind.NONE
	visible = false
