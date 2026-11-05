# 敌人脚本共享变量实施计划

Spec: `docs/archive/specs/2026-10-26-enemy-script-vars-design.md`（用户已确认）。
工作分支 `codex/core-module-cleanup`；实施阶段保留已有未提交整理，不自动提交或推送。2026-10-26验收后，用户另行授权阶段性提交推送，见文末交付记录。

## Global Constraints

- 确定性 Q16.16/BAM、固定 60 Hz、扁平 POD、按槽升序。World 不依赖 ECL 类型。
- EnemyPool 每敌 4 个 i32、8 个 Fx；常量 ENEMY_INT_SLOTS=4、ENEMY_FX_SLOTS=8。
- self_int/self_fx 读号170/171，set_self_int/set_self_fx 写号450/451。仅当前敌 owner，可立即共享；任务结束和阶段切换保留，创建及复用清零，不隐式继承。
- 非敌 owner FAULT_BAD_OP；坏槽 read0/write no-op、一次 contract_viol、STATUS_BAD_ARGS；失效句柄沿用 STATUS_STALE_HANDLE。
- ENGINE_VER=27，全槽 checksum/save/copy，数值状态增加12288 B，旧状态不兼容。
- 不改任务调度、shooter 所有权、插值写回、训练仓和导入器。不新增依赖或防御机制。
- 新增测试只覆盖关键行为缺口；沿用真实生产逻辑和已有夹具。集中验证，不反复全量重跑。

### Task 1: runtime/compiler 接入

所有权：crates/ 下 runtime、编译器及受影响 EnemyInit 调用点、版本/尺寸哨兵、必要 core 单测。主 agent 独占新增集成测试 enemy_vars.rs 及文档/生成物，勿覆盖。

1. enemy.rs 新增 ENEMY_INT_SLOTS/ENEMY_FX_SLOTS 及 script_int/script_fx 池字段，所有 EnemyInit 显式初始化零。
2. world 层提供 pub(crate) handle-based getter/setter（可独立 world/enemy_vars.rs），使用已有 P4 约定。create_enemy 保证新敌脚本槽清零；不在任务创建/退出/阶段切换时清理。
3. syscall 根保留新 public SYS 常量、白名单和 dispatch；实现放私有 enemy.rs；弹参顺序遵循现有，i32 槽不允许窄化回绕。compiler builtin registry/typecheck 和 const registry 注入新名字，普通 builtin 走已集中 gen_builtin_call。
4. ENGINE_VER 26→27，WorldBody/World 两个布局各+12288 B（预计1070616/1283664），EnemyPool30496→42784；更新冻结 syscall/compiler 表。
5. 先添加一个世界层行为测试验证复用清零与旧句柄隔离，运行并记录预期失败，再实现；该层不重复主 agent 的脚本端到端覆盖。主 agent 负责真实脚本测试共享、隔离、非法槽、save/copy恢复和 owner fault。
6. 执行受影响局部检查（与主 agent 协调 Cargo 锁），自审，写报告，不提交或推送。

### Task 2: 端到端行为、文档与验证（主 agent）

先写 compiler/tests/enemy_vars.rs 并记录未实现失败。用真实脚本覆盖父子双向读写、不同敌隔离、不继承、task退出保留、非法槽诊断、save/copy恢复后续算和错误 owner。补 ECL 手册、ops、池内存表、版本与进度，运行 gen-ecl-meta。
集中运行 fmt、受影响 core/compiler/harness/RL tests、clippy、verify-tables、golden。新 golden 仅记录新哈希；不要求等于26版。最终用 Sol high 审阅完整功能增量和关联整理接缝，不重跑已通过验证；处理具体缺陷后只重测受影响内容。


## 实施与验证记录

已实现池字段、世界读写、syscall、编译器注册与常量注入；更新全部 EnemyInit、ENGINE_VER27 和尺寸哨兵。运行时代码由 Sol medium 实施，主 agent 编写4项真实脚本集成测试并同步文档与生成物；世界层增加1项复用/旧句柄测试。行为测试覆盖同敌同步/async双向共享、不同敌隔离、新敌不继承、子任务结束后保留、错误owner、完整i32槽检查及save/copy恢复续算。

- `cargo run --offline -q -p stg-harness -- gen-ecl-meta`：成功。
- `cargo test --offline -q -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl`：全部通过；core806、compiler337及集成4+3+1、harness54、RL49，doc tests通过。
- `cargo clippy --offline -q -p stg-core -p stg-ecl-compiler -p stg-harness -p stg-rl --all-targets -- -D warnings`：通过。
- `cargo fmt --all -- --check`、`git diff --check`：通过。
- `cargo run --offline -q -p stg-harness -- verify-tables`：全部表与已提交字节一致。
- `cargo run --offline -q -p stg-harness -- golden --out /tmp/stg-enemy-vars-v27-20261026.txt`：三段场景完成；SHA256 `8beb3e7e1872fcb1895d2b38e6ca4acad31831c96179618a304d2988731cb1d0`。新增全槽状态，预期与26版不同。
- 号表条目按1xx/4xx归组后，`cargo test --offline -q -p stg-core syscall_`：5项通过。

功能验收时未做跨平台对拍、原作动态对照、Godot或训练验证，未重建wheel，未提交或推送。前序整理和本地dist保留。

Sol high 最终审阅：规格合规与代码质量均通过，无待修项；确认生命周期、P4、完整状态处理及相关整理边界。阶段切换保留通过所有权和阶段源码检查确认，未额外重复增加端到端用例。

## 阶段性交付

2026-10-26用户授权“阶段性提交推送”：先以 `421f041` 单独提交模块整理（ENGINE_VER26），再提交本功能及ENGINE_VER27。推送目标为 `origin/codex/core-module-cleanup`，保留本地dist产物。
