# godot-scene 金向量证据

`golden-base.txt` 是 Godot 场景刀（T1 A5 `spawn_enemy` 追 `task` 参起，一路到 T6 demo
局+双冒烟）全程逐帧校验和流的一次快照——七任务收尾反复 `diff` 过它，全部空输出，证明本刀
对模拟演化零平移（含 A5 的 `spawn_enemy` 签名迁移，靠全仓调用点追加 `, none` 做到纯语法
迁移零行为改动）。它是**本刀的零平移证据存档**，不是 CI 基线：CLAUDE.md 口径——仓库不
committed 任何金向量基线，`determinism-gate` 只互比三平台当次产出，不跟历史文件对拍；
`golden-a5.txt`（与 `golden-base.txt` 逐位相同的中途重复产物）已删，避免误当"基线"引用。
