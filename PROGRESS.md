# 当前进度

2026-11-05。这里记录当前状态；过去的交付与验证记录见[历史进度](docs/archive/progress-through-2026-11-05.md)，待办见[技术债清单](docs/follow-ups.md)。

- Rust 确定性内核、ECL 编译器、Godot 游戏壳和 RL/Python 接口已落地；架构入口见[子系统地图](docs/architecture.md)。
- 当前 ENGINE_VER32：新增按局 LEGACY/MODERN 圆形判定及独立擦弹开关，接入 Rust、Godot、RL/Python；默认 LEGACY/开启擦弹。规则进入快照、存档和回放，回放格式升至3；有意识差异及未覆盖的矩形体碰见[碰撞规则](docs/collision-rules.md)。
- TH06/TH18 弹型目录保留旧 192 项，扩至 1120 个逻辑项、635 个物理图格。目录输入、生成物和重烘方法见[弹型目录](docs/bullet-catalog.md)。
- 碰撞更新的 core/RL/harness/Godot 测试、fmt/clippy、表校验与三段 golden 通过；本地 Python wheel 测试17通过、1跳过，独立审阅无发现。未做跨平台或原作实机验证，未发布或接入训练仓。弹型此前的验证记录保留在历史进度中。
- xform 三阶段 rand、动态表达式、单值 STEP、敌人共享变量及缓动插值已实现；设计记录见[历史档案](docs/archive/README.md)，提交状态以 Git 历史为准。
- 本轮整理：历史设计与计划归档，工具过程产物移出仓库；harness 按 bench/golden/共用风铃场景归位；syscall 测试按七个领域分组，共用辅助留在入口。整理本身保留原有测试与运行逻辑，相关检查通过，harness 拆分前后 golden 逐字节一致。以上与弹型、碰撞更新一并阶段性交付，提交以 Git 历史为准。
- 原作导入器、弹任务带参、共享 shooter/时钟、EX 出生态和曲线激光仍属独立范围，具体路线需另行确认。

## 维护

更新时重写当前状态，保留约 5–8 条；已完成事项的细节留在 Git 历史或已有档案，不追加整段复审记录。历史设计不自动授权新功能，新增长期文档只在确有持续维护价值时创建。
