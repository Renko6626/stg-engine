class_name Hud
extends CanvasLayer
## 右栏(x≥424):分/残机/bomb/power/graze + 曲名;boss 条覆盖弹幕域顶部;中央横幅。

var score_l: Label
var lives_l: Label
var bombs_l: Label
var power_l: Label
var graze_l: Label
var bgm_l: Label
var boss_bar: ColorRect
var boss_bar_bg: ColorRect
var spell_l: Label
var banner: Label
var _banner_left := 0.0

func _ready() -> void:
	var panel := VBoxContainer.new()
	panel.position = Vector2(424, 24)
	panel.custom_minimum_size = Vector2(200, 0)
	add_child(panel)
	score_l = _row(panel); lives_l = _row(panel); bombs_l = _row(panel)
	power_l = _row(panel); graze_l = _row(panel); bgm_l = _row(panel)

	boss_bar_bg = ColorRect.new()
	boss_bar_bg.position = Vector2(40, 20); boss_bar_bg.size = Vector2(368, 4)
	boss_bar_bg.color = Color(1, 1, 1, 0.15); boss_bar_bg.visible = false
	add_child(boss_bar_bg)
	boss_bar = ColorRect.new()
	boss_bar.position = Vector2(40, 20); boss_bar.size = Vector2(368, 4)
	boss_bar.color = Color(0.9, 0.25, 0.35); boss_bar.visible = false
	add_child(boss_bar)
	spell_l = Label.new()
	spell_l.position = Vector2(40, 26)
	spell_l.add_theme_font_size_override("font_size", 10)
	add_child(spell_l)

	banner = Label.new()
	banner.position = Vector2(120, 200)
	banner.add_theme_font_size_override("font_size", 16)
	banner.visible = false
	add_child(banner)

func _row(p: Container) -> Label:
	var l := Label.new()
	l.add_theme_font_size_override("font_size", 12)
	p.add_child(l)
	return l

func _process(dt: float) -> void:
	if _banner_left > 0.0:
		_banner_left -= dt
		if _banner_left <= 0.0:
			banner.visible = false

func show_banner(text: String, secs: float) -> void:
	banner.text = text
	banner.visible = true
	_banner_left = secs

## 立即收回横幅(重开等场合用;`show_banner("", 0.0)` 不等价——`_process` 的
## `_banner_left > 0.0` 判据会跳过 0.0,变成"空文案常显"而非隐藏)。
func hide_banner() -> void:
	banner.visible = false
	_banner_left = 0.0

func set_bgm_label(n: String) -> void:
	bgm_l.text = "♪ " + n

func refresh(bridge: WorldBridge) -> void:
	var p := bridge.hud_player()
	if p.is_empty():
		return
	score_l.text = "Score  %d" % int(p["score"])
	lives_l.text = "Player %d (%d)" % [int(p["lives"]), int(p["life_pieces"])]
	bombs_l.text = "Bomb   %d (%d)" % [int(p["bombs"]), int(p["bomb_pieces"])]
	power_l.text = "Power  %.2f" % (int(p["power"]) / 100.0)
	graze_l.text = "Graze  %d" % int(p["graze"])
	var b := bridge.hud_boss(0)
	var active := not b.is_empty() and int(b["active"]) == 1
	boss_bar.visible = active
	boss_bar_bg.visible = active
	if active:
		boss_bar.size.x = 368.0 * clampf(float(b["hp_ratio"]), 0.0, 1.0)
		var s := bridge.hud_spell(0)
		if not s.is_empty() and int(s["active"]) == 1:
			var sname: String = ContentTables.SPELL_NAMES.get(int(s["spell_id"]), "Spell #%d" % int(s["spell_id"]))
			spell_l.text = "%s  %d" % [sname, int(s["frames_left"]) / 60]
		else:
			spell_l.text = ""
	else:
		spell_l.text = ""
