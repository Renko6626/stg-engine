# M0-14 create_bullets_batch 批量发射器 —— 设计 spec

> 状态：已过 grill 拍板，待实施计划。定位：**性能语义原语**——ECL 循环逐颗发弹要付
> 解释器 + syscall 分发 + 取参的每颗税，世界侧一次调用摊平成 Rust 紧循环；密集符卡
> （60-way 环、多重列）是数量级差异。上游：D12 syscall 成员清单（`create_bullets_batch`）、
> 路线甲（创建原语不内建 RNG/聪明数学）。

## 拍板纪要（grill，2026-07-17）

1. **网格 API 通吃**：环（角度插值）/列（速度插值）/多重环（双轴）是同一 N×K 网格的特例，
   一个 API、ECL 一个 syscall 号。**迭代序 = 角度外层、速度内层 = 池槽分配序**（I4 契约，
   测试钉住）。
2. **步长直给（原语派）**：`angle_step`/`speed_step` 直接进参；张角/端点推导属作者层语法糖
   ——**M1 ECL DSL 待办**（`ring(n)`/`fan(spread, n)` 编译期算步长喂原语，舍入责任在脚本侧）。
3. **超量整体拒 + 额度内尽力而为**：`n_angle == 0 ∨ n_speed == 0 ∨ N×K > BulletPool::CAP(8192)`
   → `BAD_ARGS` 整体拒绝（永不可能全额成功的请求 = 作者 bug，且防确定性烧机）；额度内
   池/段满 → **尽力而为 + 逐颗计数**（半个环是降级不是错误，per-bullet P4-a 与 `create_bullet`
   一致）；xform 坏序列 → **开跑前验一次、整体 BAD_ARGS**（宁缺勿哑管单颗语义完整性，
   验证不逐颗重复）。

## API

```rust
pub fn create_bullets_batch(
    &mut self,
    init: BulletInit,          // 模板：x/y/radius/sprite/delay/life/flags 共享；
                               // speed/angle/vx/vy 被逐颗覆写；transform_head 恒被覆写
    xform: &[XformSlot],       // 空切片 = 哑弹批；非空 = 每颗自有段拷贝（段消耗 = N×K！）
    n_angle: u16, angle0: Angle, angle_step: i16,   // 角度轴：BAM 累加回绕
    n_speed: u16, speed0: Fx, speed_step: Fx,        // 速度轴：Fx 累加
) -> u16                       // 实发数
```

- **前置验证（一次）**：轴零/超量 → BAD_ARGS 拒（`contract_viol` + `last_status`，实发 0）；
  `xform` 非空时跑完整 arity 走格验证（与 `create_bullet_with_xform` **共用同一验证助手**
  ——从后者抽私有 `fn validate_xform(...)`，纯重构行为零变化）；模板 `radius` 钳制一次
  （越界计一次 `contract_viol`，P4-b 与四写 API 同律）。
- **两轴累加器**（非乘法）：外层 `cur_angle = angle0`，每圈 `add_delta(angle_step)`（BAM
  回绕天然正确——环过 65536 自动闭合）；内层 `cur_speed` 从 `speed0` 起每步 `+ speed_step`
  （Fx 裸加，溢出属 P4-c 域 debug 断言兜底——作者责任，与 `ADD_SPEED` 同律）。累加器整数
  精确等价于乘法且无溢出中间量。
- **逐颗**：`(vx, vy) = polar_to_vec(cur_speed, cur_angle)`（白名单乘法）；哑弹批走
  `create_bullet` 等价路径（`transform_head = XFORM_NONE` 覆写）、xform 批走
  `create_bullet_with_xform` 等价路径（先段后弹 + 该颗整体失败语义）。
- **满额短路**：首次 alloc 失败（池或段）即 break——同一相位内无回收，后续颗必然同败；
  剩余颗数**一次性**计入对应 `pool_full` 计数器 + `last_status = POOL_FULL`（确定性上
  严格等价于逐颗试，省 O(剩余×池扫描) 的空转）。
- **无 RNG**（路线甲：生成器纯确定；要散布让 ECL 逐颗调 `create_bullet` 或未来加带散布变体）。

## 段消耗账（文档义务）

xform 批一次吃 **N×K 个段**（段池 cap 2048）：60-way 三重环带变换 = 180 段。API docstring
与 `docs/xform-ops.md` 消费入口节各写一句成本账；段满按尽力而为降级（计 `pool_full[XFORM]`）。

## 金向量与测试

- **金向量三压力源**：32-way 哑弹环（角度累加回绕整圈）· 5 重速度列（1.0→3.0 步进 0.5）·
  3×4 网格带两槽 xform 序列（段消耗路径入流）。双跑照旧。
- **判别式单测**：网格逐颗 `vx/vy == polar_to_vec(参考)` 逐位（非平凡 angle0/step 摆位）/
  **迭代序 = 槽序**（角度外速度内——槽 k 的参数可反推）/ 环回绕（angle 累加跨 65536 闭合）/
  池满尽力而为（预占池到只剩 M < N×K，断言实发 M + 计数 N×K−M）/ 超量与零轴拒绝（实发 0 +
  BAD_ARGS + 零副作用）/ xform 批每颗自有段 + 坏序列整体拒 / 模板 radius 钳制恰计一次。
- **变异检验 ≥3 候选**：迭代序对调（速度外角度内）→ 槽序测试红；角度累加改饱和不回绕 →
  环闭合测试红；超量检查删除 → 超量测试红。

## 收尾义务

`stg-world-design.md` D12/§4.4 附近回写一句（batch 原语落地 + 张角糖归 ECL DSL）；
`docs/xform-ops.md` 消费入口节补 batch 一行含段消耗账；`PROGRESS.md` 史行 + 现在段
（下一步候选：bomb · SPAWN_PATTERN+图样表 · M1 ECL）；follow-ups 如产生延后项入库。

## 验收

判别式单测全绿且过变异检验；金向量三平台互比全等；clippy/fmt 零告警；无新 World 字段。
