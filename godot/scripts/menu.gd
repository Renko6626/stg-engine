class_name Menu
extends CanvasLayer
## 代码生成的纵向菜单(壳子刀):标题 / 难度 / 练习 / 回放列表共用。上下选、Z 确认、X 返回。
## 不读世界,不碰桥——纯 Godot 原生 UI,住墙钟时间。

signal chosen(index: int)
signal cancelled

var _title: Label
var _rows: Array[Label] = []
var _cursor := 0
var _items: Array = []

func _init() -> void:
	layer = 6
	_title = Label.new()
	_title.position = Vector2(80, 80)
	_title.add_theme_font_size_override("font_size", 24)
	add_child(_title)

func setup(title: String, items: Array) -> void:
	_title.text = title
	for r in _rows:
		r.queue_free()
	_rows.clear()
	_items = items
	_cursor = 0
	for i in items.size():
		var l := Label.new()
		l.position = Vector2(120, 160 + i * 28)
		l.add_theme_font_size_override("font_size", 16)
		add_child(l)
		_rows.append(l)
	_refresh()

func _refresh() -> void:
	for i in _rows.size():
		_rows[i].text = ("▶ " if i == _cursor else "   ") + str(_items[i])
		_rows[i].modulate = Color.WHITE if i == _cursor else Color(0.7, 0.7, 0.7)

func _unhandled_input(ev: InputEvent) -> void:
	if not visible or _items.is_empty():
		return
	if ev.is_action_pressed("ui_down"):
		_cursor = (_cursor + 1) % _items.size()
		_refresh()
	elif ev.is_action_pressed("ui_up"):
		_cursor = (_cursor - 1 + _items.size()) % _items.size()
		_refresh()
	elif ev.is_action_pressed("stg_shot") or ev.is_action_pressed("ui_accept"):
		chosen.emit(_cursor)
	elif ev.is_action_pressed("stg_bomb") or ev.is_action_pressed("ui_cancel"):
		cancelled.emit()

## 冒烟/脚本化用:不经输入直接选。
func pick(index: int) -> void:
	_cursor = clampi(index, 0, maxi(_items.size() - 1, 0))
	chosen.emit(_cursor)
