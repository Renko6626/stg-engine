# stg-py —— `stg_rl`（stg-engine 批量 RL env 的 Python wheel）

PyO3 abi3 薄壳，包在 [`stg-rl`](../stg-rl/) 外。单进程：Rust 专属 rayon 池批量 `step`，写入调用方
持有的缓冲，期间释放 GIL。设计见 `docs/superpowers/specs/2026-09-15-stg-rl-env-design.md`。

> **不要**再套 `gymnasium.AsyncVectorEnv` / SB3 `SubprocVecEnv` / `make_vec_env`——那会把整个 env
> 复制进子进程、吃掉全部性能。训练侧请设 `torch.set_num_threads`，避免与 rayon 抢核。

## 安装

CI 在推 `rl-v*` tag 时出 wheel 挂 Release（`.github/workflows/wheels.yml`）：

```bash
pip install https://github.com/Renko6626/stg-engine/releases/download/rl-v<ver>/stg_rl-<ver>-cp310-abi3-manylinux_2_28_x86_64.whl
```

本机源码构建（独立 workspace，不进主 workspace）：

```bash
cd crates/stg-py
python3 -m venv .venv && . .venv/bin/activate
pip install maturin numpy pytest
maturin build --release -o dist
pip install --force-reinstall --no-deps dist/stg_rl-*.whl
cd .venv && python -m pytest -q -p no:cacheprovider ../tests
```

## 公开接口

```python
import stg_rl

img = stg_rl.compile_bundled("game")            # 内置 godot/ecl/game/*.ecl；或 compile_dir/compile_sources
buf = stg_rl.alloc_buffers(num_envs=512, bullets_cap=1024, backend="torch", pin=True)
env = stg_rl.VecEnv(512, 32, {"game": img}, [stg_rl.Start("game", mark=15)], bullets_cap=1024, seed=0)
env.reset()
env.step(actions)                                # actions: uint32[N]
env.set_start_weights([...])                     # 课程学习
stg_rl.hello(bullets_cap=1024)                   # proto v1 HELLO JSON
stg_rl.OFFSETS, stg_rl.STRIDES, stg_rl.EVENT_COLUMNS
stg_rl.build_info()                              # {"version","engine_ver","tables_hash","git_sha"}
```

- 编译错误抛 `stg_rl.CompileError`（消息含文件名 / 行列）；配置 / 缓冲形状错误抛 `ValueError`，
  **在构造期**抛，不在 `step` 里抛。
- 缓冲一律**一维**（多维形状由 Python 包装层 `reshape` 出视图），Rust 只见扁平切片。
  `test_smoke.py` 用 `stgagent.schema` 的 dtype 解码 CSR 行，与按 `OFFSETS` 手切结果对拍。
