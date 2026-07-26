class_name Effects
extends Node2D
## 一次性演出(挂 Playfield.world_root/FxRoot 下,世界坐标)。表现层自有计时,无纪律负担。

func explosion(pos: Vector2, score: int) -> void:
	var e := _Ring.new()
	e.position = pos
	add_child(e)
	if score > 0:
		var l := Label.new()
		l.text = str(score)
		l.position = pos + Vector2(-12, -20)
		l.add_theme_font_size_override("font_size", 10)
		add_child(l)
		var tw := l.create_tween()
		tw.tween_property(l, "position:y", l.position.y - 24.0, 0.6)
		tw.parallel().tween_property(l, "modulate:a", 0.0, 0.6)
		tw.tween_callback(l.queue_free)

class _Ring extends Node2D:
	var t := 0.0
	func _process(dt: float) -> void:
		t += dt * 3.0
		if t >= 1.0:
			queue_free()
		queue_redraw()
	func _draw() -> void:
		draw_arc(Vector2.ZERO, 4.0 + t * 28.0, 0, TAU, 24,
			Color(1.0, 0.8, 0.4, 1.0 - t), 2.0)
