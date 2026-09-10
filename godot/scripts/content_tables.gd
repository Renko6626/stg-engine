class_name ContentTables
## 演出名表归内容包(id→名是表现层契约,引擎不注册;render-contract §4)。
const BGM_NAMES := { 1: "Stage 1 ~ Placeholder March", 2: "Boss ~ Windchime of Seven Colors" }
const SPELL_NAMES := { 1: "風鈴「Rainbow Wind Chime」" }
## 练习模式入口表(壳子刀):名字 → mark 号。约定第 N 关道中 N*10、boss 段 N*10+5
## (ecl/game/main.ecl 的 mark 号与此对应;引擎不管名字)。
const PRACTICE := [
	{ "name": "Stage 1", "mark": 10 },
	{ "name": "Stage 1 Boss", "mark": 15 },
]

## ── 敌人木偶动画表(表现契约 v2 §5.1)────────────────────────────────────────
## `anm_state` 是核内电平(ECL `set_anm_state` 写,世界不解释);这里把 `(sprite, state)` 映射到
## 图集格序列 + 每格持续帧数,壳侧按 `state_age`(step 后帧号 − 状态盖帧)**手动**选格——
## 永远不用 AnimatedSprite2D 的自动播放(它走壁钟,违反 render-contract §0 第 3 条)。
## 查不到的 `(sprite, state)` 退化为图集单格 `sprite`。约定状态号 `ANM_HIDDEN` = 隐藏。
const ANM_HIDDEN := 255
## sprite → { state → { "frames": [格号…], "period": 每格帧数 } }。
## 当前占位图集 enemies.png 是 4×1 单格(64px),故每个 sprite 只有单帧;表在这里是为了让
## 换真美术时只填表、不改代码。示例:给 sprite 0 的状态 1 配一段两格循环。
const ENEMY_ANIM := {
	0: { 0: { "frames": [0], "period": 1 }, 1: { "frames": [0, 2], "period": 8 } },
	1: { 0: { "frames": [1], "period": 1 } },
}

## 按 `(sprite, state, age)` 选图集格号(行优先;越界回卷由 Sprite2D.frame 自己取模不了,
## 这里对 frames 数组长度取模)。
static func enemy_frame(sprite: int, state: int, age: int) -> int:
	var by_state: Dictionary = ENEMY_ANIM.get(sprite, {})
	var anim: Dictionary = by_state.get(state, {})
	if anim.is_empty():
		return sprite
	var frames: Array = anim["frames"]
	var period: int = maxi(int(anim.get("period", 1)), 1)
	return int(frames[(age / period) % frames.size()])

## ── 特效种类(fx 层 custom.x;表现契约 v2 §5.2/§5.3)──────────────────────────
## 0..7 引擎/壳侧内建;脚本 `fx_at`/`fx_on` 的 kind 从 8 起自定(shader 未识别的 kind 画通用
## 闪点)。寿命单位是**帧**(age = step 后帧号 − 出生帧,不走壁钟;宿主暂停 = 帧不走 = 特效停)。
const FX_BULLET_FADE := 0   # 消弹淡出(数据源 vanished(),custom.w = 弹图集格号)
const FX_EXPLOSION := 1     # 敌死爆炸环(REQ_ENEMY_DEATH)
const FX_SPARK := 2         # 自机弹命中火花(EVT_SHOT_HIT_ENEMY)
const FX_FLASH := 3         # 通用闪点(脚本 fx_at 默认 / 未识别 kind)
const FX_AURA := 4          # 依附光环(fx_on 示例:跟随敌人,param = 半径像素)
const FX_LIFE := { 0: 12, 1: 20, 2: 7, 3: 30, 4: 90 }
const FX_LIFE_DEFAULT := 30

static func fx_life(kind: int, param: int) -> int:
	# 依附光环的寿命由 param 之外的第二语义给:param 是半径,寿命固定;其余按表。
	return int(FX_LIFE.get(kind, FX_LIFE_DEFAULT))
