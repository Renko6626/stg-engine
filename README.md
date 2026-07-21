# stg_engine

Godot 前端 + Rust 确定性内核的 **2D 弹幕 STG 引擎**。一等目标：rollback 联机、逐帧回放、
headless 高速模拟。核心性质是**跨平台 bit 级确定性**。

> 设计与开发纪律见 [`CLAUDE.md`](./CLAUDE.md)、[`design_doc.md`](./design_doc.md)（总纲 v0.3）、
> [`stg-world-design.md`](./stg-world-design.md)（世界层蓝图 v1.0）。

## 架构一句话

五层一断层线：表现层（Godot / headless 消费者）在断层线**以上**；确定性模拟核 `world` + ECL VM
在**以下**。向下唯一入口是每帧一份 `InputFrame`，向上只有两个出口：状态内存视图（通道 A）与
渲染请求队列（通道 B）。断层线由 Cargo 依赖图编译期焊死——`stg-core` 不依赖 godot/时钟/浮点/RNG。

```
step: world[n+1] = step(world[n], static_ecl, input_frame[n])
```

**完整子系统地图 / 断层线 / 相位流水线 / 未来接缝见 [`docs/architecture.md`](./docs/architecture.md)。**

## 当前状态

Phase 1 = `stg-core` + `stg-ecl-compiler` + `stg-harness`。DoD = 金向量在 x86_64 与 aarch64 上
逐帧校验和一致——**这条流水线从第一天起就是绿的，每个切片都过三平台对拍**。

当前位置 / 下一步 / 里程碑史见 [`PROGRESS.md`](./PROGRESS.md)（**唯一权威**，本节不复述以免漂移）。

```
crates/
  stg-core/          确定性内核（断层线以下）
  stg-derive/        proc-macro：#[derive(Checksum)] + define_pool!
  stg-ecl-compiler/  离线 ECL 字节码编译器与表层语言（M1/M1.9）
  stg-harness/       CLI：金向量对拍 + 烘焙表 bake/verify
```

## 专题文档

| 文档 | 内容 |
|---|---|
| [`docs/architecture.md`](./docs/architecture.md) | **子系统地图**：五层断层线 / 七不变量 / crate 依赖图 / step 相位 0-10 / 未来接缝（M2-M5） |
| [`docs/ecl-lang.md`](./docs/ecl-lang.md) · [`docs/ecl-ops.md`](./docs/ecl-ops.md) | `.ecl` 表层语言手册 / 字节码层速查（op/syscall/fault 码） |
| [`docs/fixed-point-corners.md`](./docs/fixed-point-corners.md) | `Fx`/`Angle` 的坑与规范速查（`Q(m).f × Q(m).f = Q(2m).(2f)`、累加器模式、Angle 回绕） |
| [`docs/checksum-mechanism.md`](./docs/checksum-mechanism.md) | 校验和机制 + "新字段默认入校验"的保证是怎么来的 |
| [`docs/pool-memory-layout.md`](./docs/pool-memory-layout.md) | 池 SoA 布局与缓存精算（热路径驻 L2） |
| [`docs/xform-ops.md`](./docs/xform-ops.md) | 弹变换 op 速查表（编号/效果/参数语义 + LOOP 等关键坑） |
| [`docs/superpowers/specs/`](./docs/superpowers/specs/) · [`plans/`](./docs/superpowers/plans/) | 各切片的设计 spec 与实施计划（历史记录） |

## 构建与测试

需要 Rust `1.92.0`（由 `rust-toolchain.toml` 自动装）。

```bash
cargo build --workspace
cargo test  --workspace
cargo run -p stg-harness -- golden        # 跑金向量，逐帧输出校验和
```

CI（`.github/workflows/ci.yml`）在 Windows-x86_64 / Linux-x86_64 / Linux-aarch64 三平台构建、
测试，并断言金向量逐帧校验和**逐字节全等**——把跨平台确定性当作持续验证的性质。
