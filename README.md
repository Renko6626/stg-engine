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

## 当前状态：Phase 1 脚手架

Phase 1 = `stg-core` + `stg-ecl-compiler` + `stg-harness`。DoD = 金向量在 x86_64 与 aarch64 上
逐帧校验和一致。当前仅落地确定性契约最底层（vendored FNV-1a 校验和）与端到端 CI 流水线；
数学核 / 池 / World / step 随 **M0** 逐模块 TDD 落地。

```
crates/
  stg-core/          确定性内核（断层线以下）
  stg-derive/        proc-macro：#[derive(Checksum)]
  stg-ecl-compiler/  离线 ECL 字节码编译器（M1）
  stg-harness/       CLI：金向量对拍 + 烘焙表 bake/verify
```

## 构建与测试

需要 Rust `1.92.0`（由 `rust-toolchain.toml` 自动装）。

```bash
cargo build --workspace
cargo test  --workspace
cargo run -p stg-harness -- golden        # 跑金向量，逐帧输出校验和
```

CI（`.github/workflows/ci.yml`）在 Windows-x86_64 / Linux-x86_64 / Linux-aarch64 三平台构建、
测试，并断言金向量逐帧校验和**逐字节全等**——把跨平台确定性当作持续验证的性质。
