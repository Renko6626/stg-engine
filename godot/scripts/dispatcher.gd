class_name Dispatcher
extends Node
## 通道 B 路由:id → Callable。"核出请求,壳做演出"的壳侧落点。
## id 值源 crates/stg-core/src/reqs.rs(半冻结契约,1..=63 引擎段),经 WorldBridge.REQ_* 常量
## 转出(不手抄镜像;表现契约 v2 §4.7)。
##
## 分类与水位(表现契约 v2 §5.4 / render-contract §4):
## - 即发即忘:播完即忘,回滚误播接受为鬼影;
## - 须确认:缓冲到 `frame <= confirmed_frame` 才播(M3 前 confirmed_frame == 当前帧,
##   行为与从前完全一致;接回滚时只改 drain 的实参);
## - 电平镜像:边沿通知,真值在 anchors(),宿主开机/读档后先对表(main.gd `_sync_anchors`)。
## 水位:`frame <= watermark` 的请求丢弃(回滚重演已播过的帧);重开/读档 `reset()`。

enum Class { FIRE_AND_FORGET, CONFIRMED_ONLY, LEVEL_MIRROR }

var classes := {}
var handlers := {}
var watermark := -1
var pending: Array = []
var _warned := {}

func _init() -> void:
	classes = {
		WorldBridge.REQ_ENEMY_DEATH: Class.FIRE_AND_FORGET,
		WorldBridge.REQ_FX_AT: Class.FIRE_AND_FORGET,
		WorldBridge.REQ_FX_ATTACHED: Class.FIRE_AND_FORGET,
		WorldBridge.REQ_SPELL_DECLARE: Class.CONFIRMED_ONLY,
		WorldBridge.REQ_SPELL_RESULT: Class.CONFIRMED_ONLY,
		WorldBridge.REQ_STAGE_CLEAR: Class.CONFIRMED_ONLY,
		WorldBridge.REQ_BGM: Class.LEVEL_MIRROR,
		WorldBridge.REQ_BG: Class.LEVEL_MIRROR,
		WorldBridge.REQ_BG_PHASE: Class.LEVEL_MIRROR,
	}

func register(id: int, fn: Callable) -> void:
	handlers[id] = fn

## 脚本段(64+)默认即发即忘;内容包可改类(如 boss 登场演出标须确认)。
func set_class(id: int, cls: Class) -> void:
	classes[id] = cls

func reset() -> void:
	watermark = -1
	pending.clear()

func drain(arr: Array, confirmed_frame: int) -> void:
	var top := watermark
	for d in arr:
		var f: int = d["frame"]
		if f <= watermark:
			continue # 回滚重演已呈现过的帧(render-contract §4 水位)
		top = maxi(top, f)
		var id: int = d["id"]
		if classes.get(id, Class.FIRE_AND_FORGET) == Class.CONFIRMED_ONLY:
			pending.append(d)
		else:
			_dispatch(d)
	watermark = top
	# 须确认类:越过确认地平线才播(FIFO,保持帧序)
	while not pending.is_empty() and int(pending[0]["frame"]) <= confirmed_frame:
		_dispatch(pending.pop_front())

func _dispatch(d: Dictionary) -> void:
	var id: int = d["id"]
	if handlers.has(id):
		handlers[id].call(d["args"])
	elif not _warned.has(id):
		_warned[id] = true
		push_warning("[stg] 未注册请求 id=%d(同类后续不再报)" % id)
