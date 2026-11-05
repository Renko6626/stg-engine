# 补齐语义缓动曲线实施计划

> 使用 superpowers:executing-plans，在当前 main 执行；用户要求不开分支、不自动提交或推送。

**目标：** 按用户已确认范围，将现有8条曲线扩为30条；不实现TH18 mode7/8/17的状态型插值。

**架构：** 复用 `Easing` 枚举、ECL常量注册表和257点Q16.16烘焙表。0–7及其表字节保留，
新增曲线追加8–29；合法值判断从枚举集中取得。保持World布局、指令签名和模块边界。

**技术栈：** 现有Rust工具链，无新依赖。浮点仅用于harness烘焙；core继续使用定点。

**依据：** [三方语义表](../../easing-reference.md)，用户确认“先补齐现有的”，其余三种另做功能／函数。

## 全局约束

- 保留现有数值编码；新增合法值改变此前坏参数结果，ENGINE_VER24→25。
- P4坏参数no-op并计数；不改已有API钳制、实体布局、调度或现有卡池轨迹。
- 超调允许负值／超1；保持／跳变通过插值器的首次推进和终帧语义兑现。
- 不新增源作品到目标编号的数字映射；源表仍按语义名称表达。
- 不重建训练wheel、不迁移既有训练卡池或启动训练；使用新曲线前需另同步wheel。

## 验证重点

- 四次曲线与三次曲线不同，OUT_IN与IN_OUT不同：独立数学锚点覆盖每条新增曲线。
- 反向／超调经过真实位置、速度与角度插值仍保留，不被隐藏钳制。
- HOLD保持中途起点、终帧落目标；STEP_START首次推进跳至目标，指令发出时不抢跑。
- 弹STEP接受新编号；非法编号在单发／批量／运动入口仍拒绝。
- 旧8条表前缀、旧场景golden与原先一致；新ECL名称完整可编译。

## Task 1：曲线、入口与名称

文件：`crates/stg-core/src/math/easing.rs`、`consts.rs`、`lib.rs`、
`world.rs`、`world/motion.rs`、`world/integrate.rs`；
`crates/stg-harness/src/tables.rs`；`crates/stg-core/src/math/tables/easing.bin`。

- [x] 保存旧表与旧场景golden；为新增曲线数学锚点和运动／STEP边界写短用例，确认失败。
- [x] 追加22条枚举，提供 `Easing::ALL`、`Easing::from_id(u8)->Option<Easing>`；运行期保留内部fallback。
- [x] 烘焙按枚举遍历、按语义匹配公式；追加数据，不钳曲线输出。
- [x] 三处World曲线校验改用同一枚举判定；新增22个ECL语义常量；ENGINE_VER升25。
- [x] 更新原本把8当坏值的用例，使其继续检查真正无效的值；不删除断言或放宽结果。

## Task 2：文档与集中验证

文件：`docs/easing-reference.md`、`docs/ecl-lang/{3-enemy,7-reference}.md`、
`docs/xform-ops.md`、`PROGRESS.md`；训练仓 `transcribe/th06/mapping.md` 和项目记忆。

- [x] 三方表改为实际支持状态，填入追加编号；TH06四次映射使用准确名称，历史旧卡不改。
- [x] 扩展既有名称编译用例；运行core、compiler、harness测试、fmt、clippy、verify-tables。
- [x] 比较旧表前缀与改前后golden；一次独立审阅限定本批改动，按用户约束分诊发现。
- [x] 记录实际命令、结果与未验证项；保留其他未提交工作，不提交或推送。

## 执行与验证记录（2026-10-23）

- 新增数学锚点、真实运动与弹STEP用例先在旧实现失败，名称编译用例在新增名称前失败；接入后通过。
- `cargo test --offline -p stg-core -p stg-ecl-compiler -p stg-harness`：通过。初轮仅版本锚点仍钉24失败，
  已将精确断言改为25并补上升版理由；没有放宽断言。集中重跑通过。
- 独立只读审阅无阻碍交付问题；修正一处旧文档范围，并复用集成用例增加负权重案例，
  `cargo test --offline -p stg-core extended_` 通过。没有扩大产品机制或再做重复全量审阅。
- `cargo fmt --all -- --check`、`cargo clippy --offline -p stg-core -p stg-ecl-compiler -p stg-harness --all-targets -- -D warnings`：通过。
- 旧表8224字节前缀对比通过；新表30840字节。烘焙bootstrap仅暂补空行以编译既有生成器，
  随即由 `cargo run --offline -p stg-harness -- bake-tables` 覆写成真实表，空行未用于验收。
- 未做跨平台运行、原游戏实机对拍、训练、wheel重建、卡池迁移、提交或推送。
- `cargo run --offline -p stg-harness -- verify-tables`：四份烘焙表全部通过。
- `cargo run --offline -p stg-harness -- golden --out /tmp/stg-easing30-before.txt` 和
  `cargo run --offline -p stg-harness -- golden --out /tmp/stg-easing30-after.txt` 分别在改前后运行；
  `cmp /tmp/stg-easing30-before.txt /tmp/stg-easing30-after.txt` 通过，SHA256均为
  `599e53fd102fbbfcb398822e920cd3aec4cd2cfea1fea8b56ebf17af1ebdda86`。
