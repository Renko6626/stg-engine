# 底层模块整理 Implementation Plan

> For agentic workers: 使用 superpowers:subagent-driven-development。用户明确指定 Sol medium 并行实现、Sol high 审阅；共享工作区按文件所有权隔离。

**Goal:** 完成 syscall 按域拆分、builtin 降低集中和符卡逻辑归位，保持可观察行为。

**Architecture:** 只拆私有文件与原样迁移实现，保留 crate 和公开 API。统一 dispatch、普通 codegen 与 step 相位继续作为原来的入口。

**Tech Stack:** 现有 Rust 1.94.0 工具链，无新依赖。

**Spec:** `docs/archive/specs/2026-10-26-core-module-cleanup-design.md`，对应用户本轮确认。

## Global Constraints

- 保持 ENGINE_VER=26、opcode/syscall 编号、公开路径与签名、World/Task/池布局、checksum/SaveBytes/copy 结果、烘焙表不变。
- 保持参数求值与弹栈顺序、返回值消费、P4 诊断和校验先后序、owner/任务/符卡生命周期与相位顺序。
- 不新增注册宏、通用降低模型、依赖、运行状态、原作能力或永久测试；不合并 sh_fire/batch。
- 三位实现者各有文件所有权，不独占仓库，不撤销他人修改；不改共享文档、不运行 cargo fmt --all、不提交/推送、不再派子代理。
- 实现者运行自己文件的 rustfmt；主代理在完整改动完成后集中运行已有检查，避免并行 Cargo 竞争和重复测试。
- 用户已批准路线并明确要求立即实施，计划不再增加确认步骤。采用当前目录的短期整理分支，保留 dist/ 本地产物；本轮不自动提交或推送。

## Review Focus

审阅原样迁移是否保留 syscall 校验/计数、builtin 参数与尾指令、符卡事件/清弹/记分顺序；检查 Rust 私有模块导致的名称遮蔽、路径变化和多余可见性。复用已有测试，不新增与实现镜像的测试。

### Task 1: syscall 按域拆分

**Ownership:** `crates/stg-core/src/ecl/syscall.rs` 及新增 `crates/stg-core/src/ecl/syscall/`。不得修改 world、spell 或 compiler。

**Interfaces:** `ecl::syscall::SYS_*`、`syscall_implemented`、`dispatch(no, task, ctx)` 保持原路径和签名。私有子模块复用父模块的栈/owner 助手，以最窄可见性供父派发调用。

- [x] 阅读现有独立函数、dispatch 分段及 tests，按已成立职责划分 shooter、laser、bullet、enemy、spell、interp 等私有实现；不为每个小函数造文件。
- [x] 公开号表/白名单/dispatch 保留根文件；先原样移动已有函数块，对大的内联派发块仅提取必要的同签名 helper，保持求值、诊断和返回顺序。
- [x] 将现有测试整体移到 `syscall/tests.rs`；保持测试名、断言、辅助逻辑，不造测试框架。
- [x] 检查 tests 对搬迁 helper 的访问，调整局部 import/最窄可见性；对拥有的文件 rustfmt，报告实际映射与风险。

### Task 2: 编译器 builtin 降低集中

**Ownership:** `crates/stg-ecl-compiler/src/lang/codegen.rs`、新增 `lang/codegen/builtin.rs`；只在必要时修改 `lang/builtins.rs` 中与 lowering 直接相关的简单 helper。不得修改 core。

**Interfaces:** `Gen` 保留一般生成器；子模块包含原 `gen_builtin_call` 和 builtin 专用 helper，采用 `impl Gen` 与最窄 `pub(super)` 可见性；Builtin 注册模型及输出 EclImage 不变。

- [x] 定位 self-bullet handle 求值丢弃、phase_begin 注入、die 尾指令、stage_clear wait、shape/color 折叠、SubRefArgs 参数处理。
- [x] 将 builtin 专用降低集中到 `codegen/builtin.rs`，继续调用父模块 `gen_expr` 等既有实现，不引入策略枚举/降低语言或跨 crate 号表。
- [x] 保持调用入口、参数求值顺序、独立字节码测试和生成产物；只调整必要测试引用。
- [x] 对拥有的文件 rustfmt，自审原样迁移和 import，报告新职责边界。

### Task 3: 符卡世界逻辑归位

**Ownership:** `crates/stg-core/src/world.rs`、`crates/stg-core/src/spell.rs`、新增 `crates/stg-core/src/world/spell.rs` 及其测试子文件。不得修改 ecl 或 compiler。

**Interfaces:** 根 `crate::spell` 的 POD 类型和常量路径不变；现有 WorldBody 方法公开签名不变，`spell_slot_bound_to` 对 sibling 相位仅给所需的最窄内部可见性。

- [x] 根 spell.rs 保留 SpellSlot、常量；将其 WorldBody 实现、decay_bonus/hp_ratio 以及 world.rs 的符卡查询/begin/end/结算方法移入 world/spell.rs。
- [x] 相关符卡行为测试随实现移动，保持原断言；WorldBody 字段和 settle 的伤害下钳及调用位置不变。
- [x] 修正搬迁直接相关的文档注释，特别是 settle_spells 注释位置；注意 world 子模块 `spell` 与 `crate::spell` 名称不同。
- [x] 对拥有文件 rustfmt，自审作用域和生命周期次序，报告迁移范围。

### Task 4: 集中验证与 Sol high 审阅

**Ownership:** 主代理负责计划、进度、架构导航文档及集成；审阅者只读。

- [x] 使用 baseline 270500c 与 `/tmp/stg-core-cleanup-before-20261026.txt`；已有同基线相关测试通过，不重复跑基线全量。
- [x] 完整改动后运行 `cargo fmt --all -- --check`、`git diff --check`、`cargo test --offline -q -p stg-core -p stg-ecl-compiler -p stg-harness`。
- [x] 运行相关三包 `cargo clippy --offline --all-targets -- -D warnings`、`verify-tables`，生成 after golden 并与 before 逐字节比较。
- [x] 三位 Sol high 分别审阅 syscall、compiler、world；syscall 审阅同时核对两条跨任务接口，核对任务规格和实际差异，不重复已有检查，不扩充理论需求。若有具体问题，交原实现者局部修正和针对性复核。
- [x] 视合并结果需要进行一次跨任务边界审阅，避免重复全面审阅；更新架构导航与 PROGRESS，报告实际命令、结果和未验证项。不自动提交/推送。

## 集中验证记录（2026-10-26）

- `cargo test --offline -q -p stg-core -p stg-ecl-compiler -p stg-harness`：core 805、compiler 337、插值集成3、reseed集成1、harness54及doc-test全部通过。迁移后发现的一个未使用测试导入已删除，断言未动。
- `cargo fmt --all -- --check`、`git diff --check`、`cargo clippy --offline -q -p stg-core -p stg-ecl-compiler -p stg-harness --all-targets -- -D warnings`：通过，最终无警告。
- `cargo run --offline -q -p stg-harness -- verify-tables`：四份表通过。
- 改前/改后分别 `cargo run --offline -q -p stg-harness -- golden --out /tmp/stg-core-cleanup-{before,after}-20261026.txt`，`cmp`逐字节一致，SHA256均为 `75d6a0e3dbff13e7ccb736f5240142ca0407a293d61cd8adc87cc129bd50d706`。
- 定点表、ENGINE_VER、Task/ops/step/save/checksum及伤害结算文件与基线270500c无差异。WorldBody字段与SpellSlot由审阅逐项核对未变。
- 未新增永久测试，未运行浏览器、Godot、训练或跨平台对拍，未重建wheel；这些不在本次私有模块整理范围。未自动提交或推送。

## 审阅与最终交接

三位Sol medium按文件所有权并行实现，三位Sol high分别审阅syscall、builtin lowering、world符卡；规格合规与代码质量均通过，无待修项。syscall审阅同时核对compiler→syscall与syscall spell→WorldBody两条跨任务接缝。原函数、提取的内联handler与既有测试经静态对照保持；一项未使用测试导入已由原实现者删除，并由审阅者核对最终差异，最终clippy无警告。

未引入新框架或登记机制；仅私有文件、必要的最窄可见性与import调整。主代理更新CLAUDE、架构导航及PROGRESS。最终状态保留在codex/core-module-cleanup分支，未提交/推送；无额外实施决策等待确认。
