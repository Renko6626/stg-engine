class_name Hud
extends CanvasLayer
## 右栏(x≥424):分/残机/停止/power/graze/偏差 + 冷却条 + 曲名;boss 条覆盖弹幕域顶部;中央横幅。

var score_l: Label
var lives_l: Label
var bombs_l: Label
var power_l: Label
var graze_l: Label
var deaths_l: Label # 偏差值 = 死亡数(玩法刀)
var jump_bg: ColorRect # 跳躍冷却条底(满 = 可跳,只画条不写数字)
var jump_bar: ColorRect
var bgm_l: Label
var boss_bar: ColorRect
var boss_bar_bg: ColorRect
var spell_l: Label
var banner: Label
var time_l: Label # 时间机制提示:観測中 / 跳躍中 / 决死窗口「V 遡行」/ 遡行倒放中
var _banner_left := 0.0

func _ready() -> void:
	var panel := VBoxContainer.new()
	panel.position = Vector2(424, 24)
	panel.custom_minimum_size = Vector2(200, 0)
	add_child(panel)
	score_l = _row(panel); lives_l = _row(panel); bombs_l = _row(panel)
	power_l = _row(panel); graze_l = _row(panel); deaths_l = _row(panel); bgm_l = _row(panel)
	jump_bg = ColorRect.new()
	jump_bg.custom_minimum_size = Vector2(120, 4)
	jump_bg.color = Color(1, 1, 1, 0.15)
	panel.add_child(jump_bg)
	jump_bar = ColorRect.new()
	jump_bar.size = Vector2(120, 4)
	jump_bar.color = Color(0.55, 0.8, 1.0)
	jump_bg.add_child(jump_bar)

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

	time_l = Label.new()
	time_l.position = Vector2(40, 440)
	time_l.add_theme_font_size_override("font_size", 11)
	time_l.modulate = Color(0.7, 0.9, 1.0)
	add_child(time_l)

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

## 时间机制一行提示(电平:每帧由 main.gd 按自己的状态机 + hud_player 决定文案)。
func set_time_hint(text: String) -> void:
	time_l.text = text

func refresh(bridge: WorldBridge) -> void:
	var p := bridge.hud_player()
	if p.is_empty():
		return
	score_l.text = "Score  %d" % int(p["score"])
	lives_l.text = "Player %d (%d)" % [int(p["lives"]), int(p["life_pieces"])]
	bombs_l.text = "Stop   %d (%d)" % [int(p["bombs"]), int(p["bomb_pieces"])]
	deaths_l.text = "偏差   %d" % int(p.get("deaths", 0))
	var cd := int(p.get("jump_cd", 0))
	jump_bar.size.x = 120.0 * (1.0 - float(cd) / float(WorldBridge.JUMP_COOLDOWN))
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
			var secs := int(s["frames_left"]) / 60
			if int(s["flags"]) & WorldBridge.SPELL_NONSPELL:
				# 非符段（boss 换段刀）：只显示倒计时，不显示卡名
				spell_l.text = "%d" % secs
			else:
				var sname: String = ContentTables.SPELL_NAMES.get(int(s["spell_id"]), "Spell #%d" % int(s["spell_id"]))
				spell_l.text = "%s  %d" % [sname, secs]
		else:
			spell_l.text = ""
	else:
		spell_l.text = ""
