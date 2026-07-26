class_name Dispatcher
extends Node
## 通道 B 路由:id → Callable。"核出请求,壳做演出"的壳侧落点。
## id 值源 crates/stg-core/src/reqs.rs(半冻结契约,1..=63 引擎段;桥面冻结不镜像常量)。

const REQ_ENEMY_DEATH := 1
const REQ_SPELL_DECLARE := 2
const REQ_SPELL_RESULT := 3
const REQ_STAGE_CLEAR := 4
const REQ_BGM := 5
const REQ_BG := 6
const REQ_BG_PHASE := 7

var handlers := {}
var _warned := {}

func register(id: int, fn: Callable) -> void:
	handlers[id] = fn

func drain(arr) -> void:
	for d in arr:
		var id: int = d["id"]
		if handlers.has(id):
			handlers[id].call(d["args"])
		elif not _warned.has(id):
			_warned[id] = true
			push_warning("[stg] 未注册请求 id=%d(同类后续不再报)" % id)
