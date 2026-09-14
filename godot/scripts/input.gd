class_name StgInput
extends Node
## InputMap 代码注册(免手写序列化;物理键:方向键 + Z 射 X 停止 Shift 低速 C 観測/跳躍)。
## 位掩码零翻译:WorldBridge.BTN_* 即 stg-core 动作位。玩法刀(2026-09-14):X = 停止(BTN_BOMB,
## 时停+触碰消弹合一),D/V 两键退场(遡行改为死亡自动触发)。**跳躍位不在 mask() 里**:
## 観測→跳躍的两段协议归 play.gd,只在第二下按 C 时注入一帧 BTN_JUMP。

const KEYS := {
	"stg_up": KEY_UP, "stg_down": KEY_DOWN,
	"stg_left": KEY_LEFT, "stg_right": KEY_RIGHT,
	"stg_shot": KEY_Z, "stg_bomb": KEY_X, "stg_slow": KEY_SHIFT,
	"stg_observe": KEY_C,
}

func _ready() -> void:
	for a in KEYS:
		if InputMap.has_action(a):
			continue
		InputMap.add_action(a)
		var ev := InputEventKey.new()
		ev.physical_keycode = KEYS[a]
		InputMap.action_add_event(a, ev)

func mask() -> int:
	var m := 0
	if Input.is_action_pressed("stg_up"): m |= WorldBridge.BTN_UP
	if Input.is_action_pressed("stg_down"): m |= WorldBridge.BTN_DOWN
	if Input.is_action_pressed("stg_left"): m |= WorldBridge.BTN_LEFT
	if Input.is_action_pressed("stg_right"): m |= WorldBridge.BTN_RIGHT
	if Input.is_action_pressed("stg_shot"): m |= WorldBridge.BTN_SHOT
	if Input.is_action_pressed("stg_bomb"): m |= WorldBridge.BTN_BOMB
	if Input.is_action_pressed("stg_slow"): m |= WorldBridge.BTN_SLOW
	return m

## 観測键的上升沿(物理帧级 just_pressed)。
func observe_pressed() -> bool:
	return Input.is_action_just_pressed("stg_observe")
