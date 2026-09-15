# stg-rl-train 第一刀 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. 任务间审阅从简；合并前全分支大审阅由控制者亲自做（用户规矩）。

**Goal:** 新建训练仓 `stg-rl-train`，交付「任意有 GPU 的 Linux 机器上 `bash run.sh <config>` 一条命令完成装依赖 → 训练 → 评测 → 出图 → 打包」的躲弹小模型 PPO 训练。

**Architecture:** `stg_rl` wheel 提供批量 env 与原始 proto 缓冲；训练仓的胶水层（`envwrap` 解码 + 意图 + 镜像、`featurize` 危险度 top-K、`reward` 分项）把缓冲变成 GPU 特征与 reward；模型、特征化器、意图生成器、reward 项都走注册表按配置选名；`ppo.py` 以 LeanRL `ppo_atari_envpool_torchcompile.py` 为底本；`metrics`/`plots`/`perf`/`evaluate`/`checkpoint` 各司一职；`train.py` 编排，`run.sh` 只负责 `uv sync --frozen` 后调用它。

**Tech Stack:** Python 3.12、uv 0.11、torch 2.14.0、tensordict 0.14.2（只用 `CudaGraphModule` / `TensorDict` / `TensorDictModule` / `from_module`）、stg_rl 0.1.0（`rl-v0.1.0` Release wheel）、stgagent 0.1.0（git tag）、numpy、matplotlib、psutil、nvidia-ml-py、tensorboard、pytest。

**Spec:** `docs/superpowers/specs/2026-09-15-stg-rl-train-design.md`（stg-engine 仓；Task 13 复制进训练仓 `docs/`）

## Global Constraints

- 训练仓路径 `/data/sunyunbo/www/stg-rl-train`，GitHub `Renko6626/stg-rl-train`（public）。本计划文档住 stg-engine 仓，代码全部写在训练仓。
- Python `>=3.12,<3.13`（`.python-version` = `3.12`）；依赖版本钉死：`torch==2.14.0`、`tensordict==0.14.2`；`stg-rl @ https://github.com/Renko6626/stg-engine/releases/download/rl-v0.1.0/stg_rl-0.1.0-cp310-abi3-manylinux_2_28_x86_64.whl`；`stgagent @ git+https://github.com/Renko6626/stg-agent-proto@v0.1.0`。
- 一律 `uv sync --frozen` / `uv run --frozen`；改依赖 = 改 `pyproject.toml` + `uv lock` + 单独提交。
- 坐标系：场界 x ∈ [-192, 192]，y ∈ [0, 448]，y 向下；fx 字段 = little-endian int32 / 65536。
- 缓冲字段偏移一律取 `stg_rl.OFFSETS[表][字段][0]`，不写魔数；键位一律取 `stgagent.consts.BTN_*`。
- 动作表 v1：`action_id = 方向 × 2 + slow`，方向 0 不动 1 上 2 右上 3 右 4 右下 5 下 6 左下 7 左 8 左上；镜像 `(0,1,8,7,6,5,4,3,2)`；SHOT 恒按；BOMB 永不置位。`ACTION_TABLE_VERSION = 1`。
- done 码：0 继续 / 1 死亡 / 2 段落结束 / 3 超时截断；`done != 0` 时缓冲已是新局首帧。
- 测试全部在 CPU 上可跑（`device="cpu"`），不得依赖 GPU；测试卡只用 `tests/fixtures/cards/`。
- 模型前向输入是 `Mapping[str, Tensor]`（运行时传 `TensorDict`），输出 `(logits[N,18], value[N])`；形状固定（CUDA 图前提）。
- 每个 Task 结束：`uv run --frozen pytest -q` 全绿再提交。
- 清理临时/变异代码只用编辑撤回，**禁止 `git checkout <file>` / `git stash`**（本机已被咬两次）。
- commit 结尾：`Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`。

## Rulings（写计划时裁定，偏离 spec 处；Task 13 回写 spec §13）

1. **刷新步的遵从塑形不置 0，改为用旧指令点精确计算**：reward 用 `prev.target_xy`（这一步动作所瞄的点）同时算前后两个距离，刷新发生在 reward 之后，所以没有「目标突变」的跳变，spec 担心的问题不存在；只保留「终局步记 0」。单测断言改为「一局内（γ=1）塑形总和 = 各刷新段 Φ 差之和」。
2. **到达用时**：每局记「各刷新段到达帧数的均值」（未到达记 `interval` 上限），评测汇总取各局值的中位数。
3. **新增三个文件**（spec §2 未列）：`cards.py`（卡池发现 / 元数据 / 评测划分 / 起点）、`episodes.py`（逐局统计累加器，训练与评测共用）、`bench.py`（`--bench` 实现，避免 `perf.py` 依赖全链路）。
4. **打包在 `train.py` 里用 `tarfile` 做**（`--no-pack` 可关），`run.sh` 只做装依赖 + 调入口，便于测试。
5. **评测按 (卡, rank) 各开一个 VecEnv**，`num_envs = episodes`，每个 env 只取第一局，保证每组恰好 `episodes` 局。
6. **不加 LICENSE**（stg-engine 同样没有）；`ppo.py` 文件头保留 LeanRL 的 MIT 许可全文。
7. **续训时 env 种子 = `run.seed + 起始更新号 − 1`**，避免重放同一批局；这是 spec §5 已声明的「非逐字节续训」。

## File Map（全部在 `stg-rl-train/`）

| 文件 | 职责 | Task |
|---|---|---|
| `pyproject.toml` `uv.lock` `.python-version` `.gitignore` `README.md` `.github/workflows/ci.yml` | 仓库骨架、依赖、CI | 1 |
| `src/stgtrain/__init__.py` `registry.py` `config.py` | 注册表、配置默认值 / 读写 / 校验 | 1 |
| `src/stgtrain/cards.py` `tests/fixtures/cards/*` `tests/fixtures/eval_splits.toml` `tests/conftest.py` | 卡池与测试夹具 | 2 |
| `src/stgtrain/actions.py` `intent.py` | 动作表 v1、意图生成器 | 3 |
| `src/stgtrain/envwrap.py` | VecEnv 包装：缓冲 → `RawObs` / `StepInfo` | 4 |
| `src/stgtrain/featurize/__init__.py` `featurize/danger_topk_v1.py` | 特征化器 | 5 |
| `src/stgtrain/reward.py` `episodes.py` | reward 项、逐局统计 | 6 |
| `src/stgtrain/models/__init__.py` `models/set_attn_v1.py` | 首个模型 | 7 |
| `src/stgtrain/metrics.py` `plots.py` | jsonl + TensorBoard、出图 | 8 |
| `src/stgtrain/perf.py` | 分阶段计时、负载采样、机器画像 | 9 |
| `src/stgtrain/ppo.py` `checkpoint.py` | LeanRL 底本 PPO、存读 | 10 |
| `src/stgtrain/evaluate.py` | 固定评测集 | 11 |
| `src/stgtrain/train.py` `bench.py` `run.sh` `configs/base.toml` `configs/smoke.toml` `cards/.gitkeep` `eval/splits.toml` | 编排、bench、入口、配置 | 12 |
| `docs/`（训练仓）+ stg-engine `PROGRESS.md` / spec §13 | 文档收口 | 13 |

---

### Task 1: 仓库骨架 —— 依赖、注册表、配置

**Files:**
- Create: `pyproject.toml`, `.python-version`, `.gitignore`, `README.md`, `.github/workflows/ci.yml`
- Create: `src/stgtrain/__init__.py`, `src/stgtrain/registry.py`, `src/stgtrain/config.py`
- Test: `tests/test_registry.py`, `tests/test_config.py`

**Interfaces:**
- Produces:
  - `stgtrain.registry.Registry(kind: str)`：`.register(name) -> decorator`、`.get(name) -> object`（未知名 `ValueError`，消息列出已注册名）、`.names() -> list[str]`
  - 全局实例 `MODELS`、`FEATURIZERS`、`INTENTS`、`REWARD_TERMS`
  - `BUILTIN_MODULES: list[str]`（后续 Task 往里追加模块名）、`load_builtins() -> None`（逐个 `importlib.import_module`）
  - `check_compat(requires: dict[str, tuple[int, ...]], spec: dict[str, tuple[int, ...]]) -> None`（缺键或形状不符 `ValueError`）
  - `stgtrain.config.DEFAULTS: dict`、`deep_merge(base: dict, over: dict) -> dict`、`validate(cfg: dict) -> None`、`load_config(path: str | Path, overrides: dict | None = None) -> dict`、`from_dict(d: dict) -> dict`（= merge DEFAULTS + validate）、`dump_toml(cfg: dict, path: str | Path) -> None`

- [ ] **Step 1: 建目录、git init、Python 版本、忽略文件**

```bash
mkdir -p /data/sunyunbo/www/stg-rl-train && cd /data/sunyunbo/www/stg-rl-train
git init -b main
echo "3.12" > .python-version
mkdir -p src/stgtrain tests .github/workflows
```

`.gitignore`：

```gitignore
.venv/
__pycache__/
*.pyc
.pytest_cache/
runs/
*.tar.gz
```

- [ ] **Step 2: 写 `pyproject.toml` 并锁依赖**

```toml
[project]
name = "stgtrain"
version = "0.1.0"
description = "stg-engine 躲弹小模型训练仓（PPO，LeanRL 底本）"
requires-python = ">=3.12,<3.13"
dependencies = [
  "stg-rl @ https://github.com/Renko6626/stg-engine/releases/download/rl-v0.1.0/stg_rl-0.1.0-cp310-abi3-manylinux_2_28_x86_64.whl",
  "stgagent @ git+https://github.com/Renko6626/stg-agent-proto@v0.1.0",
  "torch==2.14.0",
  "tensordict==0.14.2",
  "numpy",
  "matplotlib",
  "psutil",
  "nvidia-ml-py",
  "tensorboard",
]

[dependency-groups]
dev = ["pytest"]

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[tool.hatch.metadata]
allow-direct-references = true

[tool.hatch.build.targets.wheel]
packages = ["src/stgtrain"]

[tool.pytest.ini_options]
testpaths = ["tests"]
```

Run: `uv lock && uv sync --frozen`
Expected: `uv.lock` 生成；`uv run --frozen python -c "import stg_rl, stgagent.consts, torch, tensordict; print(stg_rl.build_info()['version'], torch.__version__)"` 输出 `0.1.0 2.14.0+...`（或 `2.14.0`）。

`src/stgtrain/__init__.py`：

```python
"""stgtrain —— stg-engine 躲弹小模型训练仓（spec: stg-engine docs/superpowers/specs/2026-09-15-stg-rl-train-design.md）。"""
```

- [ ] **Step 3: 写注册表的失败测试**

`tests/test_registry.py`：

```python
import pytest

from stgtrain.registry import Registry, check_compat


def test_register_and_get():
    r = Registry("thing")

    @r.register("a")
    class A:
        pass

    assert r.get("a") is A
    assert r.names() == ["a"]


def test_duplicate_and_unknown_raise():
    r = Registry("thing")
    r.register("a")(object)
    with pytest.raises(ValueError, match="重复"):
        r.register("a")(object)
    with pytest.raises(ValueError, match="a"):
        r.get("nope")


def test_check_compat():
    spec = {"bullets": (64, 7), "cond": (4,)}
    check_compat({"bullets": (64, 7)}, spec)
    with pytest.raises(ValueError, match="缺"):
        check_compat({"enemies": (8, 6)}, spec)
    with pytest.raises(ValueError, match="形状"):
        check_compat({"bullets": (32, 7)}, spec)
```

Run: `uv run --frozen pytest tests/test_registry.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.registry`）

- [ ] **Step 4: 实现 `registry.py`**

```python
"""注册表：模型 / 特征化器 / 意图生成器 / reward 项按配置里的名字选用（spec §4）。"""
from __future__ import annotations

import importlib
from typing import Callable, TypeVar

T = TypeVar("T")


class Registry:
    def __init__(self, kind: str):
        self.kind = kind
        self._items: dict[str, object] = {}

    def register(self, name: str) -> Callable[[T], T]:
        def deco(obj: T) -> T:
            if name in self._items:
                raise ValueError(f"{self.kind} 注册名重复：{name!r}")
            self._items[name] = obj
            return obj

        return deco

    def get(self, name: str):
        if name not in self._items:
            raise ValueError(f"未知的 {self.kind} {name!r}；已注册：{self.names()}")
        return self._items[name]

    def names(self) -> list[str]:
        return sorted(self._items)


MODELS = Registry("model")
FEATURIZERS = Registry("featurizer")
INTENTS = Registry("intent")
REWARD_TERMS = Registry("reward term")

# 各内置实现所在模块；后续 Task 逐个追加。import 即注册。
BUILTIN_MODULES: list[str] = []


def load_builtins() -> None:
    for mod in BUILTIN_MODULES:
        importlib.import_module(mod)


def check_compat(requires: dict[str, tuple[int, ...]], spec: dict[str, tuple[int, ...]]) -> None:
    """模型要的特征键必须都由特征化器提供，且形状（去掉 batch 维）一致。"""
    for key, shape in requires.items():
        if key not in spec:
            raise ValueError(f"模型要的特征 {key!r} 特征化器没提供（缺）；特征化器提供：{sorted(spec)}")
        if tuple(spec[key]) != tuple(shape):
            raise ValueError(f"特征 {key!r} 形状不符：模型要 {tuple(shape)}，特征化器给 {tuple(spec[key])}")
```

Run: `uv run --frozen pytest tests/test_registry.py -q`
Expected: PASS

- [ ] **Step 5: 写配置的失败测试**

`tests/test_config.py`：

```python
import copy

import pytest

from stgtrain.config import DEFAULTS, deep_merge, dump_toml, from_dict, load_config, validate


def test_defaults_are_valid():
    validate(copy.deepcopy(DEFAULTS))


def test_deep_merge_keeps_unmentioned_keys():
    out = deep_merge(DEFAULTS, {"reward": {"terms": {"hold": 0.5}}})
    assert out["reward"]["terms"]["hold"] == 0.5
    assert out["reward"]["terms"]["death"] == DEFAULTS["reward"]["terms"]["death"]
    assert DEFAULTS["reward"]["terms"]["hold"] == 0.01, "不得改动 DEFAULTS 本身"


@pytest.mark.parametrize(
    "over, msg",
    [
        ({"env": {"nmu_envs": 4}}, "未知配置键"),
        ({"env": {"bullets_cap": 0}}, "bullets_cap"),
        ({"featurize": {"k_bullets": 2048}}, "k_bullets"),
        ({"intent": {"interval": [300, 120]}}, "interval"),
        ({"run": {"device": "gpu"}}, "device"),
        ({"env": {"num_envs": 6}, "ppo": {"num_steps": 5, "num_minibatches": 4}}, "num_minibatches"),
        ({"env": {"ranks": [5]}}, "ranks"),
    ],
)
def test_validate_rejects(over, msg):
    with pytest.raises(ValueError, match=msg):
        from_dict(over)


def test_toml_roundtrip(tmp_path):
    cfg = from_dict({"model": {"d": 32}, "env": {"ranks": [0, 4]}})
    p = tmp_path / "c.toml"
    dump_toml(cfg, p)
    assert load_config(p) == cfg


def test_load_config_overrides(tmp_path):
    p = tmp_path / "c.toml"
    p.write_text('[run]\nseed = 7\n', encoding="utf-8")
    cfg = load_config(p, overrides={"run": {"total_updates": 3}})
    assert cfg["run"]["seed"] == 7 and cfg["run"]["total_updates"] == 3
```

Run: `uv run --frozen pytest tests/test_config.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.config`）

- [ ] **Step 6: 实现 `config.py`**

```python
"""配置：默认值 + TOML 读写 + 校验（spec §4 / §5）。reward 项名的合法性由 reward.RewardFn 校验。"""
from __future__ import annotations

import copy
import json
import tomllib
from pathlib import Path

DEFAULTS: dict = {
    "run": {"seed": 1, "device": "auto", "total_updates": 2000, "ckpt_every": 50, "eval_every": 50,
            "torch_threads": 2},
    "env": {"cards_dir": "cards", "eval_splits": "eval/splits.toml", "num_envs": 2048, "threads": 0,
            "frame_skip": 1, "max_frames": 3600, "warmup_max": 120, "bullets_cap": 1024, "ranks": [2],
            "mirror": True},
    "intent": {"name": "lower_half_uniform_v1", "margin": 16.0, "interval": [120, 300]},
    "featurize": {"name": "danger_topk_v1", "k_bullets": 64, "k_enemies": 8, "horizon": 60, "d_max": 128.0},
    "model": {"name": "set_attn_v1", "d": 64, "heads": 4, "trunk": 256},
    "reward": {"hold_radius": 24.0, "edge_margin": 16.0,
               "terms": {"death": 10.0, "follow_shaping": 1.0, "hold": 0.01, "segment_survived": 0.0,
                         "key_press": 0.0, "shift_toggle": 0.0, "edge_hug": 0.0}},
    "ppo": {"num_steps": 64, "gamma": 0.995, "gae_lambda": 0.95, "num_minibatches": 8, "update_epochs": 4,
            "clip_coef": 0.2, "clip_vloss": True, "ent_coef": 0.01, "vf_coef": 0.5, "max_grad_norm": 0.5,
            "learning_rate": 3e-4, "anneal_lr": True, "norm_adv": True, "compile": True, "cudagraphs": True},
    "eval": {"episodes": 32, "greedy": True, "seed": 12345},
    "log": {"tensorboard": True, "perf_sync_every": 20, "sample_hz": 1.0},
    "bench": {"seconds": 10.0, "num_envs": [512, 1024, 2048, 4096]},
}

# 这些 section 的子键允许自由增删（模型/特征化器/意图/reward 项各有自己的参数）。
_FREE_SECTIONS = {("model",), ("featurize",), ("intent",), ("reward", "terms")}


def deep_merge(base: dict, over: dict) -> dict:
    out = copy.deepcopy(base)
    for k, v in over.items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = deep_merge(out[k], v)
        else:
            out[k] = copy.deepcopy(v)
    return out


def _check_keys(cfg: dict, ref: dict, path: tuple[str, ...] = ()) -> None:
    if path in _FREE_SECTIONS:
        return
    for k, v in cfg.items():
        if k not in ref:
            raise ValueError(f"未知配置键 {'.'.join(path + (k,))}")
        if isinstance(v, dict) and isinstance(ref[k], dict):
            _check_keys(v, ref[k], path + (k,))


def validate(cfg: dict) -> None:
    _check_keys(cfg, DEFAULTS)
    run, env, ppo, feat, intent = cfg["run"], cfg["env"], cfg["ppo"], cfg["featurize"], cfg["intent"]
    if run["device"] not in ("auto", "cuda", "cpu"):
        raise ValueError(f"run.device 须为 auto/cuda/cpu，得 {run['device']!r}")
    if env["num_envs"] < 1 or env["frame_skip"] < 1 or env["max_frames"] < 1:
        raise ValueError("env.num_envs / frame_skip / max_frames 须 ≥ 1")
    if not 1 <= env["bullets_cap"] <= 8192:
        raise ValueError(f"env.bullets_cap 须在 1..=8192，得 {env['bullets_cap']}")
    if not env["ranks"] or any(not 0 <= r <= 4 for r in env["ranks"]):
        raise ValueError(f"env.ranks 须非空且每项在 0..=4，得 {env['ranks']}")
    if not 1 <= feat["k_bullets"] <= env["bullets_cap"]:
        raise ValueError(f"featurize.k_bullets 须在 1..=bullets_cap({env['bullets_cap']})，得 {feat['k_bullets']}")
    if not 1 <= feat["k_enemies"] <= 256:
        raise ValueError(f"featurize.k_enemies 须在 1..=256，得 {feat['k_enemies']}")
    lo, hi = intent["interval"]
    if not 0 < lo <= hi:
        raise ValueError(f"intent.interval 须满足 0 < lo <= hi，得 {intent['interval']}")
    if (env["num_envs"] * ppo["num_steps"]) % ppo["num_minibatches"] != 0:
        raise ValueError("num_envs × ppo.num_steps 须能被 ppo.num_minibatches 整除（CUDA 图要求固定 minibatch 形状）")
    for k in ("total_updates", "ckpt_every", "eval_every", "torch_threads"):
        if run[k] < 1:
            raise ValueError(f"run.{k} 须 ≥ 1")


def from_dict(d: dict) -> dict:
    cfg = deep_merge(DEFAULTS, d)
    validate(cfg)
    return cfg


def load_config(path: str | Path, overrides: dict | None = None) -> dict:
    with open(path, "rb") as f:
        user = tomllib.load(f)
    if overrides:
        user = deep_merge(user, overrides)
    return from_dict(user)


def _toml_value(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return repr(v)
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    if isinstance(v, list):
        return "[" + ", ".join(_toml_value(x) for x in v) + "]"
    raise TypeError(f"dump_toml 不支持的值类型 {type(v).__name__}")


def _dump_table(d: dict, prefix: str, lines: list[str]) -> None:
    scalars = {k: v for k, v in d.items() if not isinstance(v, dict)}
    tables = {k: v for k, v in d.items() if isinstance(v, dict)}
    if prefix and scalars:
        lines.append(f"[{prefix}]")
    for k, v in scalars.items():
        lines.append(f"{k} = {_toml_value(v)}")
    if scalars:
        lines.append("")
    for k, v in tables.items():
        _dump_table(v, f"{prefix}.{k}" if prefix else k, lines)


def dump_toml(cfg: dict, path: str | Path) -> None:
    lines: list[str] = []
    _dump_table(cfg, "", lines)
    Path(path).write_text("\n".join(lines), encoding="utf-8")
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS（registry 3 + config 11）

- [ ] **Step 7: README 与 CI**

`README.md`：

````markdown
# stg-rl-train

stg-engine 躲弹小模型的训练仓。设计见 `docs/2026-09-15-stg-rl-train-design.md`。

## 一条命令跑训练（任意有 GPU 的 Linux）

```bash
git clone https://github.com/Renko6626/stg-rl-train && cd stg-rl-train
bash run.sh configs/base.toml my-run          # 装依赖 → 训练 → 评测 → 出图 → runs/<时间>-my-run.tar.gz
bash run.sh --resume runs/<目录>               # 续训
bash run.sh configs/base.toml --bench          # 新机器先测吞吐，选 threads × num_envs
```

租机注意：env 在 CPU 上跑，吞吐主要吃 CPU 核数。

## 开发

```bash
uv sync --frozen
uv run --frozen pytest -q
bash run.sh configs/smoke.toml smoke           # CPU 冒烟
uv run --frozen python -m stgtrain.plots runs/<目录>   # 重画曲线
```
````

`.github/workflows/ci.yml`：

```yaml
name: ci
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v7
      - uses: astral-sh/setup-uv@v10
        with:
          enable-cache: true
      - run: uv sync --frozen
      - run: uv run --frozen pytest -q
```

- [ ] **Step 8: Commit + 建 GitHub 仓库**

```bash
cd /data/sunyunbo/www/stg-rl-train
git add -A
git commit -m "chore: 仓库骨架——uv 依赖钉版本、注册表、配置默认值/校验/TOML 读写、CI

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
gh repo create Renko6626/stg-rl-train --public --description "stg-engine 躲弹小模型训练仓（PPO）" --source . --remote origin --push
```

Expected: 推送成功；`gh run list -R Renko6626/stg-rl-train --limit 1` 显示 ci 在跑（结果在 Task 12 末尾确认）。

---

### Task 2: 卡池 —— 发现、元数据、评测划分、起点 + 测试夹具

**Files:**
- Create: `src/stgtrain/cards.py`
- Create: `tests/fixtures/cards/example_ring/main.ecl`, `tests/fixtures/cards/example_calm/main.ecl`, `tests/fixtures/cards/example_calm/meta.toml`, `tests/fixtures/eval_splits.toml`
- Create: `tests/conftest.py`
- Test: `tests/test_cards.py`

**Interfaces:**
- Consumes: `stgtrain.config.from_dict`（Task 1）
- Produces:
  - `Card(id: str, path: Path, meta: dict)`（frozen dataclass）
  - `discover(cards_dir: str | Path) -> dict[str, Card]`（子目录含 `*.ecl` 即为卡；按 id 排序；目录不存在 → `{}`）
  - `EvalSpec(card: str, ranks: tuple[int, ...], episodes: int)`（frozen dataclass）
  - `load_splits(path: str | Path, default_episodes: int) -> list[EvalSpec]`（文件不存在 → `[]`）
  - `allowed_ranks(card: Card, ranks: list[int]) -> list[int]`
  - `compile_cards(cards: dict[str, Card]) -> dict[str, stg_rl.Image]`
  - `train_starts(cards: dict[str, Card], eval_ids: set[str], ranks: list[int]) -> list[stg_rl.Start]`（无可用起点 → `ValueError`）
  - 测试夹具（`tests/conftest.py`）：`FIXTURES: Path`、`small_cfg(**sections) -> dict`（CPU 小配置，指向夹具卡）

- [ ] **Step 1: 放测试卡**

`tests/fixtures/cards/example_ring/main.ecl`（与 stg-engine `docs/rl-card-pool.md` §6 骨架卡逐字相同，已实测）：

```ecl
// RL 卡池骨架：无敌 boss + 一个非符段，时限到 = EVT_PHASE_ENDED = done 2。
const TIME_LIMIT: int = 1800; // 30 s
const RICE: int = 64; // 弹型号 = 图集行 × BULLET_COLOR_STRIDE（抄自 godot/ecl/game/bullets.ecl）

async sub ring_pattern() {
    sh_reset(0);
    sh_ring(0, 1);
    sh_sprite(0, RICE, 0);
    var base: angle = 0deg;
    wait(90); // 开场缓冲：预热随机游走期间别打死人
    loop {
        sh_count(0, 16 + global(GVAR_RANK) * 4, 1);
        sh_speed(0, 2.0fx, 0fx);
        sh_angle(0, base, 0deg);
        sh_fire(0);
        base = base + 7deg;
        wait(40);
    }
}

async sub boss_main() {
    set_invuln(65535); // 常按 SHOT 打不掉血：段长只由时限决定
    phase_begin(0, ring_pattern, TIME_LIMIT, 0);
    wait_spell();
    loop { wait(1); }
}

sub main() {
    _ = spawn_enemy(0.0fx, 100.0fx, 1000, 0, 0, 1, boss_main);
    loop { wait(600); }
}
```

`tests/fixtures/cards/example_calm/main.ecl`（不发弹、300 帧收段：评测与 done=2 测试用）：

```ecl
// 测试卡：无敌 boss、不发弹，300 帧时限到 = done 2。
const TIME_LIMIT: int = 300;

async sub calm_pattern() {
    loop { wait(600); }
}

async sub boss_main() {
    set_invuln(65535);
    phase_begin(0, calm_pattern, TIME_LIMIT, 0);
    wait_spell();
    loop { wait(1); }
}

sub main() {
    _ = spawn_enemy(0.0fx, 100.0fx, 1000, 0, 0, 1, boss_main);
    loop { wait(600); }
}
```

`tests/fixtures/cards/example_calm/meta.toml`：

```toml
title = "测试：静场"
source = "original"
ranks = [0, 2]
marks = [0]
time_limit = 300
tags = []
```

`tests/fixtures/eval_splits.toml`：

```toml
[[eval]]
card = "example_calm"
ranks = [2]
episodes = 4
```

- [ ] **Step 2: 写 `tests/conftest.py`**

```python
from pathlib import Path

import pytest

from stgtrain.config import deep_merge, from_dict

FIXTURES = Path(__file__).parent / "fixtures"


def small_cfg(**sections) -> dict:
    """CPU 小配置：8 env、小模型、指向夹具卡。关键字参数按 section 深合并覆盖。"""
    base = {
        "run": {"device": "cpu", "total_updates": 3, "ckpt_every": 2, "eval_every": 3, "torch_threads": 2},
        "env": {"cards_dir": str(FIXTURES / "cards"), "eval_splits": str(FIXTURES / "eval_splits.toml"),
                "num_envs": 8, "threads": 2, "bullets_cap": 256},
        "featurize": {"k_bullets": 16},
        "model": {"d": 16, "heads": 2, "trunk": 32},
        "ppo": {"num_steps": 16, "num_minibatches": 2, "update_epochs": 1, "compile": False, "cudagraphs": False},
        "eval": {"episodes": 4},
        "log": {"tensorboard": False},
        "bench": {"seconds": 0.5, "num_envs": [8]},
    }
    return from_dict(deep_merge(base, sections))


@pytest.fixture
def cfg():
    return small_cfg()
```

- [ ] **Step 3: 写失败测试**

`tests/test_cards.py`：

```python
import pytest
import stg_rl

from conftest import FIXTURES
from stgtrain.cards import EvalSpec, allowed_ranks, compile_cards, discover, load_splits, train_starts


def test_discover_finds_fixture_cards():
    cards = discover(FIXTURES / "cards")
    assert list(cards) == ["example_calm", "example_ring"]
    assert cards["example_calm"].meta["ranks"] == [0, 2]
    assert cards["example_ring"].meta == {}


def test_discover_missing_dir_is_empty(tmp_path):
    assert discover(tmp_path / "nope") == {}


def test_load_splits():
    specs = load_splits(FIXTURES / "eval_splits.toml", default_episodes=32)
    assert specs == [EvalSpec(card="example_calm", ranks=(2,), episodes=4)]
    assert load_splits(FIXTURES / "missing.toml", 32) == []


def test_allowed_ranks_filters_by_meta():
    cards = discover(FIXTURES / "cards")
    assert allowed_ranks(cards["example_calm"], [0, 1, 2, 3, 4]) == [0, 1, 2]
    assert allowed_ranks(cards["example_ring"], [1, 4]) == [1, 4]


def test_train_starts_excludes_eval_cards():
    cards = discover(FIXTURES / "cards")
    starts = train_starts(cards, {"example_calm"}, [2, 3])
    assert [(s.image, s.mark, s.rank) for s in starts] == [("example_ring", 0, 2), ("example_ring", 0, 3)]
    with pytest.raises(ValueError, match="起点"):
        train_starts(cards, {"example_calm", "example_ring"}, [2])


def test_compile_cards():
    images = compile_cards(discover(FIXTURES / "cards"))
    assert set(images) == {"example_calm", "example_ring"}
    assert all(isinstance(i, stg_rl.Image) for i in images.values())
```

Run: `uv run --frozen pytest tests/test_cards.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.cards`）

- [ ] **Step 4: 实现 `cards.py`**

```python
"""卡池：一张卡 = 一个目录（spec §2 / stg-engine docs/rl-card-pool.md §2）。"""
from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path

import stg_rl


@dataclass(frozen=True)
class Card:
    id: str
    path: Path
    meta: dict


@dataclass(frozen=True)
class EvalSpec:
    card: str
    ranks: tuple[int, ...]
    episodes: int


def discover(cards_dir: str | Path) -> dict[str, Card]:
    root = Path(cards_dir)
    if not root.is_dir():
        return {}
    out: dict[str, Card] = {}
    for d in sorted(p for p in root.iterdir() if p.is_dir()):
        if not any(d.glob("*.ecl")):
            continue
        meta_path = d / "meta.toml"
        meta = tomllib.loads(meta_path.read_text(encoding="utf-8")) if meta_path.exists() else {}
        out[d.name] = Card(id=d.name, path=d, meta=meta)
    return out


def load_splits(path: str | Path, default_episodes: int) -> list[EvalSpec]:
    p = Path(path)
    if not p.exists():
        return []
    data = tomllib.loads(p.read_text(encoding="utf-8"))
    return [
        EvalSpec(card=e["card"], ranks=tuple(e.get("ranks", [2])), episodes=int(e.get("episodes", default_episodes)))
        for e in data.get("eval", [])
    ]


def allowed_ranks(card: Card, ranks: list[int]) -> list[int]:
    lo, hi = card.meta.get("ranks", [0, 4])
    return [r for r in ranks if lo <= r <= hi]


def compile_cards(cards: dict[str, Card]) -> dict[str, stg_rl.Image]:
    return {cid: stg_rl.compile_dir(card.path) for cid, card in cards.items()}


def train_starts(cards: dict[str, Card], eval_ids: set[str], ranks: list[int]) -> list[stg_rl.Start]:
    starts = [
        stg_rl.Start(image=cid, mark=int(mark), rank=r, weight=1.0)
        for cid, card in cards.items()
        if cid not in eval_ids
        for mark in card.meta.get("marks", [0])
        for r in allowed_ranks(card, ranks)
    ]
    if not starts:
        raise ValueError("没有可用的训练起点：卡池为空、全部划进了评测集，或 rank 全被 meta 过滤掉")
    return starts
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 卡池发现/元数据/评测划分/训练起点 + 测试夹具卡（ring 骨架卡 / calm 静场卡）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---
### Task 3: 动作表 v1 + 意图生成器

**Files:**
- Create: `src/stgtrain/actions.py`, `src/stgtrain/intent.py`
- Modify: `src/stgtrain/registry.py`（`BUILTIN_MODULES` 追加 `"stgtrain.intent"`）
- Test: `tests/test_actions.py`, `tests/test_intent.py`

**Interfaces:**
- Consumes: `stgtrain.registry.INTENTS`、`load_builtins`（Task 1）；`tests/conftest.small_cfg`（Task 2）
- Produces:
  - `actions.ACTION_TABLE_VERSION = 1`、`NUM_ACTIONS = 18`、`DIR_BUTTONS: tuple[int, ...]`（9 项）、`MIRROR_DIR: tuple[int, ...]`、`DIR_MASK: int`
  - `actions.action_buttons(action_id: int) -> int`
  - `actions.buttons_table(device) -> Tensor[int64, (18,)]`、`actions.mirror_table(device) -> Tensor[int64, (18,)]`
  - `actions.key_changes(prev: Tensor, cur: Tensor) -> tuple[Tensor int64 新按下键数, Tensor bool shift 是否切换]`
  - `actions.direction_changed(prev: Tensor, cur: Tensor) -> Tensor bool`
  - 注册名 `"lower_half_uniform_v1"` → `LowerHalfUniform(cfg: dict, num_envs: int, device: torch.device, seed: int)`：属性 `target: Tensor (N,2) float32`（**未镜像**的场内坐标）、`countdown: Tensor (N,) int64`；方法 `reset_all() -> None`、`reset(mask: Tensor bool) -> None`、`advance(frames: int, active: Tensor bool) -> Tensor bool`（返回本步刷新的 env）

- [ ] **Step 1: 写动作表失败测试**

`tests/test_actions.py`：

```python
import torch
from stgagent import consts as C

from stgtrain import actions as A


def swap_lr(b: int) -> int:
    l, r = bool(b & C.BTN_LEFT), bool(b & C.BTN_RIGHT)
    b &= ~(C.BTN_LEFT | C.BTN_RIGHT)
    return b | (C.BTN_RIGHT if l else 0) | (C.BTN_LEFT if r else 0)


def test_table_contents():
    assert A.NUM_ACTIONS == 18 and A.ACTION_TABLE_VERSION == 1
    assert A.action_buttons(0) == C.BTN_SHOT
    assert A.action_buttons(1) == C.BTN_SHOT | C.BTN_SLOW
    assert A.action_buttons(2) == C.BTN_UP | C.BTN_SHOT
    assert A.action_buttons(6) == C.BTN_RIGHT | C.BTN_SHOT
    assert A.action_buttons(17) == C.BTN_UP | C.BTN_LEFT | C.BTN_SHOT | C.BTN_SLOW
    all_b = [A.action_buttons(a) for a in range(18)]
    assert len(set(all_b)) == 18
    assert all(b & C.BTN_SHOT and not b & C.BTN_BOMB for b in all_b)


def test_tensor_tables_match_python():
    bt, mt = A.buttons_table("cpu"), A.mirror_table("cpu")
    assert bt.tolist() == [A.action_buttons(a) for a in range(18)]
    assert torch.equal(mt[mt], torch.arange(18)), "镜像两次回到原样"
    for a in range(18):
        assert A.action_buttons(int(mt[a])) == swap_lr(A.action_buttons(a)), a


def test_key_changes_and_direction():
    prev = torch.tensor([C.BTN_UP | C.BTN_SHOT, C.BTN_SHOT | C.BTN_SLOW, C.BTN_SHOT])
    cur = torch.tensor([C.BTN_UP | C.BTN_RIGHT | C.BTN_SHOT | C.BTN_SLOW, C.BTN_SHOT, C.BTN_SHOT])
    presses, toggled = A.key_changes(prev, cur)
    assert presses.tolist() == [2, 0, 0]
    assert toggled.tolist() == [True, True, False]
    assert A.direction_changed(prev, cur).tolist() == [True, False, False]
```

Run: `uv run --frozen pytest tests/test_actions.py -q`
Expected: FAIL（`ImportError: cannot import name 'actions'`）

- [ ] **Step 2: 实现 `actions.py`**

```python
"""动作表 v1（spec §3.2，冻结）：action_id = 方向 × 2 + slow；SHOT 恒按，BOMB 永不置位。

部署侧（th06nc / TH18 DLL）照抄本表。改动任何一项 = 新版本号 + 旧 checkpoint 作废。
"""
from __future__ import annotations

import torch
from stgagent import consts as C
from torch import Tensor

ACTION_TABLE_VERSION = 1
NUM_ACTIONS = 18
# 方向 0 不动 1 上 2 右上 3 右 4 右下 5 下 6 左下 7 左 8 左上（与 stg-rl 预热游走表 WALK_DIRS 同序）
DIR_BUTTONS: tuple[int, ...] = (
    0,
    C.BTN_UP,
    C.BTN_UP | C.BTN_RIGHT,
    C.BTN_RIGHT,
    C.BTN_DOWN | C.BTN_RIGHT,
    C.BTN_DOWN,
    C.BTN_DOWN | C.BTN_LEFT,
    C.BTN_LEFT,
    C.BTN_UP | C.BTN_LEFT,
)
MIRROR_DIR: tuple[int, ...] = (0, 1, 8, 7, 6, 5, 4, 3, 2)
DIR_MASK = C.BTN_UP | C.BTN_DOWN | C.BTN_LEFT | C.BTN_RIGHT
_KEY_BITS = 7  # UP DOWN LEFT RIGHT SHOT BOMB SLOW


def action_buttons(action_id: int) -> int:
    d, slow = divmod(int(action_id), 2)
    return DIR_BUTTONS[d] | C.BTN_SHOT | (C.BTN_SLOW if slow else 0)


def buttons_table(device) -> Tensor:
    return torch.tensor([action_buttons(a) for a in range(NUM_ACTIONS)], dtype=torch.int64, device=device)


def mirror_table(device) -> Tensor:
    return torch.tensor([MIRROR_DIR[a // 2] * 2 + a % 2 for a in range(NUM_ACTIONS)], dtype=torch.int64, device=device)


def _popcount(x: Tensor) -> Tensor:
    return sum(((x >> i) & 1) for i in range(_KEY_BITS))


def key_changes(prev: Tensor, cur: Tensor) -> tuple[Tensor, Tensor]:
    changed = prev ^ cur
    return _popcount(changed & cur), (changed & C.BTN_SLOW) != 0


def direction_changed(prev: Tensor, cur: Tensor) -> Tensor:
    return (prev & DIR_MASK) != (cur & DIR_MASK)
```

Run: `uv run --frozen pytest tests/test_actions.py -q`
Expected: PASS

- [ ] **Step 3: 写意图失败测试**

`tests/test_intent.py`：

```python
import torch

from conftest import small_cfg
from stgtrain.registry import INTENTS, load_builtins

load_builtins()


def make(n=64, seed=3, **intent):
    cfg = small_cfg(intent=intent) if intent else small_cfg()
    return INTENTS.get("lower_half_uniform_v1")(cfg, n, torch.device("cpu"), seed)


def test_points_and_countdowns_in_range():
    it = make(n=4096, margin=16.0, interval=[120, 300])
    x, y = it.target[:, 0], it.target[:, 1]
    assert x.min() >= -176 and x.max() <= 176
    assert y.min() >= 224 and y.max() <= 432
    assert it.countdown.min() >= 120 and it.countdown.max() <= 300
    assert x.min() < -150 and x.max() > 150 and y.min() < 240 and y.max() > 420, "应铺满区域"


def test_advance_refreshes_only_expired_active_envs():
    it = make(n=4)
    it.countdown = torch.tensor([1, 5, 1, 1])
    before = it.target.clone()
    active = torch.tensor([True, True, True, False])
    refreshed = it.advance(1, active)
    assert refreshed.tolist() == [True, False, True, False]
    assert torch.equal(it.target[1], before[1]) and torch.equal(it.target[3], before[3])
    assert it.countdown[1] == 4 and it.countdown[3] == 1, "未激活的 env 不扣计时"
    assert (it.countdown[[0, 2]] >= 120).all()


def test_reset_mask_and_determinism():
    a, b = make(seed=9), make(seed=9)
    assert torch.equal(a.target, b.target)
    mask = torch.zeros(64, dtype=torch.bool)
    mask[5] = True
    keep = a.target.clone()
    a.reset(mask)
    b.reset(mask)
    assert torch.equal(a.target, b.target)
    assert torch.equal(a.target[~mask], keep[~mask])
    assert not torch.equal(a.target[5], keep[5])
```

Run: `uv run --frozen pytest tests/test_intent.py -q`
Expected: FAIL（`ValueError: 未知的 intent 'lower_half_uniform_v1'`）

- [ ] **Step 4: 实现 `intent.py` 并登记模块**

`src/stgtrain/intent.py`：

```python
"""意图生成器（spec §3.3）：下半屏均匀随机目标点，随机间隔刷新。坐标为未镜像的场内坐标。"""
from __future__ import annotations

import torch
from torch import Tensor

from .registry import INTENTS


@INTENTS.register("lower_half_uniform_v1")
class LowerHalfUniform:
    def __init__(self, cfg: dict, num_envs: int, device: torch.device, seed: int):
        c = cfg["intent"]
        m = float(c["margin"])
        self.x_lo, self.x_hi = -192.0 + m, 192.0 - m
        self.y_lo, self.y_hi = 224.0, 448.0 - m
        self.lo, self.hi = int(c["interval"][0]), int(c["interval"][1])
        self.n, self.device = int(num_envs), device
        self.gen = torch.Generator(device=device)
        self.gen.manual_seed(int(seed) % (2**63))
        self.target = torch.zeros(self.n, 2, device=device)
        self.countdown = torch.zeros(self.n, dtype=torch.int64, device=device)
        self.reset_all()

    def _sample(self) -> tuple[Tensor, Tensor]:
        # 每次都为全部 env 抽样再按掩码取用：随机数消耗与掩码无关，保证可复现。
        u = torch.rand(self.n, 2, generator=self.gen, device=self.device)
        xy = torch.stack(
            [self.x_lo + u[:, 0] * (self.x_hi - self.x_lo), self.y_lo + u[:, 1] * (self.y_hi - self.y_lo)], dim=-1
        )
        cd = torch.randint(self.lo, self.hi + 1, (self.n,), generator=self.gen, device=self.device)
        return xy, cd

    def reset_all(self) -> None:
        self.target, self.countdown = self._sample()

    def reset(self, mask: Tensor) -> None:
        xy, cd = self._sample()
        self.target = torch.where(mask[:, None], xy, self.target)
        self.countdown = torch.where(mask, cd, self.countdown)

    def advance(self, frames: int, active: Tensor) -> Tensor:
        self.countdown = self.countdown - int(frames) * active.to(torch.int64)
        refreshed = active & (self.countdown <= 0)
        self.reset(refreshed)
        return refreshed
```

`src/stgtrain/registry.py` 中把

```python
BUILTIN_MODULES: list[str] = []
```

改为

```python
BUILTIN_MODULES: list[str] = [
    "stgtrain.intent",
]
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: 动作表 v1（冻结、镜像置换、按键变化）+ 意图生成器 lower_half_uniform_v1

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: `envwrap` —— 缓冲 → 设备张量

**Files:**
- Create: `src/stgtrain/envwrap.py`
- Test: `tests/test_envwrap.py`

**Interfaces:**
- Consumes: `actions.buttons_table / mirror_table`、`INTENTS`（Task 3）；`cards.discover / compile_cards`（Task 2）
- Produces:
  - `RawObs`（dataclass，全部在 `device` 上；**x 类量已按镜像取反**）：`player_xy (N,2)`、`player_hit_r (N,)`、`player_speed (N,)`、`player_focus (N,) bool`、`bullets (N,C,5)`（列 `x, y, vx, vy, radius`）、`bullets_mask (N,C) bool`（存在且 collidable）、`enemies (N,E,4)`（列 `x, y, radius, is_boss`，E = 256）、`enemies_mask (N,E) bool`、`target_xy (N,2)`
  - `StepInfo`（dataclass）：`done (N,) int64`、`events (N,8) int64`、`ep_frames (N,) int64`、`refreshed (N,) bool`、`buttons (N,) int64`（本步实际发给 env 的物理按键）、`prev_buttons (N,) int64`（上一步物理按键；新局首步为 0）
  - `EnvWrapper(cfg: dict, images: dict[str, stg_rl.Image], starts: list[stg_rl.Start], device: torch.device, seed: int, num_envs: int | None = None, mirror: bool | None = None)`
    - 属性：`n`、`cap`、`frame_skip`、`device`、`buf`、`intent`、`mirrored (N,) bool`、`prev_buttons (N,) int64`
    - `reset() -> RawObs`
    - `step(action_ids: Tensor (N,) int64 on device, timer=None) -> tuple[RawObs, StepInfo]`；`timer` 若非 None 须有 `phase(name)` 上下文管理器（Task 9 的 `PhaseTimer`），本方法计 `"env_step"` 与 `"h2d"` 两段
  - 常量 `FX_SCALE`、`ENEMY_FLAG_BOSS = 0x01`、`ENEMY_FLAG_COLLIDABLE = 0x10`、`BULLET_FLAG_COLLIDABLE = 0x01`

- [ ] **Step 1: 写失败测试**

`tests/test_envwrap.py`：

```python
import numpy as np
import pytest
import stg_rl
import torch
from stgagent import consts as C

from conftest import FIXTURES, small_cfg
from stgtrain.cards import compile_cards, discover
from stgtrain.envwrap import EnvWrapper
from stgtrain.registry import load_builtins

load_builtins()
CPU = torch.device("cpu")
IMAGES = compile_cards(discover(FIXTURES / "cards"))


def ring(seed=1, mirror=False, **env):
    cfg = small_cfg(env={"mirror": mirror, **env})
    return EnvWrapper(cfg, IMAGES, [stg_rl.Start("example_ring", 0, 2)], CPU, seed=seed)


def fx_col(u8: np.ndarray, off: int) -> np.ndarray:
    return np.frombuffer(np.ascontiguousarray(u8[..., off:off + 4]).tobytes(), "<i4").astype(np.float64) / 65536


def still(n):
    return torch.zeros(n, dtype=torch.int64)


def test_reset_shapes():
    w = ring()
    o = w.reset()
    assert o.player_xy.shape == (8, 2) and o.bullets.shape == (8, 256, 5) and o.bullets_mask.shape == (8, 256)
    assert o.enemies.shape == (8, 256, 4) and o.enemies_mask.shape == (8, 256) and o.target_xy.shape == (8, 2)
    assert (o.player_xy[:, 0].abs() <= 192).all() and (o.player_xy[:, 1] <= 448).all()
    assert torch.allclose(o.target_xy, w.intent.target)


def test_decode_matches_raw_buffers():
    w = ring()
    w.reset()
    for _ in range(200):
        o, _ = w.step(still(8))
    p = w.buf["player"].numpy()
    x_off = stg_rl.OFFSETS["player"]["x"][0]
    assert np.allclose(o.player_xy[:, 0].numpy(), fx_col(p, x_off))
    rows, offs = w.buf["bullets"].numpy(), w.buf["bullets_offsets"].numpy()
    assert offs[-1] > 0, "200 帧后场上应有弹"
    bo = stg_rl.OFFSETS["bullets"]
    for i in range(8):
        r = rows[offs[i]:offs[i + 1]]
        k = len(r)
        assert np.allclose(o.bullets[i, :k, 0].numpy(), fx_col(r, bo["x"][0]))
        assert np.allclose(o.bullets[i, :k, 4].numpy(), fx_col(r, bo["radius"][0]))
        assert o.bullets_mask[i, :k].numpy().tolist() == ((r[:, bo["flags"][0]] & 1) != 0).tolist()
        assert not o.bullets_mask[i, k:].any()
    assert (o.enemies_mask.sum(1) >= 1).all(), "boss 本体可碰撞"


def test_mirror_flips_x_consistently():
    a, b = ring(seed=5, mirror=False), ring(seed=5, mirror=True)
    a.reset()
    b.reset()
    for _ in range(120):
        oa, _ = a.step(still(8))  # 方向 0 在镜像下不变 ⇒ 两边物理输入相同
        ob, _ = b.step(still(8))
    s = torch.where(b.mirrored, -1.0, 1.0)
    assert b.mirrored.any() and (~b.mirrored).any(), "种子 5 下 8 个 env 应两种都有"
    assert torch.allclose(ob.player_xy[:, 0], s * oa.player_xy[:, 0])
    assert torch.allclose(ob.player_xy[:, 1], oa.player_xy[:, 1])
    assert torch.allclose(ob.target_xy[:, 0], s * oa.target_xy[:, 0])
    assert torch.allclose(ob.bullets[..., 0], s[:, None] * oa.bullets[..., 0])
    assert torch.allclose(ob.bullets[..., 2], s[:, None] * oa.bullets[..., 2])


def test_actions_are_mirrored_before_sending():
    w = ring(mirror=False)
    w.reset()
    right = torch.full((8,), 6, dtype=torch.int64)
    w.mirrored[:4] = True
    _, info = w.step(right)
    assert info.buttons[:4].eq(C.BTN_LEFT | C.BTN_SHOT).all()
    assert info.buttons[4:].eq(C.BTN_RIGHT | C.BTN_SHOT).all()
    _, info2 = w.step(right)
    assert torch.equal(info2.prev_buttons, info.buttons)


def test_done_resets_per_env_state():
    cfg = small_cfg(env={"mirror": True, "warmup_max": 0})
    w = EnvWrapper(cfg, IMAGES, [stg_rl.Start("example_calm", 0, 2)], CPU, seed=2)
    w.reset()
    for _ in range(400):
        _, info = w.step(torch.full((8,), 10, dtype=torch.int64))  # 一直按下：贴底，远离 (0,100) 的 boss 本体，不会撞死
        if (info.done == 2).all():
            break
    else:
        pytest.fail("静场卡 300 帧应以 done=2 收段")
    assert (info.ep_frames > 250).all()
    assert w.prev_buttons.eq(0).all(), "新局首步的上一帧按键清零"
    assert not info.refreshed.any(), "结束的 env 本步不算刷新"
    assert (w.intent.countdown >= 120).all()


def test_same_seed_is_deterministic():
    a, b = ring(seed=11, mirror=True), ring(seed=11, mirror=True)
    oa, ob = a.reset(), b.reset()
    g = torch.Generator().manual_seed(0)
    for _ in range(60):
        act = torch.randint(0, 18, (8,), generator=g)
        oa, ia = a.step(act)
        ob, ib = b.step(act)
    for f in ("player_xy", "bullets", "bullets_mask", "enemies", "target_xy"):
        assert torch.equal(getattr(oa, f), getattr(ob, f)), f
    assert torch.equal(ia.done, ib.done)
```

Run: `uv run --frozen pytest tests/test_envwrap.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.envwrap`）

- [ ] **Step 2: 实现 `envwrap.py`**

```python
"""包 stg_rl.VecEnv：pinned 缓冲 → 设备张量（RawObs / StepInfo），外加意图、镜像、上一帧按键（spec §3.1）。

镜像：镜像局里 RawObs 的所有 x 类量取反，模型输出的动作先按动作表置换再发给 env——模型始终活在一个自洽的镜像世界里。
意图目标在 intent 里存未镜像坐标；RawObs.target_xy 是镜像后的。
"""
from __future__ import annotations

import contextlib
import os
from dataclasses import dataclass

import stg_rl
import torch
from torch import Tensor

from . import actions
from .registry import INTENTS

FX_SCALE = 1.0 / 65536.0
ENEMY_FLAG_BOSS = 0x01
ENEMY_FLAG_COLLIDABLE = 0x10
BULLET_FLAG_COLLIDABLE = 0x01


@dataclass
class RawObs:
    player_xy: Tensor
    player_hit_r: Tensor
    player_speed: Tensor
    player_focus: Tensor
    bullets: Tensor
    bullets_mask: Tensor
    enemies: Tensor
    enemies_mask: Tensor
    target_xy: Tensor


@dataclass
class StepInfo:
    done: Tensor
    events: Tensor
    ep_frames: Tensor
    refreshed: Tensor
    buttons: Tensor
    prev_buttons: Tensor


def _off(table: str, field: str) -> int:
    return stg_rl.OFFSETS[table][field][0]


def _fx(rows: Tensor, off: int) -> Tensor:
    """rows[..., off:off+4] 小端 int32 定点 → float32 像素。"""
    return rows[..., off:off + 4].contiguous().view(torch.int32).squeeze(-1).to(torch.float32) * FX_SCALE


def _u16(rows: Tensor, off: int) -> Tensor:
    return rows[..., off].to(torch.int64) | (rows[..., off + 1].to(torch.int64) << 8)


def _phase(timer, name: str):
    return timer.phase(name) if timer is not None else contextlib.nullcontext()


class EnvWrapper:
    def __init__(self, cfg: dict, images: dict, starts: list, device: torch.device, seed: int,
                 num_envs: int | None = None, mirror: bool | None = None):
        e = cfg["env"]
        self.n = int(num_envs if num_envs is not None else e["num_envs"])
        self.cap = int(e["bullets_cap"])
        self.frame_skip = int(e["frame_skip"])
        self.device = device
        threads = max(1, min(int(e["threads"]) or (os.cpu_count() or 1), self.n))
        self.buf = stg_rl.alloc_buffers(self.n, self.cap, backend="torch", pin=device.type == "cuda")
        self.env = stg_rl.VecEnv(
            self.n, threads, images, starts, frame_skip=self.frame_skip, max_frames=int(e["max_frames"]),
            warmup_max=int(e["warmup_max"]), bullets_cap=self.cap, seed=int(seed), buffers=self.buf,
        )
        self.intent = INTENTS.get(cfg["intent"]["name"])(cfg, self.n, device, seed)
        self.mirror_enabled = bool(e["mirror"] if mirror is None else mirror)
        self._mirror_gen = torch.Generator(device=device)
        self._mirror_gen.manual_seed((int(seed) * 2654435761 + 97) % (2**63))
        self.mirrored = torch.zeros(self.n, dtype=torch.bool, device=device)
        self.prev_buttons = torch.zeros(self.n, dtype=torch.int64, device=device)
        self._buttons = actions.buttons_table(device)
        self._mirror = actions.mirror_table(device)
        self._arange_n = torch.arange(self.n, device=device)
        self._arange_e = torch.arange(stg_rl.ENEMIES_CAP, device=device)

    def _dev(self, t: Tensor) -> Tensor:
        # CUDA：从 pinned 缓冲异步拷贝。安全性来自下一次 step 之前 `buttons.to("cpu")` 的阻塞同步——
        # 在那之前 env 不会再写缓冲。CPU：必须 clone，否则 RawObs 与下一步被覆写的缓冲共享内存。
        return t.to(self.device, non_blocking=True) if self.device.type == "cuda" else t.clone()

    def _draw_mirror(self, mask: Tensor) -> None:
        if not self.mirror_enabled:
            return
        flips = torch.rand(self.n, generator=self._mirror_gen, device=self.device) < 0.5
        self.mirrored = torch.where(mask, flips, self.mirrored)

    def reset(self) -> RawObs:
        self.env.reset()
        self.intent.reset_all()
        self._draw_mirror(torch.ones(self.n, dtype=torch.bool, device=self.device))
        self.prev_buttons = torch.zeros(self.n, dtype=torch.int64, device=self.device)
        return self._decode()

    def step(self, action_ids: Tensor, timer=None) -> tuple[RawObs, StepInfo]:
        ids = torch.where(self.mirrored, self._mirror[action_ids], action_ids)
        buttons = self._buttons[ids]
        with _phase(timer, "env_step"):
            self.env.step(buttons.to("cpu"))
        with _phase(timer, "h2d"):
            done = self._dev(self.buf["done"]).to(torch.int64)
            events = self._dev(self.buf["events"]).to(torch.int64)
            ep_frames = self._dev(self.buf["ep_frames"]).to(torch.int64)
            ended = done != 0
            self.intent.reset(ended)
            self._draw_mirror(ended)
            refreshed = self.intent.advance(self.frame_skip, ~ended)
            info = StepInfo(done=done, events=events, ep_frames=ep_frames, refreshed=refreshed,
                            buttons=buttons, prev_buttons=self.prev_buttons)
            self.prev_buttons = torch.where(ended, torch.zeros_like(buttons), buttons)
            obs = self._decode()
        return obs, info

    def _decode(self) -> RawObs:
        b, n, cap, dev = self.buf, self.n, self.cap, self.device
        player = self._dev(b["player"])
        px, py = _fx(player, _off("player", "x")), _fx(player, _off("player", "y"))
        hit_r = _fx(player, _off("player", "hit_radius"))
        speed = _fx(player, _off("player", "speed"))
        focus = player[:, _off("player", "focus")] != 0

        m = int(b["bullets_offsets"][n])  # CPU 缓冲，读它不触发 GPU 同步
        rows = self._dev(b["bullets"][:m])
        offs = self._dev(b["bullets_offsets"]).to(torch.int64)
        counts = offs[1:] - offs[:-1]
        env_idx = torch.repeat_interleave(self._arange_n, counts, output_size=m)
        slot = torch.arange(m, device=dev) - offs[:-1][env_idx]
        feats = torch.stack([_fx(rows, _off("bullets", f)) for f in ("x", "y", "vx", "vy", "radius")], dim=-1)
        collidable = (rows[:, _off("bullets", "flags")] & BULLET_FLAG_COLLIDABLE) != 0
        bullets = torch.zeros(n, cap, 5, device=dev)
        bmask = torch.zeros(n, cap, dtype=torch.bool, device=dev)
        bullets[env_idx, slot] = feats
        bmask[env_idx, slot] = collidable

        en = self._dev(b["enemies"])
        ecount = self._dev(b["enemies_count"]).to(torch.int64)
        eflags = _u16(en, _off("enemies", "flags"))
        emask = (self._arange_e[None, :] < ecount[:, None]) & ((eflags & ENEMY_FLAG_COLLIDABLE) != 0)
        enemies = torch.stack([
            _fx(en, _off("enemies", "x")), _fx(en, _off("enemies", "y")), _fx(en, _off("enemies", "hit_w")),
            ((eflags & ENEMY_FLAG_BOSS) != 0).to(torch.float32),
        ], dim=-1)

        target = self.intent.target.clone()
        sign = torch.where(self.mirrored, -1.0, 1.0)
        px = px * sign
        bullets[..., 0] *= sign[:, None]
        bullets[..., 2] *= sign[:, None]
        enemies[..., 0] *= sign[:, None]
        target[:, 0] *= sign
        return RawObs(
            player_xy=torch.stack([px, py], dim=-1), player_hit_r=hit_r, player_speed=speed, player_focus=focus,
            bullets=bullets, bullets_mask=bmask, enemies=enemies, enemies_mask=emask, target_xy=target,
        )
```

Run: `uv run --frozen pytest tests/test_envwrap.py -q`
Expected: PASS。若 `test_mirror_flips_x_consistently` 因「种子 5 下全同向」失败，改用另一个种子（在测试里注明所选种子的理由），不要改实现。

- [ ] **Step 3: 全量测试 + Commit**

Run: `uv run --frozen pytest -q`
Expected: PASS

```bash
git add -A
git commit -m "feat: envwrap——pinned 缓冲解码为 RawObs/StepInfo（CSR 铺成定长、镜像、意图刷新、上一帧按键）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: 特征化器 `danger_topk_v1`

**Files:**
- Create: `src/stgtrain/featurize/__init__.py`, `src/stgtrain/featurize/danger_topk_v1.py`
- Modify: `src/stgtrain/registry.py`（`BUILTIN_MODULES` 追加 `"stgtrain.featurize.danger_topk_v1"`）、`tests/conftest.py`（加 `raw_obs` 构造器）
- Test: `tests/test_featurize.py`

**Interfaces:**
- Consumes: `envwrap.RawObs`（Task 4）；`FEATURIZERS`（Task 1）
- Produces:
  - `closest_approach(p: Tensor (...,2), v: Tensor (...,2), r_sum: Tensor (...), horizon: float) -> tuple[Tensor d_min (...), Tensor t_star (...)]`
  - 注册名 `"danger_topk_v1"` → `DangerTopKV1(cfg: dict)`：`spec() -> dict[str, tuple[int, ...]]`、`player_velocity(obs: RawObs) -> Tensor (N,2)`、`__call__(obs: RawObs) -> dict[str, Tensor]`
  - 输出键与形状（不含 N）：`bullets (K_b,7)`、`bullets_mask (K_b,)` bool、`enemies (K_e,6)`、`enemies_mask (K_e,)` bool、`density (2,14,12)`、`player (3,)`、`cond (4,)`
  - 列定义：bullets `[rel_x/192, rel_y/192, rel_vx/8, rel_vy/8, radius/8, d_min/d_max(截到 ±1), t*/H]`；enemies `[rel_x/192, rel_y/192, radius/32, d_min/d_max, t*/H, is_boss]`；player `[x/192, y/192, focus]`；cond `[dx/192, dy/192, dist/448, in_R]`
  - `tests/conftest.raw_obs(n=2, cap=16, e=8, player=(0.0, 384.0) | list, target=(0.0, 300.0) | list, speed=4.0, hit_r=2.0, bullets=None, enemies=None, focus=False) -> RawObs`

- [ ] **Step 1: conftest 加 `raw_obs`**

在 `tests/conftest.py` 末尾追加：

```python
import torch

from stgtrain.envwrap import RawObs


def _per_env(v, n):
    return [tuple(x) for x in v] if isinstance(v, list) else [tuple(v)] * n


def raw_obs(n=2, cap=16, e=8, player=(0.0, 384.0), target=(0.0, 300.0), speed=4.0, hit_r=2.0,
            bullets=None, enemies=None, focus=False) -> RawObs:
    """手搓 RawObs。bullets / enemies：按 env 给行列表，弹行 (x, y, vx, vy, r)、敌行 (x, y, r, is_boss)。"""
    b = torch.zeros(n, cap, 5)
    bm = torch.zeros(n, cap, dtype=torch.bool)
    for i, rows in enumerate(bullets or []):
        for j, row in enumerate(rows):
            b[i, j] = torch.tensor(row, dtype=torch.float32)
            bm[i, j] = True
    en = torch.zeros(n, e, 4)
    em = torch.zeros(n, e, dtype=torch.bool)
    for i, rows in enumerate(enemies or []):
        for j, row in enumerate(rows):
            en[i, j] = torch.tensor(row, dtype=torch.float32)
            em[i, j] = True
    return RawObs(
        player_xy=torch.tensor(_per_env(player, n), dtype=torch.float32),
        player_hit_r=torch.full((n,), float(hit_r)),
        player_speed=torch.full((n,), float(speed)),
        player_focus=torch.full((n,), bool(focus)),
        bullets=b, bullets_mask=bm, enemies=en, enemies_mask=em,
        target_xy=torch.tensor(_per_env(target, n), dtype=torch.float32),
    )


def mirror_obs(o: RawObs) -> RawObs:
    """把 RawObs 左右镜像（x 类量取反），用于镜像一致性测试。"""
    def neg_col(t, cols):
        t = t.clone()
        for c in cols:
            t[..., c] = -t[..., c]
        return t

    return RawObs(
        player_xy=neg_col(o.player_xy, [0]), player_hit_r=o.player_hit_r, player_speed=o.player_speed,
        player_focus=o.player_focus, bullets=neg_col(o.bullets, [0, 2]), bullets_mask=o.bullets_mask,
        enemies=neg_col(o.enemies, [0]), enemies_mask=o.enemies_mask, target_xy=neg_col(o.target_xy, [0]),
    )
```

- [ ] **Step 2: 写失败测试**

`tests/test_featurize.py`：

```python
import pytest
import torch

from conftest import mirror_obs, raw_obs, small_cfg
from stgtrain.featurize.danger_topk_v1 import closest_approach
from stgtrain.registry import FEATURIZERS, load_builtins

load_builtins()


def feat(**featurize):
    return FEATURIZERS.get("danger_topk_v1")(small_cfg(featurize=featurize))


def test_closest_approach_matches_brute_force():
    g = torch.Generator().manual_seed(0)
    p = torch.randn(200, 2, generator=g) * 100
    v = torch.randn(200, 2, generator=g) * 5
    r = torch.rand(200, generator=g) * 5
    d, t = closest_approach(p, v, r, 60.0)
    ts = torch.linspace(0, 60, 60001)
    brute = (p[:, None, :] + v[:, None, :] * ts[None, :, None]).norm(dim=-1).min(dim=1).values - r
    assert torch.allclose(d, brute, atol=1e-2)
    assert ((t >= 0) & (t <= 60)).all()


def test_output_shapes_match_spec():
    f = feat()
    o = raw_obs(n=3, bullets=[[(10.0, 300.0, 0.0, 1.0, 2.0)]])
    out = f(o)
    assert set(out) == set(f.spec())
    for k, shape in f.spec().items():
        assert tuple(out[k].shape) == (3, *shape), k
    assert f.spec()["bullets"] == (16, 7) and f.spec()["density"] == (2, 14, 12)


def test_player_velocity_towards_target_or_zero_inside_hold_radius():
    f = feat()
    o = raw_obs(n=2, player=[(0.0, 400.0), (0.0, 400.0)], target=[(0.0, 300.0), (0.0, 390.0)], speed=5.0)
    v = f.player_velocity(o)
    assert torch.allclose(v[0], torch.tensor([0.0, -5.0]))
    assert torch.equal(v[1], torch.zeros(2))


def test_topk_ranks_by_danger_not_distance():
    # 自机在指令点上（v_p = 0）。A 近但静止；B 远但正面高速飞来；C 横向远离。
    a, b, c = (0.0, 300.0, 0.0, 0.0, 2.0), (0.0, 200.0, 0.0, 8.0, 2.0), (100.0, 384.0, 8.0, 0.0, 2.0)
    o = raw_obs(n=1, player=(0.0, 384.0), target=(0.0, 384.0), bullets=[[a, b, c]])
    out = feat(k_bullets=2)(o)
    assert out["bullets_mask"][0].tolist() == [True, True]
    assert out["bullets"][0, 0, 1] == pytest.approx((200 - 384) / 192), "B 最危险，排第一"
    assert out["bullets"][0, 1, 1] == pytest.approx((300 - 384) / 192)
    out3 = feat(k_bullets=3, d_max=90.0)(o)
    assert out3["bullets_mask"][0].tolist() == [True, True, False], "C 的 d_min=96 > d_max=90"
    assert torch.equal(out3["bullets"][0, 2], torch.zeros(7)), "未入选行清零"


def test_enemies_selected_and_boss_flag():
    o = raw_obs(n=1, player=(0.0, 384.0), target=(0.0, 384.0),
                enemies=[[(0.0, 100.0, 16.0, 1.0), (50.0, 360.0, 12.0, 0.0)]])
    out = feat(d_max=300.0)(o)
    assert out["enemies_mask"][0, :2].tolist() == [True, True]
    assert out["enemies"][0, 0, 5] == 0.0 and out["enemies"][0, 1, 5] == 1.0, "近的杂兵排前，boss 其次"


def test_empty_field_is_finite_and_masked():
    out = feat()(raw_obs(n=2))
    assert not out["bullets_mask"].any() and not out["enemies_mask"].any()
    for k, v in out.items():
        assert torch.isfinite(v.float()).all(), k
    assert torch.equal(out["bullets"], torch.zeros_like(out["bullets"]))
    assert torch.equal(out["density"], torch.zeros_like(out["density"]))


def test_density_counts_and_mirror_consistency():
    rows = [(10.5, 300.5, 0.0, 3.0, 2.0), (-100.5, 20.5, 1.0, 0.0, 2.0), (170.5, 440.5, -2.0, -1.0, 2.0)]
    o = raw_obs(n=1, player=(20.5, 380.5), target=(-60.5, 250.5), bullets=[rows])
    f = feat(d_max=1000.0)
    a, b = f(o), f(mirror_obs(o))
    assert a["density"][0, 0].sum() == 3
    assert a["density"][0, 0, int(300.5 // 32), int((10.5 + 192) // 32)] == 1
    assert torch.allclose(b["density"], a["density"].flip(-1))
    assert torch.equal(a["bullets_mask"], b["bullets_mask"])
    assert torch.allclose(b["bullets"][..., [0, 2]], -a["bullets"][..., [0, 2]], atol=1e-6)
    assert torch.allclose(b["bullets"][..., [1, 3, 4, 5, 6]], a["bullets"][..., [1, 3, 4, 5, 6]], atol=1e-6)
    assert torch.allclose(b["cond"][:, 0], -a["cond"][:, 0]) and torch.allclose(b["cond"][:, 1:], a["cond"][:, 1:])
    assert torch.allclose(b["player"][:, 0], -a["player"][:, 0])
```

Run: `uv run --frozen pytest tests/test_featurize.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.featurize`）

- [ ] **Step 3: 实现**

`src/stgtrain/featurize/__init__.py`：

```python
"""特征化器实现（每个文件注册一个名字）。"""
```

`src/stgtrain/featurize/danger_topk_v1.py`：

```python
"""特征化器 danger_topk_v1（spec §3.4）：按预测最近接近距离取前 K 颗弹 / 敌 + 密度图 + 自机 + 条件向量。

输出形状固定（可录 CUDA 图）；未入选行清零并由掩码标出。输入 RawObs 已按镜像处理，这里不再关心镜像。
"""
from __future__ import annotations

import torch
from torch import Tensor

from ..envwrap import RawObs
from ..registry import FEATURIZERS

GRID_H, GRID_W, CELL = 14, 12, 32.0


def closest_approach(p: Tensor, v: Tensor, r_sum: Tensor, horizon: float) -> tuple[Tensor, Tensor]:
    """相对位置 p、相对速度 v（像素/帧）匀速外推，t ∈ [0, horizon] 内的边缘最近距离与达到时刻。"""
    vv = (v * v).sum(-1)
    pv = (p * v).sum(-1)
    t = (-pv / (vv + 1e-6)).clamp(0.0, horizon)
    closest = p + v * t.unsqueeze(-1)
    return closest.norm(dim=-1) - r_sum, t


def _gather(x: Tensor, idx: Tensor) -> Tensor:
    if x.dim() == 2:
        return torch.gather(x, 1, idx)
    return torch.gather(x, 1, idx.unsqueeze(-1).expand(-1, -1, x.shape[-1]))


@FEATURIZERS.register("danger_topk_v1")
class DangerTopKV1:
    F_BULLET, F_ENEMY, F_PLAYER, F_COND = 7, 6, 3, 4

    def __init__(self, cfg: dict):
        f = cfg["featurize"]
        self.kb, self.ke = int(f["k_bullets"]), int(f["k_enemies"])
        self.horizon = float(f["horizon"])
        self.d_max = float(f["d_max"])
        self.hold_r = float(cfg["reward"]["hold_radius"])

    def spec(self) -> dict[str, tuple[int, ...]]:
        return {
            "bullets": (self.kb, self.F_BULLET), "bullets_mask": (self.kb,),
            "enemies": (self.ke, self.F_ENEMY), "enemies_mask": (self.ke,),
            "density": (2, GRID_H, GRID_W), "player": (self.F_PLAYER,), "cond": (self.F_COND,),
        }

    def player_velocity(self, obs: RawObs) -> Tensor:
        d = obs.target_xy - obs.player_xy
        dist = d.norm(dim=-1, keepdim=True)
        v = d / dist.clamp_min(1e-6) * obs.player_speed.unsqueeze(-1)
        return torch.where(dist < self.hold_r, torch.zeros_like(v), v)

    def _topk(self, p: Tensor, v: Tensor, radius: Tensor, mask: Tensor, hit_r: Tensor, k: int):
        dmin, t = closest_approach(p, v, radius + hit_r[:, None], self.horizon)
        key = dmin.masked_fill(~mask, float("inf"))
        kval, idx = torch.topk(key, k, dim=1, largest=False)
        sel = torch.isfinite(kval) & (kval <= self.d_max)
        d_norm = kval.clamp(-self.d_max, self.d_max) / self.d_max
        return idx, sel, d_norm, _gather(t, idx) / self.horizon

    def __call__(self, obs: RawObs) -> dict[str, Tensor]:
        n = obs.player_xy.shape[0]
        dev = obs.player_xy.device
        pos = obs.player_xy[:, None, :]
        vp = self.player_velocity(obs)

        pb = obs.bullets[..., 0:2] - pos
        vb = obs.bullets[..., 2:4] - vp[:, None, :]
        idx, bsel, bd, bt = self._topk(pb, vb, obs.bullets[..., 4], obs.bullets_mask, obs.player_hit_r, self.kb)
        bullets = torch.cat([
            _gather(pb, idx) / 192.0, _gather(vb, idx) / 8.0, _gather(obs.bullets[..., 4:5], idx) / 8.0,
            bd.unsqueeze(-1), bt.unsqueeze(-1),
        ], dim=-1) * bsel.unsqueeze(-1)

        pe = obs.enemies[..., 0:2] - pos
        ve = (-vp)[:, None, :].expand_as(pe)
        eidx, esel, ed, et = self._topk(pe, ve, obs.enemies[..., 2], obs.enemies_mask, obs.player_hit_r, self.ke)
        enemies = torch.cat([
            _gather(pe, eidx) / 192.0, _gather(obs.enemies[..., 2:3], eidx) / 32.0,
            ed.unsqueeze(-1), et.unsqueeze(-1), _gather(obs.enemies[..., 3:4], eidx),
        ], dim=-1) * esel.unsqueeze(-1)

        valid = obs.bullets_mask.to(torch.float32)
        col = ((obs.bullets[..., 0] + 192.0) / CELL).floor().clamp(0, GRID_W - 1).long()
        row = (obs.bullets[..., 1] / CELL).floor().clamp(0, GRID_H - 1).long()
        cells = GRID_H * GRID_W
        flat = (torch.arange(n, device=dev)[:, None] * cells + row * GRID_W + col).reshape(-1)
        unit = pb / pb.norm(dim=-1, keepdim=True).clamp_min(1e-6)
        approach = (-(unit * obs.bullets[..., 2:4]).sum(-1)).clamp_min(0.0) / 8.0
        count = torch.zeros(n * cells, device=dev).scatter_add_(0, flat, valid.reshape(-1))
        appr = torch.zeros(n * cells, device=dev).scatter_add_(0, flat, (approach * valid).reshape(-1))
        density = torch.stack([count.view(n, GRID_H, GRID_W), appr.view(n, GRID_H, GRID_W)], dim=1)

        player = torch.stack([obs.player_xy[:, 0] / 192.0, obs.player_xy[:, 1] / 192.0,
                              obs.player_focus.to(torch.float32)], dim=-1)
        d = obs.target_xy - obs.player_xy
        dn = d.norm(dim=-1)
        cond = torch.stack([d[:, 0] / 192.0, d[:, 1] / 192.0, dn / 448.0, (dn < self.hold_r).to(torch.float32)], dim=-1)
        return {"bullets": bullets, "bullets_mask": bsel, "enemies": enemies, "enemies_mask": esel,
                "density": density, "player": player, "cond": cond}
```

`registry.py` 的 `BUILTIN_MODULES` 改为：

```python
BUILTIN_MODULES: list[str] = [
    "stgtrain.intent",
    "stgtrain.featurize.danger_topk_v1",
]
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: 特征化器 danger_topk_v1——闭式最近接近距离 top-K 弹/敌、双通道密度图、自机与条件向量

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: reward 项 + 逐局统计

**Files:**
- Create: `src/stgtrain/reward.py`, `src/stgtrain/episodes.py`
- Modify: `src/stgtrain/registry.py`（`BUILTIN_MODULES` 追加 `"stgtrain.reward"`）、`tests/conftest.py`（加 `step_info`）
- Test: `tests/test_reward.py`, `tests/test_episodes.py`

**Interfaces:**
- Consumes: `RawObs`、`StepInfo`（Task 4）；`actions.key_changes / direction_changed`（Task 3）；`REWARD_TERMS`（Task 1）
- Produces:
  - `RewardContext(prev: RawObs, cur: RawObs, info: StepInfo, gamma: float, hold_radius: float, edge_margin: float)`，属性 `alive -> Tensor float`，方法 `dist_prev()`、`dist_next()`（都对 `prev.target_xy`）
  - 注册的 reward 项（函数 `(ctx) -> Tensor (N,) float`，**未加权**）：`death`（死亡 −1）、`follow_shaping`、`hold`、`segment_survived`（+1）、`key_press`（新按下键数）、`shift_toggle`（0/1）、`edge_hug`（贴边 +1；惩罚用负系数）
  - `RewardFn(cfg: dict)`：`__call__(prev: RawObs, cur: RawObs, info: StepInfo) -> tuple[Tensor total (N,), dict[str, Tensor] raw]`；属性 `terms: dict[str, float]`
  - `EpisodeTracker(n: int, device, term_names: list[str], hold_radius: float, edge_margin: float, frame_skip: int, reach_cap_frames: int)`：`update(prev, cur, info, total, raw_terms) -> None`、`pop_finished() -> list[dict]`
  - 局记录键：`env, done, frames, return, steps, in_r_frac, edge_frac, shift_toggles_per_s, dir_changes_per_s, reach_frames, term/<name>`
  - `tests/conftest.step_info(n=2, done=None, refreshed=None, buttons=None, prev_buttons=None, ep_frames=100) -> StepInfo`

- [ ] **Step 1: conftest 加 `step_info`**

在 `tests/conftest.py` 末尾追加：

```python
from stgagent import consts as C

from stgtrain.envwrap import StepInfo


def step_info(n=2, done=None, refreshed=None, buttons=None, prev_buttons=None, ep_frames=100) -> StepInfo:
    def i64(v, default):
        return torch.tensor(v, dtype=torch.int64) if v is not None else torch.full((n,), default, dtype=torch.int64)

    return StepInfo(
        done=i64(done, 0),
        events=torch.zeros(n, 8, dtype=torch.int64),
        ep_frames=torch.full((n,), ep_frames, dtype=torch.int64),
        refreshed=torch.tensor(refreshed, dtype=torch.bool) if refreshed is not None else torch.zeros(n, dtype=torch.bool),
        buttons=i64(buttons, C.BTN_SHOT),
        prev_buttons=i64(prev_buttons, C.BTN_SHOT),
    )
```

- [ ] **Step 2: 写 reward 失败测试**

`tests/test_reward.py`：

```python
import pytest
import torch
from stgagent import consts as C

from conftest import raw_obs, small_cfg, step_info
from stgtrain.registry import REWARD_TERMS, load_builtins
from stgtrain.reward import RewardContext, RewardFn

load_builtins()


def ctx(prev, cur, info, gamma=1.0):
    return RewardContext(prev, cur, info, gamma=gamma, hold_radius=24.0, edge_margin=16.0)


def term(name, *a, **k):
    return REWARD_TERMS.get(name)(ctx(*a, **k))


def test_death_and_segment_survived():
    o = raw_obs(n=3)
    info = step_info(n=3, done=[0, 1, 2])
    assert term("death", o, o, info).tolist() == [0.0, -1.0, 0.0]
    assert term("segment_survived", o, o, info).tolist() == [0.0, 0.0, 1.0]


def test_follow_shaping_telescopes_across_refresh():
    # γ=1：一局内塑形总和 = 各刷新段 Φ(段末) − Φ(段首)（计划 Ruling 1：刷新步用旧目标精确计算）
    path = [(0.0, 400.0), (10.0, 380.0), (20.0, 350.0), (-5.0, 330.0), (-30.0, 320.0), (-40.0, 300.0)]
    targets = [(0.0, 300.0)] * 3 + [(-50.0, 260.0)] * 3  # 第 2 步（0 起）之后刷新
    phi = lambda p, t: -((p[0] - t[0]) ** 2 + (p[1] - t[1]) ** 2) ** 0.5 / 448.0
    total = 0.0
    for s in range(len(path) - 1):
        prev = raw_obs(n=1, player=path[s], target=targets[s])
        cur = raw_obs(n=1, player=path[s + 1], target=targets[s + 1])
        total += term("follow_shaping", prev, cur, step_info(n=1)).item()
    expected = (phi(path[3], targets[0]) - phi(path[0], targets[0])) + (phi(path[5], targets[3]) - phi(path[3], targets[3]))
    assert total == pytest.approx(expected, abs=1e-5)


def test_follow_shaping_zero_on_terminal_step():
    prev = raw_obs(n=2, player=(150.0, 440.0), target=(-150.0, 230.0))
    cur = raw_obs(n=2, player=(0.0, 384.0), target=(0.0, 300.0))
    v = term("follow_shaping", prev, cur, step_info(n=2, done=[1, 3]), gamma=0.995)
    assert v.tolist() == [0.0, 0.0]


def test_hold_and_edge_hug():
    prev = raw_obs(n=3, target=(0.0, 300.0))
    cur = raw_obs(n=3, player=[(0.0, 310.0), (185.0, 300.0), (0.0, 310.0)])
    info = step_info(n=3, done=[0, 0, 1])
    assert term("hold", prev, cur, info).tolist() == [1.0, 0.0, 0.0]
    assert term("edge_hug", prev, cur, info).tolist() == [0.0, 1.0, 0.0]


def test_key_terms():
    o = raw_obs(n=2)
    info = step_info(n=2, prev_buttons=[C.BTN_SHOT, C.BTN_SHOT | C.BTN_SLOW],
                     buttons=[C.BTN_SHOT | C.BTN_UP | C.BTN_SLOW, C.BTN_SHOT])
    assert term("key_press", o, o, info).tolist() == [2.0, 0.0]
    assert term("shift_toggle", o, o, info).tolist() == [1.0, 1.0]


def test_reward_fn_weights_and_validation():
    cfg = small_cfg(reward={"terms": {"death": 10.0, "follow_shaping": 1.0, "hold": 0.5, "segment_survived": 0.0,
                                       "key_press": 0.0, "shift_toggle": 0.0, "edge_hug": -0.1}})
    fn = RewardFn(cfg)
    prev = raw_obs(n=2, target=(0.0, 300.0), player=(0.0, 320.0))
    cur = raw_obs(n=2, player=[(0.0, 310.0), (0.0, 310.0)])
    total, raw = fn(prev, cur, step_info(n=2, done=[0, 1]))
    manual = sum(fn.terms[k] * raw[k] for k in raw)
    assert torch.allclose(total, manual)
    assert total[1].item() == pytest.approx(-10.0)
    with pytest.raises(ValueError, match="未知"):
        RewardFn(small_cfg(reward={"terms": {"nope": 1.0}}))
    with pytest.raises(ValueError, match="防自杀"):
        RewardFn(small_cfg(reward={"terms": {"death": 1.0, "follow_shaping": 1.0}}))


def test_rollout_features_and_rewards_are_deterministic():
    """spec §8：同种子、不带学习的 rollout，特征与 reward 逐字节相同。"""
    import stg_rl

    from conftest import FIXTURES
    from stgtrain.cards import compile_cards, discover
    from stgtrain.envwrap import EnvWrapper
    from stgtrain.registry import FEATURIZERS

    cfg = small_cfg(env={"mirror": True})
    images = compile_cards(discover(FIXTURES / "cards"))
    feat = FEATURIZERS.get("danger_topk_v1")(cfg)
    fn = RewardFn(cfg)

    def run():
        w = EnvWrapper(cfg, images, [stg_rl.Start("example_ring", 0, 2)], torch.device("cpu"), seed=21)
        obs = w.reset()
        g = torch.Generator().manual_seed(5)
        feats, rewards = [], []
        for _ in range(80):
            nxt, info = w.step(torch.randint(0, 18, (w.n,), generator=g))
            total, _ = fn(obs, nxt, info)
            feats.append(feat(nxt))
            rewards.append(total)
            obs = nxt
        return feats, rewards

    fa, ra = run()
    fb, rb = run()
    for a, b in zip(fa, fb):
        for k in a:
            assert torch.equal(a[k], b[k]), k
    assert all(torch.equal(x, y) for x, y in zip(ra, rb))
```

Run: `uv run --frozen pytest tests/test_reward.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.reward`）

- [ ] **Step 3: 实现 `reward.py` 并登记**

```python
"""reward 项（spec §3.5 + 计划 Ruling 1）。每项返回未加权的 (N,) float；RewardFn 按配置系数加权求和。

符号约定：death 返回 −1（系数取正），edge_hug / key_press / shift_toggle 返回正计数（要惩罚就配负系数）。
"""
from __future__ import annotations

from dataclasses import dataclass

import torch
from torch import Tensor

from . import actions
from .envwrap import RawObs, StepInfo
from .registry import REWARD_TERMS

FIELD_HALF_W, FIELD_H, DIST_NORM = 192.0, 448.0, 448.0


@dataclass
class RewardContext:
    prev: RawObs
    cur: RawObs
    info: StepInfo
    gamma: float
    hold_radius: float
    edge_margin: float

    @property
    def alive(self) -> Tensor:
        return (self.info.done == 0).to(torch.float32)

    def dist_prev(self) -> Tensor:
        return (self.prev.player_xy - self.prev.target_xy).norm(dim=-1)

    def dist_next(self) -> Tensor:
        # 对本步动作所瞄的旧目标：刷新发生在 reward 之后，所以不会有目标突变造成的跳变
        return (self.cur.player_xy - self.prev.target_xy).norm(dim=-1)


@REWARD_TERMS.register("death")
def death(ctx: RewardContext) -> Tensor:
    return -(ctx.info.done == 1).to(torch.float32)


@REWARD_TERMS.register("follow_shaping")
def follow_shaping(ctx: RewardContext) -> Tensor:
    """γ·Φ(s') − Φ(s)，Φ = −d/448。终局步记 0：cur 已是新局，且令终态 Φ=0 会让「远离时死亡」白赚。"""
    phi_prev = -ctx.dist_prev() / DIST_NORM
    phi_next = -ctx.dist_next() / DIST_NORM
    return (ctx.gamma * phi_next - phi_prev) * ctx.alive


@REWARD_TERMS.register("hold")
def hold(ctx: RewardContext) -> Tensor:
    return (ctx.dist_next() < ctx.hold_radius).to(torch.float32) * ctx.alive


@REWARD_TERMS.register("segment_survived")
def segment_survived(ctx: RewardContext) -> Tensor:
    return (ctx.info.done == 2).to(torch.float32)


@REWARD_TERMS.register("key_press")
def key_press(ctx: RewardContext) -> Tensor:
    return actions.key_changes(ctx.info.prev_buttons, ctx.info.buttons)[0].to(torch.float32)


@REWARD_TERMS.register("shift_toggle")
def shift_toggle(ctx: RewardContext) -> Tensor:
    return actions.key_changes(ctx.info.prev_buttons, ctx.info.buttons)[1].to(torch.float32)


@REWARD_TERMS.register("edge_hug")
def edge_hug(ctx: RewardContext) -> Tensor:
    m = ctx.edge_margin
    x, y = ctx.cur.player_xy[:, 0].abs(), ctx.cur.player_xy[:, 1]
    return ((x > FIELD_HALF_W - m) | (y > FIELD_H - m) | (y < m)).to(torch.float32) * ctx.alive


class RewardFn:
    def __init__(self, cfg: dict):
        terms = {k: float(v) for k, v in cfg["reward"]["terms"].items()}
        self.fns = {name: REWARD_TERMS.get(name) for name in terms}  # 未知名 ⇒ ValueError
        follow, death_c = terms.get("follow_shaping", 0.0), terms.get("death", 0.0)
        if follow > 0 and death_c < 5.0 * follow * 1.1:
            raise ValueError(
                f"reward.terms.death={death_c} 须 ≥ 5 × follow_shaping × 1.1 = {5.0 * follow * 1.1:.2f}（防自杀，spec §3.5）"
            )
        self.terms = terms
        self.gamma = float(cfg["ppo"]["gamma"])
        self.hold_radius = float(cfg["reward"]["hold_radius"])
        self.edge_margin = float(cfg["reward"]["edge_margin"])

    def __call__(self, prev: RawObs, cur: RawObs, info: StepInfo) -> tuple[Tensor, dict[str, Tensor]]:
        ctx = RewardContext(prev, cur, info, self.gamma, self.hold_radius, self.edge_margin)
        raw = {name: fn(ctx) for name, fn in self.fns.items()}
        total = torch.zeros_like(info.done, dtype=torch.float32)
        for name, value in raw.items():
            total = total + self.terms[name] * value
        return total, raw
```

注意 `Registry.get` 的未知名错误消息是「未知的 reward term ...」，测试 `match="未知"` 据此通过。

`registry.py` 的 `BUILTIN_MODULES` 改为：

```python
BUILTIN_MODULES: list[str] = [
    "stgtrain.intent",
    "stgtrain.featurize.danger_topk_v1",
    "stgtrain.reward",
]
```

Run: `uv run --frozen pytest tests/test_reward.py -q`
Expected: PASS

- [ ] **Step 4: 写逐局统计失败测试**

`tests/test_episodes.py`：

```python
import pytest
import torch
from stgagent import consts as C

from conftest import raw_obs, step_info
from stgtrain.episodes import EpisodeTracker


def tracker(n=2, fs=1, cap=300):
    return EpisodeTracker(n, torch.device("cpu"), ["death", "hold"], hold_radius=24.0, edge_margin=16.0,
                          frame_skip=fs, reach_cap_frames=cap)


def feed(t, prev, cur, info, total):
    raw = {"death": -(info.done == 1).float(), "hold": torch.zeros(prev.player_xy.shape[0])}
    t.update(prev, cur, info, torch.tensor(total, dtype=torch.float32), raw)


def test_episode_record_fields_and_reset():
    t = tracker(fs=2)
    target = (0.0, 300.0)
    inside, outside = (0.0, 305.0), (0.0, 400.0)
    # env0：外 → 内 → 内 → 死；env1 一直在外不结束
    seq = [(outside, [0, 0]), (inside, [0, 0]), (inside, [0, 0]), (inside, [1, 0])]
    prev = raw_obs(n=2, player=[outside, outside], target=target)
    for pos, done in seq:
        cur = raw_obs(n=2, player=[pos, outside], target=target)
        feed(t, prev, cur, step_info(n=2, done=done, ep_frames=8), [1.0, 0.5])
        prev = cur
    recs = t.pop_finished()
    assert len(recs) == 1
    r = recs[0]
    assert (r["env"], r["done"], r["frames"], r["steps"]) == (0, 1, 8, 4)
    assert r["return"] == pytest.approx(4.0)
    assert r["in_r_frac"] == pytest.approx(2 / 4), "第 2、3 步在 R 内；死亡步不计"
    assert r["reach_frames"] == pytest.approx(2 * 2), "第 2 步到达 × frame_skip 2"
    assert r["term/death"] == pytest.approx(-1.0)
    assert t.pop_finished() == []
    # env0 已清零：再走一步结束，return 只含这一步
    cur = raw_obs(n=2, player=[outside, outside], target=target)
    feed(t, prev, cur, step_info(n=2, done=[2, 0]), [3.0, 0.0])
    r2 = t.pop_finished()[0]
    assert r2["steps"] == 1 and r2["return"] == pytest.approx(3.0)


def test_unreached_segment_counts_cap_and_key_rates():
    t = tracker(n=1, fs=1, cap=300)
    far = raw_obs(n=1, player=(0.0, 440.0), target=(0.0, 250.0))
    feed(t, far, far, step_info(n=1, refreshed=[True], prev_buttons=[C.BTN_SHOT], buttons=[C.BTN_SHOT | C.BTN_SLOW]), [0.0])
    feed(t, far, far, step_info(n=1, done=[3], prev_buttons=[C.BTN_SHOT | C.BTN_SLOW], buttons=[C.BTN_SHOT | C.BTN_LEFT]), [0.0])
    r = t.pop_finished()[0]
    assert r["reach_frames"] == pytest.approx(300.0), "两段都没到达，各记上限"
    assert r["shift_toggles_per_s"] == pytest.approx(2 / (2 / 60))
    assert r["dir_changes_per_s"] == pytest.approx(1 / (2 / 60))
```

Run: `uv run --frozen pytest tests/test_episodes.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.episodes`）

- [ ] **Step 5: 实现 `episodes.py`**

```python
"""逐局统计累加器（训练 metrics 与评测共用；计划 Ruling 2）。

update 全程留在设备上不同步；pop_finished 每轮 rollout 调一次，把结束的局一次性搬回 CPU。
"""
from __future__ import annotations

import torch
from torch import Tensor

from . import actions
from .envwrap import RawObs, StepInfo

_COLS = ("return", "steps", "in_r", "edge", "shift", "dirchg", "reach_sum", "reach_cnt")


class EpisodeTracker:
    def __init__(self, n: int, device, term_names: list[str], hold_radius: float, edge_margin: float,
                 frame_skip: int, reach_cap_frames: int):
        self.n, self.device = int(n), device
        self.term_names = list(term_names)
        self.hold_radius, self.edge_margin = float(hold_radius), float(edge_margin)
        self.frame_skip, self.reach_cap = int(frame_skip), float(reach_cap_frames)
        self.acc = torch.zeros(self.n, len(_COLS) + len(self.term_names), device=device)
        self.since = torch.zeros(self.n, device=device)
        self.reached = torch.zeros(self.n, dtype=torch.bool, device=device)
        self._pending: list[tuple[Tensor, Tensor, Tensor, Tensor]] = []

    def update(self, prev: RawObs, cur: RawObs, info: StepInfo, total: Tensor, raw_terms: dict[str, Tensor]) -> None:
        alive = info.done == 0
        d = (cur.player_xy - prev.target_xy).norm(dim=-1)
        in_r = (d < self.hold_radius) & alive
        m = self.edge_margin
        x, y = cur.player_xy[:, 0].abs(), cur.player_xy[:, 1]
        edge = ((x > 192.0 - m) | (y > 448.0 - m) | (y < m)) & alive
        _, toggled = actions.key_changes(info.prev_buttons, info.buttons)
        dir_chg = actions.direction_changed(info.prev_buttons, info.buttons)

        self.since = self.since + 1
        newly = in_r & ~self.reached
        reach_add = newly.float() * self.since * self.frame_skip
        reach_cnt = newly.float()
        self.reached = self.reached | newly
        seg_end = info.refreshed | ~alive
        miss = seg_end & ~self.reached
        reach_add = reach_add + miss.float() * self.reach_cap
        reach_cnt = reach_cnt + miss.float()

        cols = [total.to(torch.float32), torch.ones_like(total, dtype=torch.float32), in_r.float(), edge.float(),
                toggled.float(), dir_chg.float(), reach_add, reach_cnt]
        cols += [raw_terms[name].to(torch.float32) for name in self.term_names]
        self.acc = self.acc + torch.stack(cols, dim=-1)

        ended = ~alive
        self._pending.append((ended, self.acc.clone(), info.done, info.ep_frames))
        self.acc = torch.where(ended[:, None], torch.zeros_like(self.acc), self.acc)
        self.since = torch.where(seg_end, torch.zeros_like(self.since), self.since)
        self.reached = self.reached & ~seg_end

    def pop_finished(self) -> list[dict]:
        if not self._pending:
            return []
        ended = torch.stack([p[0] for p in self._pending]).cpu()
        acc = torch.stack([p[1] for p in self._pending]).cpu()
        done = torch.stack([p[2] for p in self._pending]).cpu()
        frames = torch.stack([p[3] for p in self._pending]).cpu()
        self._pending.clear()
        out: list[dict] = []
        for t, i in ended.nonzero().tolist():
            a = acc[t, i].tolist()
            steps = max(a[1], 1.0)
            secs = steps * self.frame_skip / 60.0
            rec = {
                "env": i, "done": int(done[t, i]), "frames": int(frames[t, i]), "return": a[0], "steps": int(a[1]),
                "in_r_frac": a[2] / steps, "edge_frac": a[3] / steps,
                "shift_toggles_per_s": a[4] / secs, "dir_changes_per_s": a[5] / secs,
                "reach_frames": a[6] / max(a[7], 1.0),
            }
            for k, name in enumerate(self.term_names):
                rec[f"term/{name}"] = a[len(_COLS) + k]
            out.append(rec)
        return out
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: reward 七项（势函数遵从塑形用旧目标精确算、终局记 0、防自杀校验）+ 逐局统计累加器

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: 模型 `set_attn_v1`

**Files:**
- Create: `src/stgtrain/models/__init__.py`, `src/stgtrain/models/set_attn_v1.py`
- Modify: `src/stgtrain/registry.py`（`BUILTIN_MODULES` 追加 `"stgtrain.models.set_attn_v1"`）
- Test: `tests/test_model.py`

**Interfaces:**
- Consumes: `FEATURIZERS["danger_topk_v1"]` 的 `spec()` 与输出（Task 5）；`actions.NUM_ACTIONS`（Task 3）；`check_compat`（Task 1）
- Produces:
  - `layer_init(layer, std=sqrt(2), bias_const=0.0) -> layer`（同 LeanRL）
  - `SetEncoder(f_in: int, d: int, heads: int, ctx_dim: int)`：`forward(x (N,K,F), mask (N,K) bool, ctx (N,ctx_dim)) -> (N, 3d)`；属性 `out_dim = 3d`
  - 注册名 `"set_attn_v1"` → `SetAttnV1(cfg: dict, spec: dict[str, tuple[int, ...]])`（`nn.Module`）：`requires() -> dict[str, tuple[int, ...]]`、`forward(feats: Mapping[str, Tensor]) -> tuple[Tensor logits (N,18), Tensor value (N,)]`

- [ ] **Step 1: 写失败测试**

`tests/test_model.py`：

```python
import pytest
import torch

from conftest import raw_obs, small_cfg
from stgtrain.registry import FEATURIZERS, MODELS, check_compat, load_builtins

load_builtins()


def build(**model):
    cfg = small_cfg(model=model) if model else small_cfg()
    feat = FEATURIZERS.get(cfg["featurize"]["name"])(cfg)
    net = MODELS.get(cfg["model"]["name"])(cfg, feat.spec())
    return cfg, feat, net


def busy_obs(n=3):
    rows = [(float(10 * j - 40), 300.0 - 7 * j, 0.5 * j, 2.0, 2.0) for j in range(9)]
    return raw_obs(n=n, player=(5.0, 390.0), target=(-40.0, 280.0), bullets=[rows] * n,
                   enemies=[[(0.0, 100.0, 16.0, 1.0)]] * n)


def test_shapes_and_requires():
    _, feat, net = build()
    check_compat(net.requires(), feat.spec())
    logits, value = net(feat(busy_obs()))
    assert logits.shape == (3, 18) and value.shape == (3,)


def test_empty_masks_are_finite():
    _, feat, net = build()
    logits, value = net(feat(raw_obs(n=2)))
    assert torch.isfinite(logits).all() and torch.isfinite(value).all()


def test_permutation_invariance_over_bullets():
    _, feat, net = build()
    f = feat(busy_obs(n=1))
    perm = torch.randperm(f["bullets"].shape[1], generator=torch.Generator().manual_seed(1))
    g = dict(f)
    g["bullets"], g["bullets_mask"] = f["bullets"][:, perm], f["bullets_mask"][:, perm]
    la, va = net(f)
    lb, vb = net(g)
    assert torch.allclose(la, lb, atol=1e-5) and torch.allclose(va, vb, atol=1e-5)


def test_gradients_reach_every_parameter():
    _, feat, net = build()
    logits, value = net(feat(busy_obs()))
    (logits.sum() + value.sum()).backward()
    missing = [n for n, p in net.named_parameters() if p.grad is None]
    assert missing == []


def test_heads_must_divide_d():
    with pytest.raises(ValueError, match="整除"):
        build(d=15, heads=2)
```

Run: `uv run --frozen pytest tests/test_model.py -q`
Expected: FAIL（`ValueError: 未知的 model 'set_attn_v1'`）

- [ ] **Step 2: 实现**

`src/stgtrain/models/__init__.py`：

```python
"""模型实现（每个文件注册一个名字）。新模型 = 新文件 + @MODELS.register + 新配置，不改 ppo.py。"""
```

`src/stgtrain/models/set_attn_v1.py`：

```python
"""模型 set_attn_v1（spec §4.1）：弹 / 敌各一个集合编码器 + 密度图小卷积 + 主干 MLP，策略头 18 路、价值头 1 路。

全空掩码安全：池化与注意力对「一行都没有」输出 0，不出 NaN。形状全固定，ONNX 友好。
"""
from __future__ import annotations

import math
from typing import Mapping

import torch
from torch import Tensor, nn

from ..actions import NUM_ACTIONS
from ..registry import MODELS

_KEYS = ("bullets", "bullets_mask", "enemies", "enemies_mask", "density", "player", "cond")


def layer_init(layer, std=math.sqrt(2), bias_const=0.0):
    torch.nn.init.orthogonal_(layer.weight, std)
    torch.nn.init.constant_(layer.bias, bias_const)
    return layer


class SetEncoder(nn.Module):
    def __init__(self, f_in: int, d: int, heads: int, ctx_dim: int):
        super().__init__()
        self.phi = nn.Sequential(layer_init(nn.Linear(f_in, d)), nn.ReLU(), layer_init(nn.Linear(d, d)), nn.ReLU())
        self.heads, self.dk = heads, d // heads
        self.q = layer_init(nn.Linear(ctx_dim, d))
        self.k = layer_init(nn.Linear(d, d))
        self.v = layer_init(nn.Linear(d, d))
        self.out_dim = 3 * d

    def forward(self, x: Tensor, mask: Tensor, ctx: Tensor) -> Tensor:
        n, k, _ = x.shape
        h = self.phi(x)
        m = mask.unsqueeze(-1).to(h.dtype)
        cnt = m.sum(1)
        any_ = (cnt > 0).to(h.dtype)
        mean = (h * m).sum(1) / cnt.clamp_min(1.0)
        mx = h.masked_fill(~mask.unsqueeze(-1), -1e4).max(1).values * any_
        q = self.q(ctx).view(n, self.heads, 1, self.dk)
        kk = self.k(h).view(n, k, self.heads, self.dk).transpose(1, 2)
        vv = self.v(h).view(n, k, self.heads, self.dk).transpose(1, 2)
        scores = (q @ kk.transpose(-1, -2)) / math.sqrt(self.dk)
        scores = scores.masked_fill(~mask[:, None, None, :], -1e4)
        attn = (scores.softmax(-1) @ vv).reshape(n, -1) * any_
        return torch.cat([mean, mx, attn], dim=-1)


@MODELS.register("set_attn_v1")
class SetAttnV1(nn.Module):
    def __init__(self, cfg: dict, spec: dict[str, tuple[int, ...]]):
        super().__init__()
        m = cfg["model"]
        d, heads, trunk = int(m["d"]), int(m["heads"]), int(m["trunk"])
        if d % heads:
            raise ValueError(f"model.d({d}) 须能被 model.heads({heads}) 整除")
        self._spec = {k: tuple(spec[k]) for k in _KEYS}
        f_b, f_e = spec["bullets"][1], spec["enemies"][1]
        f_p, f_c = spec["player"][0], spec["cond"][0]
        c_d, h_d, w_d = spec["density"]
        self.ctx = nn.Sequential(layer_init(nn.Linear(f_p + f_c, d)), nn.ReLU())
        self.bullets = SetEncoder(f_b, d, heads, d)
        self.enemies = SetEncoder(f_e, d, heads, d)
        self.density = nn.Sequential(
            layer_init(nn.Conv2d(c_d, 16, 3, padding=1)), nn.ReLU(),
            layer_init(nn.Conv2d(16, 16, 3, stride=2, padding=1)), nn.ReLU(), nn.Flatten(),
        )
        dens_out = 16 * ((h_d + 1) // 2) * ((w_d + 1) // 2)
        self.dens_proj = nn.Sequential(layer_init(nn.Linear(dens_out, d)), nn.ReLU())
        in_dim = self.bullets.out_dim + self.enemies.out_dim + 2 * d
        self.trunk = nn.Sequential(layer_init(nn.Linear(in_dim, trunk)), nn.ReLU(),
                                   layer_init(nn.Linear(trunk, trunk)), nn.ReLU())
        self.actor = layer_init(nn.Linear(trunk, NUM_ACTIONS), std=0.01)
        self.critic = layer_init(nn.Linear(trunk, 1), std=1.0)

    def requires(self) -> dict[str, tuple[int, ...]]:
        return dict(self._spec)

    def forward(self, feats: Mapping[str, Tensor]) -> tuple[Tensor, Tensor]:
        ctx = self.ctx(torch.cat([feats["player"], feats["cond"]], dim=-1))
        hb = self.bullets(feats["bullets"], feats["bullets_mask"], ctx)
        he = self.enemies(feats["enemies"], feats["enemies_mask"], ctx)
        hd = self.dens_proj(self.density(feats["density"]))
        h = self.trunk(torch.cat([hb, he, hd, ctx], dim=-1))
        return self.actor(h), self.critic(h).squeeze(-1)
```

`registry.py` 的 `BUILTIN_MODULES` 改为：

```python
BUILTIN_MODULES: list[str] = [
    "stgtrain.intent",
    "stgtrain.featurize.danger_topk_v1",
    "stgtrain.reward",
    "stgtrain.models.set_attn_v1",
]
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: 模型 set_attn_v1——弹/敌集合编码器（掩码 mean‖max + 条件交叉注意力）+ 密度卷积 + 双头，全空掩码安全

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: 指标记录 + 出图

**Files:**
- Create: `src/stgtrain/metrics.py`, `src/stgtrain/plots.py`
- Test: `tests/test_metrics.py`, `tests/test_plots.py`

**Interfaces:**
- Consumes: 逐局记录 dict（Task 6 `EpisodeTracker.pop_finished` 的格式）
- Produces:
  - `MetricsLogger(run_dir: Path, tensorboard: bool)`：`log(update: int, env_steps: int, scalars: dict[str, float]) -> None`、`close() -> None`；写 `run_dir/metrics.jsonl`（每行含 `update, env_steps, wall` 与各标量，非有限值写 `null`），TensorBoard 写 `run_dir/tb/`，横轴 `env_steps`
  - `read_jsonl(path) -> list[dict]`
  - `summarize_episodes(records: list[dict], prefix: str = "ep/") -> dict[str, float]`：`<prefix>count`、`<prefix>done1/2/3`（占比）、其余数值字段取均值（跳过 `env`、`done`）
  - `plots.group_of(key: str) -> str`、`plots.load_run(path: str | Path) -> list[dict]`（目录或 `.tar.gz`，读其中的 `metrics.jsonl`）、`plots.load_perf(path) -> list[dict]`（读 `perf.jsonl` 里 `kind == "load"` 的行）、`plots.plot_runs(runs: dict[str, list[dict]], out_dir: Path) -> list[Path]`、`plots.plot_load(rows: list[dict], out_dir: Path) -> Path | None`、`plots.main(argv: list[str] | None = None) -> int`

- [ ] **Step 1: 写失败测试**

`tests/test_metrics.py`：

```python
import math

import pytest

from stgtrain.metrics import MetricsLogger, read_jsonl, summarize_episodes


def test_logger_writes_jsonl_and_tensorboard(tmp_path):
    lg = MetricsLogger(tmp_path, tensorboard=True)
    lg.log(1, 128, {"ppo/pg_loss": 0.5, "ep/return": float("nan")})
    lg.log(2, 256, {"ppo/pg_loss": 0.25})
    lg.close()
    rows = read_jsonl(tmp_path / "metrics.jsonl")
    assert [r["update"] for r in rows] == [1, 2] and rows[1]["env_steps"] == 256
    assert rows[0]["ep/return"] is None and rows[0]["ppo/pg_loss"] == 0.5
    assert "wall" in rows[0]
    assert any((tmp_path / "tb").iterdir())


def test_summarize_episodes():
    recs = [{"env": 0, "done": 1, "return": 1.0, "frames": 100},
            {"env": 3, "done": 2, "return": 3.0, "frames": 300}]
    s = summarize_episodes(recs)
    assert s["ep/count"] == 2 and s["ep/done1"] == 0.5 and s["ep/done2"] == 0.5 and s["ep/done3"] == 0.0
    assert s["ep/return"] == pytest.approx(2.0) and s["ep/frames"] == pytest.approx(200.0)
    assert "ep/env" not in s
    assert summarize_episodes([]) == {}
```

`tests/test_plots.py`：

```python
import json
import tarfile

from stgtrain import plots


def write_rows(d, n=5):
    d.mkdir(parents=True, exist_ok=True)
    with open(d / "metrics.jsonl", "w") as f:
        for u in range(1, n + 1):
            f.write(json.dumps({"update": u, "env_steps": u * 100, "wall": float(u), "ppo/pg_loss": 1.0 / u,
                                "ep/return": float(u), "ep/term/death": -0.1 * u, "eval/survival": None}) + "\n")
    with open(d / "perf.jsonl", "w") as f:
        for t in range(3):
            f.write(json.dumps({"kind": "load", "wall": float(t), "cpu_percent": 50.0, "rss_mb": 900.0}) + "\n")
            f.write(json.dumps({"kind": "phase", "update": t, "env_step": 0.1}) + "\n")


def test_group_of():
    assert plots.group_of("ppo/pg_loss") == "ppo"
    assert plots.group_of("ep/term/death") == "ep/term"
    assert plots.group_of("ep/return") == "ep"
    assert plots.group_of("lr") == "misc"


def test_plot_runs_and_load(tmp_path):
    a, b = tmp_path / "a", tmp_path / "b"
    write_rows(a)
    write_rows(b, n=3)
    out = plots.plot_runs({"a": plots.load_run(a), "b": plots.load_run(b)}, tmp_path / "plots")
    names = sorted(p.name for p in out)
    assert names == ["ep.png", "ep_term.png", "ppo.png"], "全为 null 的 eval/survival 不出图"
    assert all(p.stat().st_size > 0 for p in out)
    load = plots.plot_load(plots.load_perf(a / "perf.jsonl"), tmp_path / "plots")
    assert load is not None and load.exists()


def test_load_run_from_tar_and_cli(tmp_path):
    run = tmp_path / "20260915-000000-x"
    write_rows(run)
    tgz = tmp_path / "x.tar.gz"
    with tarfile.open(tgz, "w:gz") as tf:
        tf.add(run, arcname=run.name)
    assert len(plots.load_run(tgz)) == 5
    out = tmp_path / "cli"
    assert plots.main([str(tgz), str(run), "--out", str(out)]) == 0
    assert (out / "ppo.png").exists()
```

Run: `uv run --frozen pytest tests/test_metrics.py tests/test_plots.py -q`
Expected: FAIL（`ModuleNotFoundError`）

- [ ] **Step 2: 实现 `metrics.py`**

```python
"""标量记录（spec §7.1）：metrics.jsonl 为权威，TensorBoard 为同名镜像。"""
from __future__ import annotations

import json
import math
import time
from pathlib import Path


class MetricsLogger:
    def __init__(self, run_dir: Path, tensorboard: bool):
        self.run_dir = Path(run_dir)
        self.path = self.run_dir / "metrics.jsonl"
        self._f = open(self.path, "a", encoding="utf-8")
        self._t0 = time.time()
        self.tb = None
        if tensorboard:
            from torch.utils.tensorboard import SummaryWriter

            self.tb = SummaryWriter(str(self.run_dir / "tb"))

    def log(self, update: int, env_steps: int, scalars: dict[str, float]) -> None:
        row: dict = {"update": int(update), "env_steps": int(env_steps), "wall": round(time.time() - self._t0, 3)}
        for k, v in scalars.items():
            v = float(v)
            row[k] = v if math.isfinite(v) else None
            if self.tb is not None and math.isfinite(v):
                self.tb.add_scalar(k, v, int(env_steps))
        self._f.write(json.dumps(row, ensure_ascii=False) + "\n")
        self._f.flush()

    def close(self) -> None:
        self._f.close()
        if self.tb is not None:
            self.tb.close()


def read_jsonl(path) -> list[dict]:
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def summarize_episodes(records: list[dict], prefix: str = "ep/") -> dict[str, float]:
    if not records:
        return {}
    n = len(records)
    out: dict[str, float] = {f"{prefix}count": float(n)}
    for code in (1, 2, 3):
        out[f"{prefix}done{code}"] = sum(1 for r in records if r["done"] == code) / n
    for key in records[0]:
        if key in ("env", "done"):
            continue
        out[f"{prefix}{key}"] = sum(float(r[key]) for r in records) / n
    return out
```

- [ ] **Step 3: 实现 `plots.py`**

```python
"""出图（spec §7.1）：按键名前缀分组，每组一张 PNG；可对中途拷回的 jsonl 或 tar 包重画、多 run 叠加。

用法：python -m stgtrain.plots <run 目录或 .tar.gz>... [--out DIR]
"""
from __future__ import annotations

import argparse
import io
import json
import math
import tarfile
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

_AXIS_KEYS = {"update", "env_steps", "wall"}


def group_of(key: str) -> str:
    parts = key.split("/")
    if len(parts) == 1:
        return "misc"
    if parts[0] == "ep" and len(parts) > 2:
        return "ep/term" if parts[1] == "term" else "/".join(parts[:2])
    return parts[0]


def _read_lines(text: str) -> list[dict]:
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def _read_member(path: Path, name: str) -> list[dict]:
    if path.is_dir():
        f = path / name
        return _read_lines(f.read_text(encoding="utf-8")) if f.exists() else []
    with tarfile.open(path, "r:gz") as tf:
        for m in tf.getmembers():
            if m.name.endswith("/" + name) or m.name == name:
                return _read_lines(io.TextIOWrapper(tf.extractfile(m), encoding="utf-8").read())
    return []


def load_run(path) -> list[dict]:
    return _read_member(Path(path), "metrics.jsonl")


def load_perf(path) -> list[dict]:
    p = Path(path)
    rows = _read_member(p.parent, p.name) if p.name == "perf.jsonl" else _read_member(p, "perf.jsonl")
    return [r for r in rows if r.get("kind") == "load"]


def _groups(runs: dict[str, list[dict]]) -> dict[str, list[str]]:
    keys: dict[str, set[str]] = {}
    for rows in runs.values():
        for r in rows:
            for k, v in r.items():
                if k in _AXIS_KEYS or not isinstance(v, (int, float)) or isinstance(v, bool):
                    continue
                if math.isfinite(float(v)):
                    keys.setdefault(group_of(k), set()).add(k)
    return {g: sorted(ks) for g, ks in keys.items()}


def _grid(n: int):
    cols = min(3, n)
    rows = math.ceil(n / cols)
    fig, axes = plt.subplots(rows, cols, figsize=(5 * cols, 3.2 * rows), squeeze=False)
    flat = [ax for row in axes for ax in row]
    for ax in flat[n:]:
        ax.set_visible(False)
    return fig, flat[:n]


def plot_runs(runs: dict[str, list[dict]], out_dir: Path) -> list[Path]:
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    paths: list[Path] = []
    for group, keys in sorted(_groups(runs).items()):
        fig, axes = _grid(len(keys))
        for ax, key in zip(axes, keys):
            for name, rows in runs.items():
                pts = [(r["env_steps"], r[key]) for r in rows if r.get(key) is not None]
                if pts:
                    ax.plot([p[0] for p in pts], [p[1] for p in pts], label=name)
            ax.set_title(key, fontsize=9)
            ax.set_xlabel("env steps", fontsize=8)
            ax.grid(alpha=0.3)
        if len(runs) > 1:
            axes[0].legend(fontsize=7)
        fig.tight_layout()
        p = out_dir / f"{group.replace('/', '_')}.png"
        fig.savefig(p, dpi=110)
        plt.close(fig)
        paths.append(p)
    return paths


def plot_load(rows: list[dict], out_dir: Path) -> Path | None:
    if not rows:
        return None
    keys = sorted({k for r in rows for k, v in r.items()
                   if k not in ("kind", "wall") and isinstance(v, (int, float)) and not isinstance(v, bool)})
    if not keys:
        return None
    fig, axes = _grid(len(keys))
    for ax, key in zip(axes, keys):
        pts = [(r["wall"], r[key]) for r in rows if r.get(key) is not None]
        ax.plot([p[0] for p in pts], [p[1] for p in pts])
        ax.set_title(key, fontsize=9)
        ax.set_xlabel("wall s", fontsize=8)
        ax.grid(alpha=0.3)
    fig.tight_layout()
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    p = out_dir / "load.png"
    fig.savefig(p, dpi=110)
    plt.close(fig)
    return p


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m stgtrain.plots")
    ap.add_argument("runs", nargs="+", help="run 目录或 .tar.gz")
    ap.add_argument("--out", default=None, help="输出目录；缺省 = 第一个 run 目录下的 plots/（tar 包则 ./plots）")
    args = ap.parse_args(argv)
    paths = [Path(r) for r in args.runs]
    runs = {p.name.removesuffix(".tar.gz"): load_run(p) for p in paths}
    out = Path(args.out) if args.out else (paths[0] / "plots" if paths[0].is_dir() else Path("plots"))
    written = plot_runs(runs, out)
    if len(paths) == 1:
        load = plot_load(load_perf(paths[0]), out)
        if load:
            written.append(load)
    for w in written:
        print(w)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: metrics.jsonl + TensorBoard 镜像、逐局汇总；plots 按前缀分组出图（目录/tar、多 run 叠加、负载图）

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: 开销记录 `perf.py`

**Files:**
- Create: `src/stgtrain/perf.py`
- Test: `tests/test_perf.py`

**Interfaces:**
- Produces:
  - `PHASES = ("env_step", "h2d", "featurize", "policy", "reward", "update")`
  - `maybe_phase(timer, name: str)`：`timer` 为 None 时返回 `contextlib.nullcontext()`，否则 `timer.phase(name)`
  - `PerfWriter(path: Path)`：线程安全 `write(row: dict)`、`close()`
  - `PhaseTimer(sync_every: int, device: torch.device)`：`start_iteration(iteration: int)`、`phase(name)`（上下文管理器；只在 `iteration % sync_every == 0` 的迭代计时，CUDA 上前后 `synchronize`）、`pop_iteration() -> dict[str, float] | None`（键 `<phase>_s` 与 `total_s`；非计时迭代返回 None）
  - `LoadSampler(writer: PerfWriter, hz: float)`：`start()`、`stop()`、`sample() -> dict`、`summary() -> dict[str, float]`（键前缀 `load/`）；行格式 `{"kind": "load", "wall", "cpu_percent", "cpu_max_core", "cpu_per_core": [...], "rss_mb", "sys_mem_percent", 可选 "gpu<i>_util", "gpu<i>_mem_mb", "gpu<i>_power_w", "torch_max_alloc_mb"}`
  - `machine_info() -> dict`（`cpu_model, cpu_logical, cpu_physical, mem_gb, python, torch, cuda, gpus, driver`）
  - `summarize(phase_rows: list[dict], load_summary: dict, env_steps_per_iter: int) -> dict`（`iter_s, sps, phase_frac{…}, gpu_waits_cpu, cpu_waits_gpu` + load 汇总）

- [ ] **Step 1: 写失败测试**

`tests/test_perf.py`：

```python
import json
import time

import pytest
import torch

from stgtrain.perf import LoadSampler, PerfWriter, PhaseTimer, machine_info, maybe_phase, summarize


def test_phase_timer_only_times_sync_iterations():
    t = PhaseTimer(sync_every=2, device=torch.device("cpu"))
    t.start_iteration(0)
    with t.phase("env_step"):
        time.sleep(0.02)
    with t.phase("policy"):
        time.sleep(0.01)
    row = t.pop_iteration()
    assert row["env_step_s"] >= 0.018 and row["policy_s"] >= 0.008
    assert row["total_s"] >= row["env_step_s"] + row["policy_s"]
    t.start_iteration(1)
    with t.phase("env_step"):
        pass
    assert t.pop_iteration() is None


def test_maybe_phase_accepts_none():
    with maybe_phase(None, "x"):
        pass


def test_load_sampler_writes_rows(tmp_path):
    w = PerfWriter(tmp_path / "perf.jsonl")
    s = LoadSampler(w, hz=20.0)
    s.start()
    time.sleep(0.35)
    s.stop()
    w.close()
    rows = [json.loads(l) for l in (tmp_path / "perf.jsonl").read_text().splitlines()]
    assert len(rows) >= 2 and all(r["kind"] == "load" for r in rows)
    assert {"cpu_percent", "rss_mb", "cpu_per_core"} <= set(rows[0])
    assert "load/cpu_percent" in s.summary()


def test_machine_info_and_summarize():
    info = machine_info()
    assert {"cpu_model", "cpu_logical", "torch", "gpus"} <= set(info)
    rows = [{"env_step_s": 0.5, "policy_s": 0.2, "update_s": 0.2, "total_s": 1.0},
            {"env_step_s": 0.3, "policy_s": 0.2, "update_s": 0.4, "total_s": 1.0}]
    s = summarize(rows, {"load/cpu_percent": 40.0}, env_steps_per_iter=1000)
    assert s["iter_s"] == pytest.approx(1.0) and s["sps"] == pytest.approx(1000.0)
    assert s["gpu_waits_cpu"] == pytest.approx(0.4)
    assert s["cpu_waits_gpu"] == pytest.approx(0.5)
    assert s["load/cpu_percent"] == 40.0
    assert summarize([], {}, 10) == {"phase_frac": {}}
```

Run: `uv run --frozen pytest tests/test_perf.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.perf`）

- [ ] **Step 2: 实现 `perf.py`**

```python
"""开销记录（spec §7.2）：分阶段计时（只在采样迭代上 synchronize）、后台负载采样、机器画像、汇总。"""
from __future__ import annotations

import contextlib
import json
import os
import platform
import threading
import time
from pathlib import Path

import psutil
import torch

PHASES = ("env_step", "h2d", "featurize", "policy", "reward", "update")


def maybe_phase(timer, name: str):
    return timer.phase(name) if timer is not None else contextlib.nullcontext()


class PerfWriter:
    def __init__(self, path: Path):
        self.path = Path(path)
        self._lock = threading.Lock()
        self._f = open(self.path, "a", encoding="utf-8")

    def write(self, row: dict) -> None:
        with self._lock:
            self._f.write(json.dumps(row, ensure_ascii=False) + "\n")
            self._f.flush()

    def close(self) -> None:
        with self._lock:
            self._f.close()


class PhaseTimer:
    def __init__(self, sync_every: int, device: torch.device):
        self.sync_every = max(1, int(sync_every))
        self.cuda = device.type == "cuda"
        self._sync = False
        self._acc: dict[str, float] = {}
        self._t0 = 0.0

    def _synchronize(self) -> None:
        if self.cuda:
            torch.cuda.synchronize()

    def start_iteration(self, iteration: int) -> None:
        self._sync = iteration % self.sync_every == 0
        self._acc = {}
        if self._sync:
            self._synchronize()
        self._t0 = time.perf_counter()

    @contextlib.contextmanager
    def phase(self, name: str):
        if not self._sync:
            yield
            return
        self._synchronize()
        t = time.perf_counter()
        try:
            yield
        finally:
            self._synchronize()
            self._acc[name] = self._acc.get(name, 0.0) + time.perf_counter() - t

    def pop_iteration(self) -> dict[str, float] | None:
        if not self._sync:
            return None
        self._synchronize()
        row = {f"{k}_s": v for k, v in self._acc.items()}
        row["total_s"] = time.perf_counter() - self._t0
        self._sync = False
        return row


def _nvml_handles() -> list:
    try:
        import pynvml

        pynvml.nvmlInit()
        return [pynvml.nvmlDeviceGetHandleByIndex(i) for i in range(pynvml.nvmlDeviceGetCount())]
    except Exception:
        return []


class LoadSampler:
    def __init__(self, writer: PerfWriter, hz: float):
        self.writer = writer
        self.period = 1.0 / max(float(hz), 1e-3)
        self.rows: list[dict] = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True, name="perf-sampler")
        self._proc = psutil.Process()
        self._nvml = _nvml_handles()
        self._t0 = time.time()

    def start(self) -> None:
        psutil.cpu_percent(percpu=True)  # 首次调用只建立基准
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._thread.join(timeout=5)

    def sample(self) -> dict:
        per = psutil.cpu_percent(percpu=True)
        row = {
            "kind": "load", "wall": round(time.time() - self._t0, 3),
            "cpu_percent": sum(per) / len(per), "cpu_max_core": max(per), "cpu_per_core": per,
            "rss_mb": self._proc.memory_info().rss / 2**20, "sys_mem_percent": psutil.virtual_memory().percent,
        }
        if self._nvml:
            import pynvml

            for i, h in enumerate(self._nvml):
                row[f"gpu{i}_util"] = pynvml.nvmlDeviceGetUtilizationRates(h).gpu
                row[f"gpu{i}_mem_mb"] = pynvml.nvmlDeviceGetMemoryInfo(h).used / 2**20
                with contextlib.suppress(Exception):
                    row[f"gpu{i}_power_w"] = pynvml.nvmlDeviceGetPowerUsage(h) / 1000.0
        if torch.cuda.is_available():
            row["torch_max_alloc_mb"] = torch.cuda.max_memory_allocated() / 2**20
        return row

    def _run(self) -> None:
        while not self._stop.wait(self.period):
            row = self.sample()
            self.rows.append(row)
            self.writer.write(row)

    def summary(self) -> dict[str, float]:
        keys = {k for r in self.rows for k, v in r.items()
                if k not in ("kind", "wall") and isinstance(v, (int, float)) and not isinstance(v, bool)}
        return {f"load/{k}": sum(r[k] for r in self.rows if k in r) / max(1, sum(1 for r in self.rows if k in r))
                for k in sorted(keys)}


def machine_info() -> dict:
    cpu_model = platform.processor()
    with contextlib.suppress(OSError):
        for line in open("/proc/cpuinfo", encoding="utf-8"):
            if line.startswith("model name"):
                cpu_model = line.split(":", 1)[1].strip()
                break
    info = {
        "cpu_model": cpu_model, "cpu_logical": os.cpu_count(), "cpu_physical": psutil.cpu_count(logical=False),
        "mem_gb": round(psutil.virtual_memory().total / 2**30, 1), "python": platform.python_version(),
        "torch": torch.__version__, "cuda": torch.version.cuda, "gpus": [], "driver": None,
    }
    if torch.cuda.is_available():
        info["gpus"] = [torch.cuda.get_device_name(i) for i in range(torch.cuda.device_count())]
    with contextlib.suppress(Exception):
        import pynvml

        pynvml.nvmlInit()
        v = pynvml.nvmlSystemGetDriverVersion()
        info["driver"] = v.decode() if isinstance(v, bytes) else v
    return info


def summarize(phase_rows: list[dict], load_summary: dict, env_steps_per_iter: int) -> dict:
    if not phase_rows:
        return {"phase_frac": {}, **load_summary}
    n = len(phase_rows)
    total = sum(r["total_s"] for r in phase_rows) / n
    frac = {p: sum(r.get(f"{p}_s", 0.0) for r in phase_rows) / n / total for p in PHASES}
    return {
        "iter_s": total, "sps": env_steps_per_iter / total, "phase_frac": frac,
        "gpu_waits_cpu": frac["env_step"],
        "cpu_waits_gpu": frac["featurize"] + frac["policy"] + frac["reward"] + frac["update"],
        **load_summary,
    }
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: perf——采样迭代分阶段计时、后台 CPU/内存/NVML 负载采样、机器画像、GPU等CPU/CPU等GPU 汇总

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 10: PPO（LeanRL 底本）+ checkpoint

**Files:**
- Create: `src/stgtrain/ppo.py`, `src/stgtrain/checkpoint.py`
- Test: `tests/test_ppo.py`, `tests/test_checkpoint.py`

**Interfaces:**
- Consumes: `EnvWrapper`（Task 4）、特征化器（Task 5）、`RewardFn` / `EpisodeTracker`（Task 6）、模型（Task 7）、`perf.maybe_phase`（Task 9）、`actions.ACTION_TABLE_VERSION`（Task 3）
- Produces:
  - `Agent(model: nn.Module)`：`get_value(feats) -> Tensor (N,)`、`get_action_and_value(feats, action=None) -> (action, logprob, entropy, value)`
  - `gae(rewards (T,N), values (T,N), dones (T,N) int64, next_value (N,), gamma: float, lam: float) -> tuple[Tensor advantages (T,N), Tensor returns (T,N)]`
  - `PPO(cfg: dict, model_factory: Callable[[], nn.Module], device: torch.device)`
    - `rollout(envw, featurizer, reward_fn, tracker, timer, obs: RawObs) -> tuple[RawObs, TensorDict container (T,N), Tensor next_value (N,)]`；container 键 `feats`（子 TensorDict）、`vals`、`actions`、`logprobs`、`rewards`、`dones`
    - `train_step(container, next_value, iteration: int, num_iterations: int) -> dict[str, float]`：键 `approx_kl, v_loss, pg_loss, entropy_loss, old_approx_kl, clipfrac, gn, explained_variance, lr, done3_frac`
    - `act(feats: Mapping[str, Tensor], greedy: bool) -> Tensor (N,) int64`
    - `state_dict() -> dict`、`load_state_dict(sd: dict) -> None`
    - 属性 `agent`、`agent_inference`、`optimizer`、`compile`、`cudagraphs`（CPU 上两者恒 False）
  - `checkpoint.FORMAT = 1`；`save_checkpoint(path, *, ppo: PPO, update: int, env_steps: int, cfg: dict, extra: dict | None = None) -> None`（原子写）；`load_checkpoint(path, map_location="cpu") -> dict`（键 `format, action_table_version, update, env_steps, cfg, model_name, featurizer_name, state, torch_rng, cuda_rng, extra`；格式或动作表版本不符 `ValueError`）；`restore_rng(ck: dict) -> None`

- [ ] **Step 1: 取 LeanRL 许可文本**

Run: `curl -s https://raw.githubusercontent.com/meta-pytorch/LeanRL/main/LICENSE`
Expected: MIT License，`Copyright (c) 2024 LeanRL developers`。Step 4 把全文原样放进 `ppo.py` 头部注释。

- [ ] **Step 2: 写 PPO 失败测试**

`tests/test_ppo.py`：

```python
import pytest
import stg_rl
import torch

from conftest import FIXTURES, small_cfg
from stgtrain.cards import compile_cards, discover
from stgtrain.envwrap import EnvWrapper
from stgtrain.episodes import EpisodeTracker
from stgtrain.ppo import PPO, gae
from stgtrain.registry import FEATURIZERS, MODELS, load_builtins
from stgtrain.reward import RewardFn

load_builtins()
CPU = torch.device("cpu")


def gae_oracle(r, v, d, nv, gamma, lam):
    """逐 env 标量写法，作为 gae 的对照实现。"""
    T, N = r.shape
    adv = torch.zeros(T, N)
    for i in range(N):
        last = 0.0
        for t in reversed(range(T)):
            code = int(d[t, i])
            v_next = float(nv[i]) if t == T - 1 else float(v[t + 1, i])
            if code == 0:
                delta = r[t, i] + gamma * v_next - v[t, i]
                last = delta + gamma * lam * last
            elif code == 3:
                last = r[t, i] + gamma * v[t, i] - v[t, i]
            else:
                last = r[t, i] - v[t, i]
            adv[t, i] = last
    return adv


def test_gae_done_codes():
    r = torch.tensor([[1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]])
    v = torch.tensor([[0.5, 0.5, 0.5, 0.5], [0.4, 0.4, 0.4, 0.4], [0.3, 0.3, 0.3, 0.3]])
    d = torch.tensor([[0, 0, 0, 0], [0, 1, 3, 0], [0, 0, 0, 2]])
    nv = torch.full((4,), 2.0)
    adv, ret = gae(r, v, d, nv, 0.9, 0.8)
    assert torch.allclose(adv, gae_oracle(r, v, d, nv, 0.9, 0.8), atol=1e-6)
    assert adv[1, 1] == pytest.approx(1.0 - 0.4), "done=1：不自举"
    assert adv[1, 2] == pytest.approx(1.0 + 0.9 * 0.4 - 0.4), "done=3：用 V(obs_t) 自举"
    assert adv[2, 3] == pytest.approx(1.0 - 0.3), "done=2：不自举，也不用 next_value"
    assert torch.allclose(ret, adv + v)


def setup(num_steps=16):
    cfg = small_cfg(ppo={"num_steps": num_steps})
    images = compile_cards(discover(FIXTURES / "cards"))
    envw = EnvWrapper(cfg, images, [stg_rl.Start("example_ring", 0, 2)], CPU, seed=4)
    feat = FEATURIZERS.get("danger_topk_v1")(cfg)
    factory = lambda: MODELS.get("set_attn_v1")(cfg, feat.spec())
    ppo = PPO(cfg, factory, CPU)
    rf = RewardFn(cfg)
    tr = EpisodeTracker(envw.n, CPU, list(rf.terms), 24.0, 16.0, 1, 300)
    return cfg, envw, feat, ppo, rf, tr


def test_rollout_and_train_step_on_cpu():
    torch.manual_seed(0)
    cfg, envw, feat, ppo, rf, tr = setup()
    assert not ppo.compile and not ppo.cudagraphs
    obs = envw.reset()
    before = [p.detach().clone() for p in ppo.agent.parameters()]
    obs, container, next_value = ppo.rollout(envw, feat, rf, tr, None, obs)
    assert container.batch_size == torch.Size([16, 8])
    assert container["feats", "bullets"].shape == (16, 8, 16, 7)
    assert next_value.shape == (8,)
    stats = ppo.train_step(container, next_value, iteration=1, num_iterations=10)
    for k in ("approx_kl", "v_loss", "pg_loss", "entropy_loss", "clipfrac", "gn", "explained_variance", "lr", "done3_frac"):
        assert k in stats
    assert all(v == v for k, v in stats.items() if k != "explained_variance"), "非 NaN"
    assert any(not torch.equal(a, b) for a, b in zip(before, ppo.agent.parameters())), "参数应被更新"
    # agent_inference 与 agent 共享数据
    for a, b in zip(ppo.agent.parameters(), ppo.agent_inference.parameters()):
        assert torch.equal(a.data, b.data)


def test_act_greedy_and_lr_anneal():
    cfg, envw, feat, ppo, rf, tr = setup()
    f = feat(envw.reset())
    a = ppo.act(f, greedy=True)
    assert a.shape == (8,) and a.dtype == torch.int64 and ((0 <= a) & (a < 18)).all()
    assert torch.equal(a, ppo.act(f, greedy=True))
    obs, c, nv = ppo.rollout(envw, feat, rf, tr, None, envw.reset())
    stats = ppo.train_step(c, nv, iteration=6, num_iterations=10)
    assert stats["lr"] == pytest.approx(cfg["ppo"]["learning_rate"] * 0.5)
```

`tests/test_checkpoint.py`：

```python
import pytest
import torch

from test_ppo import setup
from stgtrain.checkpoint import load_checkpoint, restore_rng, save_checkpoint


def test_roundtrip(tmp_path):
    cfg, envw, feat, ppo, rf, tr = setup()
    p = tmp_path / "ck" / "latest.pt"
    save_checkpoint(p, ppo=ppo, update=7, env_steps=896, cfg=cfg, extra={"best": [0.5, 0.2]})
    saved = [x.detach().clone() for x in ppo.agent.parameters()]
    with torch.no_grad():
        for x in ppo.agent.parameters():
            x.add_(1.0)
    ck = load_checkpoint(p)
    assert ck["update"] == 7 and ck["env_steps"] == 896 and ck["extra"]["best"] == [0.5, 0.2]
    assert ck["model_name"] == "set_attn_v1" and ck["cfg"] == cfg
    ppo.load_state_dict(ck["state"])
    for a, b in zip(ppo.agent.parameters(), saved):
        assert torch.equal(a, b)
    for a, b in zip(ppo.agent.parameters(), ppo.agent_inference.parameters()):
        assert torch.equal(a.data, b.data)
    restore_rng(ck)
    assert not (p.parent / "latest.pt.tmp").exists()


def test_rejects_other_action_table(tmp_path):
    cfg, envw, feat, ppo, rf, tr = setup()
    p = tmp_path / "x.pt"
    save_checkpoint(p, ppo=ppo, update=1, env_steps=1, cfg=cfg)
    ck = torch.load(p, weights_only=False)
    ck["action_table_version"] = 2
    torch.save(ck, p)
    with pytest.raises(ValueError, match="动作表"):
        load_checkpoint(p)
```

Run: `uv run --frozen pytest tests/test_ppo.py tests/test_checkpoint.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.ppo`）

- [ ] **Step 3: 实现 `ppo.py`**

头部注释放 Step 1 取到的 MIT 全文（原样），然后：

```python
"""PPO —— 以 LeanRL leanrl/ppo_atari_envpool_torchcompile.py 为底本（MIT，许可见上）。

与底本的差异只有：envpool → EnvWrapper + 特征化器 + RewardFn（本仓胶水）；CNN Agent → 模型注册表；
GAE 按 stg_rl 的 done 码处理（spec §5）；日志 / checkpoint 由 train.py 负责。
rollout / loss / 更新 / compile + CudaGraphModule 的用法照抄底本。
"""
from __future__ import annotations

import os

os.environ.setdefault("TORCHDYNAMO_INLINE_INBUILT_NN_MODULES", "1")

from typing import Callable, Mapping  # noqa: E402

import torch  # noqa: E402
import torch.nn as nn  # noqa: E402
import torch.optim as optim  # noqa: E402
from tensordict import TensorDict, from_module  # noqa: E402
from tensordict.nn import CudaGraphModule, TensorDictModule  # noqa: E402
from torch import Tensor  # noqa: E402
from torch.distributions.categorical import Categorical, Distribution  # noqa: E402

from .perf import maybe_phase  # noqa: E402

Distribution.set_default_validate_args(False)

# 底本的临时修补（pytorch#138080）：torch 2.14 的 Categorical.logits/probs 仍是 lazy_property，照抄；
# 将来 torch 去掉 lazy_property（没有 .wrapped）时自动跳过。
for _name in ("logits", "probs"):
    _attr = Categorical.__dict__.get(_name)
    if hasattr(_attr, "wrapped"):
        setattr(Categorical, _name, property(_attr.wrapped))

torch.set_float32_matmul_precision("high")


class Agent(nn.Module):
    def __init__(self, model: nn.Module):
        super().__init__()
        self.model = model

    def get_value(self, feats):
        return self.model(feats)[1]

    def get_action_and_value(self, feats, action=None):
        logits, value = self.model(feats)
        probs = Categorical(logits=logits)
        if action is None:
            action = probs.sample()
        return action, probs.log_prob(action), probs.entropy(), value


def gae(rewards: Tensor, values: Tensor, dones: Tensor, next_value: Tensor, gamma: float, lam: float):
    """dones[t] 是第 t 步动作产生的 done 码；done ≠ 0 时 obs[t+1] 已是新局首帧，故任何 done 都切断优势回传。
    done ∈ {1, 2}：终止，不自举。done == 3：超时截断，缺末帧观测，用 V(obs_t) 近似自举（spec §5）。"""
    nonterminal = (dones == 0).to(rewards.dtype)
    truncated = (dones == 3).to(rewards.dtype)
    lastgaelam = torch.zeros_like(next_value)
    next_v = next_value
    advantages = []
    for t in range(rewards.shape[0] - 1, -1, -1):
        boot = nonterminal[t] * next_v + truncated[t] * values[t]
        delta = rewards[t] + gamma * boot - values[t]
        lastgaelam = delta + gamma * lam * nonterminal[t] * lastgaelam
        advantages.append(lastgaelam)
        next_v = values[t]
    adv = torch.stack(list(reversed(advantages)))
    return adv, adv + values


class PPO:
    def __init__(self, cfg: dict, model_factory: Callable[[], nn.Module], device: torch.device):
        self.p = cfg["ppo"]
        self.device = device
        self.num_steps = int(self.p["num_steps"])
        self.agent = Agent(model_factory()).to(device)
        # 底本：推理用一份共享参数数据、但不带梯度的副本
        self.agent_inference = Agent(model_factory()).to(device)
        from_module(self.agent).data.to_module(self.agent_inference)

        on_cuda = device.type == "cuda"
        self.compile = bool(self.p["compile"]) and on_cuda
        self.cudagraphs = bool(self.p["cudagraphs"]) and on_cuda
        self.optimizer = optim.Adam(
            self.agent.parameters(), lr=torch.tensor(float(self.p["learning_rate"]), device=device), eps=1e-5,
            capturable=self.cudagraphs and not self.compile,
        )

        policy = self.agent_inference.get_action_and_value
        update = TensorDictModule(
            self._update,
            in_keys=["feats", "actions", "logprobs", "advantages", "returns", "vals"],
            out_keys=["approx_kl", "v_loss", "pg_loss", "entropy_loss", "old_approx_kl", "clipfrac", "gn"],
        )
        if self.compile:
            mode = "reduce-overhead" if not self.cudagraphs else None
            policy = torch.compile(policy, mode=mode)
            update = torch.compile(update, mode=mode)
        if self.cudagraphs:
            policy = CudaGraphModule(policy, warmup=20)
            update = CudaGraphModule(update, warmup=20)
        self.policy, self.update = policy, update

    def _update(self, feats, actions, logprobs, advantages, returns, vals):
        p = self.p
        self.optimizer.zero_grad()
        _, newlogprob, entropy, newvalue = self.agent.get_action_and_value(feats, actions)
        logratio = newlogprob - logprobs
        ratio = logratio.exp()

        with torch.no_grad():
            old_approx_kl = (-logratio).mean()
            approx_kl = ((ratio - 1) - logratio).mean()
            clipfrac = ((ratio - 1.0).abs() > p["clip_coef"]).float().mean()

        if p["norm_adv"]:
            advantages = (advantages - advantages.mean()) / (advantages.std() + 1e-8)

        pg_loss1 = -advantages * ratio
        pg_loss2 = -advantages * torch.clamp(ratio, 1 - p["clip_coef"], 1 + p["clip_coef"])
        pg_loss = torch.max(pg_loss1, pg_loss2).mean()

        newvalue = newvalue.view(-1)
        if p["clip_vloss"]:
            v_loss_unclipped = (newvalue - returns) ** 2
            v_clipped = vals + torch.clamp(newvalue - vals, -p["clip_coef"], p["clip_coef"])
            v_loss_clipped = (v_clipped - returns) ** 2
            v_loss = 0.5 * torch.max(v_loss_unclipped, v_loss_clipped).mean()
        else:
            v_loss = 0.5 * ((newvalue - returns) ** 2).mean()

        entropy_loss = entropy.mean()
        loss = pg_loss - p["ent_coef"] * entropy_loss + v_loss * p["vf_coef"]
        loss.backward()
        gn = nn.utils.clip_grad_norm_(self.agent.parameters(), p["max_grad_norm"])
        self.optimizer.step()
        return approx_kl, v_loss.detach(), pg_loss.detach(), entropy_loss.detach(), old_approx_kl, clipfrac, gn

    def rollout(self, envw, featurizer, reward_fn, tracker, timer, obs):
        n = envw.n
        ts = []
        for _ in range(self.num_steps):
            with maybe_phase(timer, "featurize"):
                feats = TensorDict(featurizer(obs), batch_size=[n])
            with maybe_phase(timer, "policy"):
                torch.compiler.cudagraph_mark_step_begin()
                action, logprob, _, value = self.policy(feats)
            next_obs, info = envw.step(action, timer)
            with maybe_phase(timer, "reward"):
                reward, raw = reward_fn(obs, next_obs, info)
                tracker.update(obs, next_obs, info, reward, raw)
            ts.append(TensorDict._new_unsafe(
                feats=feats, vals=value.flatten(), actions=action, logprobs=logprob, rewards=reward,
                dones=info.done, batch_size=(n,),
            ))
            obs = next_obs
        container = torch.stack(ts, 0)
        with maybe_phase(timer, "featurize"):
            next_feats = TensorDict(featurizer(obs), batch_size=[n])
        with torch.no_grad():
            next_value = self.agent_inference.get_value(next_feats)
        return obs, container, next_value

    def train_step(self, container, next_value, iteration: int, num_iterations: int) -> dict[str, float]:
        p = self.p
        if p["anneal_lr"]:
            frac = 1.0 - (iteration - 1.0) / num_iterations
            self.optimizer.param_groups[0]["lr"].copy_(frac * float(p["learning_rate"]))
        adv, ret = gae(container["rewards"], container["vals"], container["dones"], next_value,
                       float(p["gamma"]), float(p["gae_lambda"]))
        container["advantages"] = adv
        container["returns"] = ret
        flat = container.view(-1)
        mb = flat.shape[0] // int(p["num_minibatches"])
        outs = []
        for _ in range(int(p["update_epochs"])):
            for b in torch.randperm(flat.shape[0], device=self.device).split(mb):
                torch.compiler.cudagraph_mark_step_begin()
                outs.append(self.update(flat[b], tensordict_out=TensorDict()))
        stats = {k: torch.stack([o[k] for o in outs]).float().mean().item() for k in outs[0].keys()}
        var_y = ret.var()
        stats["explained_variance"] = (
            (1.0 - (ret - container["vals"]).var() / var_y).item() if var_y.item() > 0 else float("nan")
        )
        stats["lr"] = float(self.optimizer.param_groups[0]["lr"])
        stats["done3_frac"] = (container["dones"] == 3).float().mean().item()
        return stats

    @torch.no_grad()
    def act(self, feats: Mapping[str, Tensor], greedy: bool) -> Tensor:
        logits, _ = self.agent_inference.model(feats)
        return logits.argmax(-1) if greedy else Categorical(logits=logits).sample()

    def state_dict(self) -> dict:
        return {"agent": self.agent.state_dict(), "optimizer": self.optimizer.state_dict()}

    def load_state_dict(self, sd: dict) -> None:
        self.agent.load_state_dict(sd["agent"])
        self.optimizer.load_state_dict(sd["optimizer"])
        from_module(self.agent).data.to_module(self.agent_inference)
```

Run: `uv run --frozen pytest tests/test_ppo.py -q`
Expected: PASS。若 `TensorDictModule` 对子 TensorDict 键 `feats` 取值方式与预期不同（报 `KeyError` 或拿到的不是 TensorDict），**先读 tensordict 0.14.2 `TensorDictModule.forward` 源码确认，再改调用方式**，在 `_update` 上方注释写明原因；不要改成绕过 `TensorDictModule`（CUDA 图路径依赖它）。

- [ ] **Step 4: 实现 `checkpoint.py`**

```python
"""checkpoint 存 / 读（spec §5）：自描述（注册名 + 完整配置 + 动作表版本），原子写。"""
from __future__ import annotations

import os
from pathlib import Path

import torch

from .actions import ACTION_TABLE_VERSION

FORMAT = 1


def save_checkpoint(path, *, ppo, update: int, env_steps: int, cfg: dict, extra: dict | None = None) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "format": FORMAT, "action_table_version": ACTION_TABLE_VERSION,
        "update": int(update), "env_steps": int(env_steps), "cfg": cfg,
        "model_name": cfg["model"]["name"], "featurizer_name": cfg["featurize"]["name"],
        "state": ppo.state_dict(),
        "torch_rng": torch.get_rng_state(),
        "cuda_rng": torch.cuda.get_rng_state_all() if torch.cuda.is_available() else None,
        "extra": extra or {},
    }
    tmp = path.with_name(path.name + ".tmp")
    torch.save(payload, tmp)
    os.replace(tmp, path)


def load_checkpoint(path, map_location="cpu") -> dict:
    ck = torch.load(path, map_location=map_location, weights_only=False)
    if ck.get("format") != FORMAT:
        raise ValueError(f"checkpoint 格式 {ck.get('format')} ≠ {FORMAT}")
    if ck.get("action_table_version") != ACTION_TABLE_VERSION:
        raise ValueError(f"checkpoint 的动作表版本 {ck.get('action_table_version')} ≠ 当前 {ACTION_TABLE_VERSION}")
    return ck


def restore_rng(ck: dict) -> None:
    # 续训时 checkpoint 按 map_location=device 读入，RNG 状态须搬回 CPU ByteTensor 才能 set
    torch.set_rng_state(ck["torch_rng"].cpu())
    if ck.get("cuda_rng") is not None and torch.cuda.is_available():
        torch.cuda.set_rng_state_all([s.cpu() for s in ck["cuda_rng"]])
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: PPO（LeanRL 底本：rollout/loss/compile+CudaGraphModule 照抄，GAE 按 done 码）+ 自描述原子写 checkpoint

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 11: 固定评测集 `evaluate.py`

**Files:**
- Create: `src/stgtrain/evaluate.py`
- Test: `tests/test_evaluate.py`

**Interfaces:**
- Consumes: `cards.EvalSpec / load_splits / compile_cards / discover`（Task 2）、`EnvWrapper`（Task 4）、`RewardFn` / `EpisodeTracker`（Task 6）、`PPO.act`（Task 10）
- Produces:
  - `summarize_eval(records: list[dict]) -> dict[str, float]`：键 `episodes, survival, death, timeout, frames_mean, return_mean, in_r_frac, reach_frames_median, shift_toggles_per_s, dir_changes_per_s, edge_frac`（空列表 → `{"episodes": 0.0}`）
  - `score(overall: dict) -> tuple[float, float]` = `(survival, in_r_frac)`，用于挑 `best.pt`（元组比较，越大越好）
  - `run_group(cfg, ppo, featurizer, image, card: str, rank: int, episodes: int, device) -> list[dict]`：恰好 `episodes` 条局记录（每个 env 只取第一局，Ruling 5）
  - `evaluate(cfg, ppo, featurizer, images: dict, specs: list[EvalSpec], device) -> dict`：`{"cards": {card: {"r<rank>": summary}}, "overall": summary}`

- [ ] **Step 1: 写失败测试**

`tests/test_evaluate.py`：

```python
import pytest
import torch

from conftest import FIXTURES, small_cfg
from stgtrain.cards import compile_cards, discover, load_splits
from stgtrain.evaluate import evaluate, score, summarize_eval
from stgtrain.ppo import PPO
from stgtrain.registry import FEATURIZERS, MODELS, load_builtins

load_builtins()


def rec(done, frames=100, reach=50.0, in_r=0.5):
    return {"env": 0, "done": done, "frames": frames, "return": 1.0, "steps": frames, "in_r_frac": in_r,
            "edge_frac": 0.0, "shift_toggles_per_s": 1.0, "dir_changes_per_s": 2.0, "reach_frames": reach}


def test_summarize_eval_and_score():
    s = summarize_eval([rec(2, reach=10.0), rec(1, reach=30.0, in_r=0.1), rec(2, reach=300.0), rec(3)])
    assert s["episodes"] == 4 and s["survival"] == 0.5 and s["death"] == 0.25 and s["timeout"] == 0.25
    assert s["reach_frames_median"] == pytest.approx(40.0)
    assert s["in_r_frac"] == pytest.approx((0.5 + 0.1 + 0.5 + 0.5) / 4)
    assert summarize_eval([]) == {"episodes": 0.0}
    assert score({"survival": 0.5, "in_r_frac": 0.9}) > score({"survival": 0.4, "in_r_frac": 1.0})


def test_evaluate_calm_card_counts_exact_episodes():
    cfg = small_cfg()
    device = torch.device("cpu")
    images = compile_cards(discover(FIXTURES / "cards"))
    specs = load_splits(FIXTURES / "eval_splits.toml", cfg["eval"]["episodes"])
    feat = FEATURIZERS.get("danger_topk_v1")(cfg)
    ppo = PPO(cfg, lambda: MODELS.get("set_attn_v1")(cfg, feat.spec()), device)
    # 固定「一直按下」：测的是评测管线，不是策略（未训练的网络可能撞上 boss 本体）
    ppo.act = lambda feats, greedy: torch.full((feats["player"].shape[0],), 10, dtype=torch.int64)
    res = evaluate(cfg, ppo, feat, images, specs, device)
    r2 = res["cards"]["example_calm"]["r2"]
    assert r2["episodes"] == 4 and r2["survival"] == 1.0
    assert r2["frames_mean"] > 250
    assert res["overall"]["episodes"] == 4
```

Run: `uv run --frozen pytest tests/test_evaluate.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.evaluate`）

- [ ] **Step 2: 实现 `evaluate.py`**

```python
"""固定评测集（spec §6 + 计划 Ruling 5）：每个 (卡, rank) 开一个 num_envs = episodes 的 VecEnv，
env 种子与意图种子固定、不镜像，每个 env 只取第一局 ⇒ 不同 checkpoint 之间可直接比较。"""
from __future__ import annotations

import statistics

import stg_rl

from .cards import EvalSpec
from .envwrap import EnvWrapper
from .episodes import EpisodeTracker
from .reward import RewardFn


def summarize_eval(records: list[dict]) -> dict[str, float]:
    n = len(records)
    if n == 0:
        return {"episodes": 0.0}

    def mean(k: str) -> float:
        return sum(float(r[k]) for r in records) / n

    def frac(code: int) -> float:
        return sum(1 for r in records if r["done"] == code) / n

    return {
        "episodes": float(n), "survival": frac(2), "death": frac(1), "timeout": frac(3),
        "frames_mean": mean("frames"), "return_mean": mean("return"), "in_r_frac": mean("in_r_frac"),
        "reach_frames_median": float(statistics.median(float(r["reach_frames"]) for r in records)),
        "shift_toggles_per_s": mean("shift_toggles_per_s"), "dir_changes_per_s": mean("dir_changes_per_s"),
        "edge_frac": mean("edge_frac"),
    }


def score(overall: dict) -> tuple[float, float]:
    return float(overall.get("survival", 0.0)), float(overall.get("in_r_frac", 0.0))


def run_group(cfg: dict, ppo, featurizer, image, card: str, rank: int, episodes: int, device) -> list[dict]:
    envw = EnvWrapper(cfg, {card: image}, [stg_rl.Start(card, 0, rank)], device,
                      seed=int(cfg["eval"]["seed"]), num_envs=episodes, mirror=False)
    reward_fn = RewardFn(cfg)
    tracker = EpisodeTracker(episodes, device, list(reward_fn.terms), cfg["reward"]["hold_radius"],
                             cfg["reward"]["edge_margin"], envw.frame_skip, cfg["intent"]["interval"][1])
    greedy = bool(cfg["eval"]["greedy"])
    first: dict[int, dict] = {}
    obs = envw.reset()
    # 每个 env 的第一局最迟在 max_frames 帧内结束（预热发生在 step 内部，不占步数）
    max_steps = int(cfg["env"]["max_frames"]) // envw.frame_skip + 2
    for step in range(max_steps):
        action = ppo.act(featurizer(obs), greedy)
        nxt, info = envw.step(action)
        total, raw = reward_fn(obs, nxt, info)
        tracker.update(obs, nxt, info, total, raw)
        obs = nxt
        if (step + 1) % 64 == 0:
            for r in tracker.pop_finished():
                first.setdefault(r["env"], r)
            if len(first) == episodes:
                break
    for r in tracker.pop_finished():
        first.setdefault(r["env"], r)
    return [first[i] for i in sorted(first)]


def evaluate(cfg: dict, ppo, featurizer, images: dict, specs: list[EvalSpec], device) -> dict:
    result: dict = {"cards": {}, "overall": {}}
    everything: list[dict] = []
    for spec in specs:
        per: dict[str, dict] = {}
        for rank in spec.ranks:
            recs = run_group(cfg, ppo, featurizer, images[spec.card], spec.card, rank, spec.episodes, device)
            per[f"r{rank}"] = summarize_eval(recs)
            everything.extend(recs)
        result["cards"][spec.card] = per
    result["overall"] = summarize_eval(everything)
    return result
```

Run: `uv run --frozen pytest tests/ -q`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: 固定评测集——每 (卡, rank) 独立 VecEnv、每 env 取第一局、活命/遵从/拟人汇总与 best 评分

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 12: 编排 —— `train.py` + `bench.py` + `gpucheck.py` + `run.sh` + 配置 + 冒烟

**Files:**
- Create: `src/stgtrain/train.py`, `src/stgtrain/bench.py`, `src/stgtrain/gpucheck.py`, `run.sh`
- Create: `configs/base.toml`, `configs/smoke.toml`, `cards/README.md`, `eval/splits.toml`
- Test: `tests/test_smoke.py`

**Interfaces:**
- Consumes: Task 1–11 的全部公开接口
- Produces:
  - `train.pick_device(name: str) -> torch.device`、`train.git_sha() -> str`、`train.make_run_dir(runs_dir: Path, name: str) -> Path`、`train.pack(run_dir: Path) -> Path`
  - `train.build_components(cfg, device) -> tuple[images: dict, starts: list, specs: list[EvalSpec], featurizer, model_factory]`
  - `train.train(cfg: dict, run_dir: Path, resume: dict | None = None, pack_result: bool = True) -> Path`
  - `train.main(argv: list[str] | None = None) -> int`：`<config> [name]`、`--resume RUN_DIR`、`--bench`、`--runs-dir DIR`（默认 `runs`）、`--total-updates N`、`--no-pack`
  - `bench.run_bench(cfg: dict, out_dir: Path) -> dict`（写 `out_dir/bench.json`：`machine, stg_rl, results[{num_envs, threads, steps, seconds, env_steps_per_s}], recommended{num_envs, threads}`）
  - `gpucheck.main(argv) -> int`（无 CUDA 返回 2；通过 0；超差 1）

- [ ] **Step 1: 配置文件与目录**

`configs/base.toml`（与 `config.DEFAULTS` 逐项相同，测试押运）：

```toml
[run]
seed = 1
device = "auto"
total_updates = 2000
ckpt_every = 50
eval_every = 50
torch_threads = 2

[env]
cards_dir = "cards"
eval_splits = "eval/splits.toml"
num_envs = 2048
threads = 0            # 0 = 全部逻辑核；先跑 --bench 选值
frame_skip = 1
max_frames = 3600
warmup_max = 120
bullets_cap = 1024
ranks = [2]
mirror = true

[intent]
name = "lower_half_uniform_v1"
margin = 16.0
interval = [120, 300]

[featurize]
name = "danger_topk_v1"
k_bullets = 64
k_enemies = 8
horizon = 60
d_max = 128.0

[model]
name = "set_attn_v1"
d = 64
heads = 4
trunk = 256

[reward]
hold_radius = 24.0
edge_margin = 16.0

[reward.terms]
death = 10.0
follow_shaping = 1.0
hold = 0.01
segment_survived = 0.0
key_press = 0.0
shift_toggle = 0.0
edge_hug = 0.0

[ppo]
num_steps = 64
gamma = 0.995
gae_lambda = 0.95
num_minibatches = 8
update_epochs = 4
clip_coef = 0.2
clip_vloss = true
ent_coef = 0.01
vf_coef = 0.5
max_grad_norm = 0.5
learning_rate = 0.0003
anneal_lr = true
norm_adv = true
compile = true
cudagraphs = true

[eval]
episodes = 32
greedy = true
seed = 12345

[log]
tensorboard = true
perf_sync_every = 20
sample_hz = 1.0

[bench]
seconds = 10.0
num_envs = [512, 1024, 2048, 4096]
```

`configs/smoke.toml`（本地 CPU 冒烟，跑夹具卡）：

```toml
[run]
device = "cpu"
total_updates = 3
ckpt_every = 2
eval_every = 3
torch_threads = 4

[env]
cards_dir = "tests/fixtures/cards"
eval_splits = "tests/fixtures/eval_splits.toml"
num_envs = 8
threads = 4
bullets_cap = 256

[featurize]
k_bullets = 16

[model]
d = 16
heads = 2
trunk = 32

[ppo]
num_steps = 16
num_minibatches = 2
update_epochs = 1
compile = false
cudagraphs = false

[eval]
episodes = 4

[bench]
seconds = 1.0
num_envs = [8, 16]
```

`cards/README.md`：

```markdown
# RL 卡池

一张卡 = 一个子目录（`main.ecl` + 可选 `meta.toml`）。写作约束见 stg-engine 仓 `docs/rl-card-pool.md`。
评测留出卡在 `eval/splits.toml` 里列出；其余卡全部用于训练。
```

`eval/splits.toml`：

```toml
# 评测卡划分（spec §6）。卡池到位后在这里列出留出卡；训练卡 = cards/ 下其余的卡。
# [[eval]]
# card = "th06_s4_boss_card1"
# ranks = [2]
# episodes = 32
```

- [ ] **Step 2: 写失败测试**

`tests/test_smoke.py`：

```python
import json
from pathlib import Path

import pytest
import torch

from conftest import small_cfg
from stgtrain import gpucheck
from stgtrain.checkpoint import load_checkpoint
from stgtrain.config import dump_toml, from_dict, load_config
from stgtrain.metrics import read_jsonl
from stgtrain.train import main, make_run_dir, train

REPO = Path(__file__).resolve().parents[1]


def test_repo_configs_load():
    assert load_config(REPO / "configs" / "base.toml") == from_dict({}), "base.toml 须与 DEFAULTS 一致"
    smoke = load_config(REPO / "configs" / "smoke.toml")
    assert smoke["run"]["device"] == "cpu" and not smoke["ppo"]["compile"] and not smoke["ppo"]["cudagraphs"]


def test_train_end_to_end_then_resume(tmp_path):
    cfg = small_cfg(run={"total_updates": 2, "ckpt_every": 2, "eval_every": 2}, log={"tensorboard": True})
    run_dir = make_run_dir(tmp_path, "smoke")
    train(cfg, run_dir, pack_result=True)
    for rel in ("config.toml", "env.json", "metrics.jsonl", "perf.jsonl", "checkpoints/latest.pt",
                "checkpoints/u2.pt", "checkpoints/best.pt", "eval/2.json", "plots/ppo.png", "tb"):
        assert (run_dir / rel).exists(), rel
    assert (tmp_path / f"{run_dir.name}.tar.gz").exists()
    rows = read_jsonl(run_dir / "metrics.jsonl")
    assert [r["update"] for r in rows if "ppo/pg_loss" in r] == [1, 2]
    assert any("eval/survival" in r for r in rows)
    env = json.loads((run_dir / "env.json").read_text(encoding="utf-8"))
    assert env["action_table_version"] == 1 and "perf_summary" in env and env["stg_rl"]["engine_ver"]

    assert main(["--resume", str(run_dir), "--total-updates", "3", "--no-pack"]) == 0
    ck = load_checkpoint(run_dir / "checkpoints" / "latest.pt")
    assert ck["update"] == 3 and ck["cfg"]["run"]["total_updates"] == 3
    assert [r["update"] for r in read_jsonl(run_dir / "metrics.jsonl") if "ppo/pg_loss" in r] == [1, 2, 3]


def test_bench_cli(tmp_path):
    cfg_path = tmp_path / "bench.toml"
    dump_toml(small_cfg(bench={"seconds": 0.3, "num_envs": [8]}), cfg_path)
    assert main([str(cfg_path), "b", "--bench", "--runs-dir", str(tmp_path / "runs")]) == 0
    (bench_json,) = list((tmp_path / "runs").glob("*-bench-b/bench.json"))
    out = json.loads(bench_json.read_text(encoding="utf-8"))
    assert out["results"] and set(out["recommended"]) == {"num_envs", "threads"}


def test_gpucheck_requires_cuda(monkeypatch, tmp_path):
    monkeypatch.setattr(torch.cuda, "is_available", lambda: False)
    assert gpucheck.main([str(REPO / "configs" / "smoke.toml")]) == 2
```

Run: `uv run --frozen pytest tests/test_smoke.py -q`
Expected: FAIL（`ModuleNotFoundError: stgtrain.gpucheck`）

- [ ] **Step 3: 实现 `train.py`**

```python
"""入口（spec §2.2）。

python -m stgtrain.train <config> [name]            训练
python -m stgtrain.train --resume <run 目录> [--total-updates N]
python -m stgtrain.train <config> [name] --bench     测吞吐
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import tarfile
import time
from pathlib import Path

import stg_rl
import torch

from . import plots
from .actions import ACTION_TABLE_VERSION
from .cards import compile_cards, discover, load_splits, train_starts
from .checkpoint import load_checkpoint, restore_rng, save_checkpoint
from .config import deep_merge, dump_toml, from_dict, load_config
from .envwrap import EnvWrapper
from .episodes import EpisodeTracker
from .evaluate import evaluate, score
from .metrics import MetricsLogger, read_jsonl, summarize_episodes
from .perf import LoadSampler, PerfWriter, PhaseTimer, machine_info, summarize
from .ppo import PPO
from .registry import FEATURIZERS, MODELS, check_compat, load_builtins
from .reward import RewardFn

REPO_ROOT = Path(__file__).resolve().parents[2]


def pick_device(name: str) -> torch.device:
    if name == "cpu":
        return torch.device("cpu")
    if name == "cuda":
        if not torch.cuda.is_available():
            raise RuntimeError("run.device = cuda，但 torch.cuda.is_available() 为 False")
        return torch.device("cuda")
    return torch.device("cuda" if torch.cuda.is_available() else "cpu")


def git_sha() -> str:
    try:
        run = lambda *a: subprocess.run(["git", *a], cwd=REPO_ROOT, capture_output=True, text=True, check=True).stdout.strip()
        sha = run("rev-parse", "--short", "HEAD")
        return sha + ("-dirty" if run("status", "--porcelain", "--untracked-files=no") else "")
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def make_run_dir(runs_dir: Path, name: str) -> Path:
    d = Path(runs_dir) / f"{time.strftime('%Y%m%d-%H%M%S')}-{name}"
    for sub in ("checkpoints", "eval", "plots"):
        (d / sub).mkdir(parents=True, exist_ok=True)
    return d


def _update_env_json(run_dir: Path, **fields) -> None:
    p = run_dir / "env.json"
    info = json.loads(p.read_text(encoding="utf-8")) if p.exists() else {}
    info.update(fields)
    p.write_text(json.dumps(info, indent=2, ensure_ascii=False), encoding="utf-8")


def pack(run_dir: Path) -> Path:
    out = run_dir.parent / f"{run_dir.name}.tar.gz"
    with tarfile.open(out, "w:gz") as tf:
        tf.add(run_dir, arcname=run_dir.name)
    return out


def build_components(cfg: dict, device: torch.device):
    load_builtins()
    cards = discover(cfg["env"]["cards_dir"])
    specs = load_splits(cfg["env"]["eval_splits"], cfg["eval"]["episodes"])
    missing = [s.card for s in specs if s.card not in cards]
    if missing:
        raise ValueError(f"评测划分里的卡在 {cfg['env']['cards_dir']} 下不存在：{missing}")
    images = compile_cards(cards)
    starts = train_starts(cards, {s.card for s in specs}, cfg["env"]["ranks"])
    featurizer = FEATURIZERS.get(cfg["featurize"]["name"])(cfg)
    spec = featurizer.spec()
    model_cls = MODELS.get(cfg["model"]["name"])

    def factory():
        return model_cls(cfg, spec)

    check_compat(factory().requires(), spec)
    return images, starts, specs, featurizer, factory


def train(cfg: dict, run_dir: Path, resume: dict | None = None, pack_result: bool = True) -> Path:
    run_dir = Path(run_dir)
    device = pick_device(cfg["run"]["device"])
    torch.manual_seed(int(cfg["run"]["seed"]))
    torch.set_num_threads(int(cfg["run"]["torch_threads"]))
    dump_toml(cfg, run_dir / "config.toml")
    images, starts, specs, featurizer, factory = build_components(cfg, device)
    ppo = PPO(cfg, factory, device)

    start, env_steps, best = 1, 0, None
    if resume is not None:
        ppo.load_state_dict(resume["state"])
        restore_rng(resume)
        start, env_steps = int(resume["update"]) + 1, int(resume["env_steps"])
        best = tuple(resume["extra"]["best"]) if resume["extra"].get("best") is not None else None
    if not (run_dir / "env.json").exists():
        _update_env_json(run_dir, stg_rl=stg_rl.build_info(), train_repo_sha=git_sha(),
                         action_table_version=ACTION_TABLE_VERSION, machine=machine_info())

    total = int(cfg["run"]["total_updates"])
    envw = EnvWrapper(cfg, images, starts, device, seed=int(cfg["run"]["seed"]) + start - 1)  # Ruling 7
    reward_fn = RewardFn(cfg)
    tracker = EpisodeTracker(envw.n, device, list(reward_fn.terms), cfg["reward"]["hold_radius"],
                             cfg["reward"]["edge_margin"], envw.frame_skip, cfg["intent"]["interval"][1])
    logger = MetricsLogger(run_dir, bool(cfg["log"]["tensorboard"]))
    perf_writer = PerfWriter(run_dir / "perf.jsonl")
    sampler = LoadSampler(perf_writer, float(cfg["log"]["sample_hz"]))
    timer = PhaseTimer(int(cfg["log"]["perf_sync_every"]), device)
    steps_per_iter = int(cfg["ppo"]["num_steps"]) * envw.n * envw.frame_skip
    ckpt_dir = run_dir / "checkpoints"
    phase_rows: list[dict] = []

    sampler.start()
    try:
        obs = envw.reset()
        for update in range(start, total + 1):
            timer.start_iteration(update)
            t0 = time.perf_counter()
            obs, container, next_value = ppo.rollout(envw, featurizer, reward_fn, tracker, timer, obs)
            with timer.phase("update"):
                stats = ppo.train_step(container, next_value, update, total)
            env_steps += steps_per_iter
            scalars = {f"ppo/{k}": v for k, v in stats.items()}
            scalars.update(summarize_episodes(tracker.pop_finished(), "ep/"))
            scalars["perf/sps"] = steps_per_iter / (time.perf_counter() - t0)
            row = timer.pop_iteration()
            if row is not None:
                phase_rows.append(row)
                perf_writer.write({"kind": "phase", "update": update, **row})
                scalars.update({f"perf/{k}": v for k, v in row.items()})
            logger.log(update, env_steps, scalars)

            if specs and (update % int(cfg["run"]["eval_every"]) == 0 or update == total):
                res = evaluate(cfg, ppo, featurizer, images, specs, device)
                (run_dir / "eval" / f"{update}.json").write_text(json.dumps(res, indent=2, ensure_ascii=False),
                                                               encoding="utf-8")
                logger.log(update, env_steps, {f"eval/{k}": v for k, v in res["overall"].items()})
                sc = score(res["overall"])
                if best is None or sc > best:
                    best = sc
                    save_checkpoint(ckpt_dir / "best.pt", ppo=ppo, update=update, env_steps=env_steps, cfg=cfg,
                                    extra={"best": list(best), "eval": res["overall"]})

            if update % int(cfg["run"]["ckpt_every"]) == 0 or update == total:
                save_checkpoint(ckpt_dir / "latest.pt", ppo=ppo, update=update, env_steps=env_steps, cfg=cfg,
                                extra={"best": list(best) if best is not None else None})
                if update % int(cfg["run"]["ckpt_every"]) == 0:
                    shutil.copyfile(ckpt_dir / "latest.pt", ckpt_dir / f"u{update}.pt")
    finally:
        sampler.stop()
        perf_writer.close()
        logger.close()

    _update_env_json(run_dir, perf_summary=summarize(phase_rows, sampler.summary(), steps_per_iter))
    plots.plot_runs({run_dir.name: read_jsonl(run_dir / "metrics.jsonl")}, run_dir / "plots")
    plots.plot_load(plots.load_perf(run_dir / "perf.jsonl"), run_dir / "plots")
    if pack_result:
        print(pack(run_dir))
    return run_dir


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m stgtrain.train")
    ap.add_argument("config", nargs="?", help="配置 TOML（--resume 时省略）")
    ap.add_argument("name", nargs="?", default="run", help="运行名，拼进结果目录名")
    ap.add_argument("--resume", metavar="RUN_DIR", help="从 RUN_DIR/checkpoints/latest.pt 续训")
    ap.add_argument("--bench", action="store_true", help="只测吞吐（spec §7.3）")
    ap.add_argument("--runs-dir", default="runs")
    ap.add_argument("--total-updates", type=int, default=None, help="覆盖 run.total_updates（续训加长用）")
    ap.add_argument("--no-pack", action="store_true", help="不打 tar.gz")
    args = ap.parse_args(argv)
    overrides = {"run": {"total_updates": args.total_updates}} if args.total_updates else {}

    if args.resume:
        run_dir = Path(args.resume)
        ck = load_checkpoint(run_dir / "checkpoints" / "latest.pt")
        cfg = from_dict(deep_merge(ck["cfg"], overrides))
        ck = load_checkpoint(run_dir / "checkpoints" / "latest.pt", map_location=pick_device(cfg["run"]["device"]))
        train(cfg, run_dir, resume=ck, pack_result=not args.no_pack)
        return 0
    if not args.config:
        ap.error("需要配置文件，或 --resume RUN_DIR")
    cfg = load_config(args.config, overrides or None)
    if args.bench:
        from .bench import run_bench

        run_bench(cfg, make_run_dir(Path(args.runs_dir), f"bench-{args.name}"))
        return 0
    train(cfg, make_run_dir(Path(args.runs_dir), args.name), pack_result=not args.no_pack)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 4: 实现 `bench.py`**

```python
"""--bench（spec §7.3）：扫 threads × num_envs，测端到端稳态吞吐（env + 胶水 + 未训练模型推理，不做更新）。"""
from __future__ import annotations

import json
import os
import time
from pathlib import Path

import stg_rl
import torch

from .config import deep_merge
from .envwrap import EnvWrapper
from .perf import machine_info
from .train import build_components, pick_device


def _step(envw, featurizer, model, obs):
    logits, _ = model(featurizer(obs))
    action = torch.distributions.Categorical(logits=logits).sample()
    nxt, _ = envw.step(action)
    return nxt


def _sync(device: torch.device) -> None:
    if device.type == "cuda":
        torch.cuda.synchronize()


def run_bench(cfg: dict, out_dir: Path) -> dict:
    device = pick_device(cfg["run"]["device"])
    images, starts, _, featurizer, factory = build_components(cfg, device)
    model = factory().to(device).eval()
    cpu = os.cpu_count() or 1
    grid = sorted({max(1, cpu // 4), max(1, cpu // 2), cpu})
    seconds = float(cfg["bench"]["seconds"])
    results = []
    print(f"{'num_envs':>8} {'threads':>7} {'env_steps/s':>12}")
    for n in cfg["bench"]["num_envs"]:
        for threads in grid:
            c = deep_merge(cfg, {"env": {"num_envs": int(n), "threads": threads}})
            envw = EnvWrapper(c, images, starts, device, seed=int(c["run"]["seed"]))
            with torch.no_grad():
                obs = envw.reset()
                for _ in range(10):
                    obs = _step(envw, featurizer, model, obs)
                _sync(device)
                t0, steps = time.perf_counter(), 0
                while time.perf_counter() - t0 < seconds:
                    obs = _step(envw, featurizer, model, obs)
                    steps += 1
                _sync(device)
                dt = time.perf_counter() - t0
            row = {"num_envs": int(n), "threads": min(threads, int(n)), "steps": steps, "seconds": dt,
                   "env_steps_per_s": steps * int(n) * envw.frame_skip / dt}
            results.append(row)
            print(f"{row['num_envs']:>8} {row['threads']:>7} {row['env_steps_per_s']:>12.0f}")
            del envw
    best = max(results, key=lambda r: r["env_steps_per_s"])
    out = {"machine": machine_info(), "stg_rl": stg_rl.build_info(), "results": results,
           "recommended": {"num_envs": best["num_envs"], "threads": best["threads"]}}
    (Path(out_dir) / "bench.json").write_text(json.dumps(out, indent=2, ensure_ascii=False), encoding="utf-8")
    print(f"推荐：env.num_envs = {best['num_envs']}，env.threads = {best['threads']}")
    return out
```

- [ ] **Step 5: 实现 `gpucheck.py`**

```python
"""GPU 验收（spec §8）：同一初始权重、同一批 rollout 数据，compile + CUDA 图 开 / 关 各做一次 train_step，
比较损失统计量的相对误差（阈值 1e-4）。

用法：uv run --frozen python -m stgtrain.gpucheck configs/base.toml
"""
from __future__ import annotations

import argparse

import torch

from .config import deep_merge, load_config
from .envwrap import EnvWrapper
from .episodes import EpisodeTracker
from .ppo import PPO
from .registry import MODELS
from .reward import RewardFn
from .train import build_components

TOLERANCE = 1e-4


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m stgtrain.gpucheck")
    ap.add_argument("config")
    args = ap.parse_args(argv)
    if not torch.cuda.is_available():
        print("gpucheck 需要 CUDA")
        return 2
    device = torch.device("cuda")
    base = load_config(args.config, {"run": {"device": "cuda"}})
    images, starts, _, featurizer, _ = build_components(base, device)
    spec = featurizer.spec()

    def make(ppo_over: dict) -> PPO:
        c = deep_merge(base, {"ppo": ppo_over})
        cls = MODELS.get(c["model"]["name"])
        torch.manual_seed(0)
        return PPO(c, lambda: cls(c, spec), device)

    off = make({"compile": False, "cudagraphs": False})
    on = make({"compile": True, "cudagraphs": True})
    envw = EnvWrapper(base, images, starts, device, seed=0)
    rf = RewardFn(base)
    tracker = EpisodeTracker(envw.n, device, list(rf.terms), base["reward"]["hold_radius"],
                             base["reward"]["edge_margin"], envw.frame_skip, base["intent"]["interval"][1])
    _, container, next_value = off.rollout(envw, featurizer, rf, tracker, None, envw.reset())
    stats = {}
    for name, ppo in (("off", off), ("on", on)):
        torch.manual_seed(123)
        stats[name] = ppo.train_step(container.clone(), next_value.clone(), 1, 10)
    worst = 0.0
    for k in ("pg_loss", "v_loss", "entropy_loss", "approx_kl"):
        a, b = stats["off"][k], stats["on"][k]
        rel = abs(a - b) / max(abs(a), 1e-8)
        worst = max(worst, rel)
        print(f"{k:>14}  off={a:.8g}  on={b:.8g}  rel={rel:.3g}")
    ok = worst <= TOLERANCE
    print("PASS" if ok else f"FAIL（最大相对误差 {worst:.3g} > {TOLERANCE}）")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 6: `run.sh`**

```bash
#!/usr/bin/env bash
# 一条命令：装依赖（uv sync --frozen）→ 训练 / 续训 / 测速。参数原样交给 python -m stgtrain.train。
#   bash run.sh configs/base.toml my-run
#   bash run.sh --resume runs/<目录> [--total-updates N]
#   bash run.sh configs/base.toml my-box --bench
set -euo pipefail
cd "$(dirname "$0")"
if ! command -v uv >/dev/null 2>&1; then
  curl -LsSf https://astral.sh/uv/install.sh | sh
  export PATH="$HOME/.local/bin:$PATH"
fi
uv sync --frozen
exec uv run --frozen python -m stgtrain.train "$@"
```

Run: `chmod +x run.sh && uv run --frozen pytest -q`
Expected: 全部 PASS（`test_smoke.py` 在 CPU 上约 1 分钟内）。

- [ ] **Step 7: 真跑一条命令**

Run: `bash run.sh configs/smoke.toml smoke`
Expected: 结束时打印 `runs/<时间>-smoke.tar.gz`；`ls runs/*-smoke/plots` 有 `ppo.png`、`ep.png`、`load.png`；`cat runs/*-smoke/env.json` 含 `perf_summary`。

Run: `bash run.sh configs/smoke.toml box --bench`
Expected: 打印吞吐表与「推荐：env.num_envs = …」。

Run: `rm -rf runs`（runs/ 已在 .gitignore，清掉本地产物）

- [ ] **Step 8: Commit + 推送 + CI**

```bash
git add -A
git commit -m "feat: train 编排（训练/评测/checkpoint/续训/出图/打包）+ bench 扫线程×env + gpucheck + run.sh + base/smoke 配置

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
git push
gh run watch -R Renko6626/stg-rl-train --exit-status $(gh run list -R Renko6626/stg-rl-train --limit 1 --json databaseId --jq '.[0].databaseId')
```

Expected: CI 绿。若 CI 失败，读日志定位修复后再推，不得跳过。

---

### Task 13: 文档收口（训练仓 + stg-engine）

**Files:**
- Create（训练仓）: `docs/2026-09-15-stg-rl-train-design.md`, `docs/2026-09-15-stg-rl-train-plan.md`
- Modify（stg-engine）: `docs/superpowers/specs/2026-09-15-stg-rl-train-design.md`（状态行 + 新增 §13）、`PROGRESS.md`（重写「现在」段 + 里程碑史加一行）、`CLAUDE.md`（Milestone 地图 M5 条）、`docs/follow-ups.md`（D23 第 1 条）

**Interfaces:**
- Consumes: Task 12 Step 2/7 的实测结果（pytest 通过数、冒烟产物）

- [ ] **Step 1: stg-engine spec —— 状态行与 §13 实施偏差**

`docs/superpowers/specs/2026-09-15-stg-rl-train-design.md` 第 3 行

```markdown
> 状态：**设计已拍板，待写计划**。
```

改为

```markdown
> 状态：**第一刀已落地**（训练仓 `Renko6626/stg-rl-train`；实施计划 `docs/superpowers/plans/2026-09-15-stg-rl-train.md`）。GPU 验收（§8）待首次上 Vast.ai 回填。
```

文末追加：

```markdown
## 13. 实施偏差

写计划（`docs/superpowers/plans/2026-09-15-stg-rl-train.md` §Rulings）时对本文的裁定：

| # | 偏差 | 理由 |
|---|---|---|
| 1 | 刷新步的遵从塑形不置 0，用旧指令点精确计算；只保留「终局步记 0」 | reward 在刷新之前算、两个距离都对 `prev.target_xy`，本就没有目标突变的跳变；§3.5 表格与 §8 对应单测按此理解 |
| 2 | 到达用时：每局记各刷新段到达帧数的均值（未到达记 `interval` 上限），评测汇总取各局中位数 | 逐段中位数需要在设备上维护变长列表，收益不抵复杂度 |
| 3 | 新增 `cards.py` / `episodes.py` / `bench.py` / `gpucheck.py` | 卡池、逐局统计、测速、GPU 验收各自独立成文件，训练与评测共用逐局统计 |
| 4 | 打包在 `train.py` 里用 `tarfile` 完成，`run.sh` 只装依赖 + 调入口 | 便于测试 |
| 5 | 评测每个 (卡, rank) 开一个 `num_envs = episodes` 的 VecEnv，每个 env 只取第一局 | VecEnv 按权重随机抽起点，无法保证每组恰好 N 局 |
| 6 | 训练仓不加 LICENSE；`ppo.py` 头部保留 LeanRL MIT 许可全文 | stg-engine 同样无 LICENSE，由作者另定 |
| 7 | 续训时 env 种子 = `run.seed + 起始更新号 − 1` | 避免续训重放同一批局（§5 已声明非逐字节续训） |
```

- [ ] **Step 2: stg-engine PROGRESS / CLAUDE.md / follow-ups**

`PROGRESS.md`：把「## 现在（…）」整段替换为（`<pytest 通过数>` 用 Task 12 Step 6 实测输出的数字替换）：

```markdown
## 现在（2026-09-15）

- **位置**：**stg-rl-train 第一刀落地**（新仓 `Renko6626/stg-rl-train`；spec `docs/superpowers/specs/2026-09-15-stg-rl-train-design.md`，计划 `docs/superpowers/plans/2026-09-15-stg-rl-train.md`）——
  躲弹小模型训练仓：意图点胶水 / 动作表 v1 / 危险度 top-K 特征化 / 势函数遵从 reward / 注册表可切换模型 /
  LeanRL 底本 PPO（compile + CUDA 图）/ 固定评测集 / metrics + TensorBoard + 出图 / 开销记录 + `--bench` / `run.sh` 一条命令。
  依赖钉 `stg_rl` `rl-v0.1.0` wheel + `stgagent` `v0.1.0`。RL 卡池写作约束 `docs/rl-card-pool.md`（写卡另开会话）。
- **实证**：训练仓 CPU 上 `pytest` <pytest 通过数> passed（含端到端训练→评测→续训→出图→打包冒烟）；GitHub CI 绿。GPU 路径未验。
- **下一步**：首次上 Vast.ai 做 GPU 验收（spec §8：bench / gpucheck / 1 小时训练 / 打断续训）→ 卡池交付后正式训练 →
  第二刀 Vast.ai 自动化 → 第三刀 ONNX 导出与 C 侧对拍；与**第 1 关内容刀**（gameplay-design §6）并列。
  `rl-v0.1.0` wheel 的 `build_info()["git_sha"]` 为 `unknown`，修复后发 `rl-v0.1.1`。
- **待办**：见 [`docs/follow-ups.md`](docs/follow-ups.md)。
```

里程碑史表头分隔行（`|---|---|---|`）下方插入一行：

```markdown
| 2026-09-15 | **stg-rl-train 第一刀（训练仓）** | 新仓 `Renko6626/stg-rl-train`（spec `2026-09-15-stg-rl-train-design.md`，Ruling 1–7 见 §13）：uv 钉版本、注册表（模型/特征化器/意图/reward 项）、`envwrap`（CSR 铺定长、镜像、意图刷新）、`danger_topk_v1`、reward 七项、`set_attn_v1`、LeanRL 底本 PPO + 自描述 checkpoint、固定评测集、metrics/plots/perf、`train`/`bench`/`gpucheck`/`run.sh`。CPU pytest 全绿 + CI 绿；GPU 验收待 Vast.ai。 |
```

`CLAUDE.md` Milestone 地图里 M5 那条的末尾「训练代码 / 特征化 / reward / 训练作业包在训练仓（本刀非目标，spec §12）。」改为：

```markdown
  训练代码 / 特征化 / reward 在训练仓 `Renko6626/stg-rl-train`（第一刀 2026-09-15，spec `2026-09-15-stg-rl-train-design.md`）；
  训练作业包（Vast.ai 自动化）是其第二刀。
```

`docs/follow-ups.md` D23 第 1 条行首追加：`**已由训练仓承接（2026-09-15，stg-rl-train 第一刀）**：`

- [ ] **Step 3: Commit（stg-engine，不推送）**

```bash
cd /data/sunyunbo/www/stg-engine
git add docs/superpowers/specs/2026-09-15-stg-rl-train-design.md PROGRESS.md CLAUDE.md docs/follow-ups.md
git commit -m "docs: stg-rl-train 第一刀收口——spec 状态与 §13 实施偏差、PROGRESS/CLAUDE.md/follow-ups D23#1

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

- [ ] **Step 4: 训练仓复制设计与计划（必须在 Step 3 提交之后，才能带上 §13 与正确的 commit 号）**

```bash
cd /data/sunyunbo/www/stg-rl-train && mkdir -p docs
E=/data/sunyunbo/www/stg-engine
SHA=$(git -C $E log -1 --format=%h -- docs/superpowers/specs/2026-09-15-stg-rl-train-design.md)
{ echo "> 复制自 stg-engine \`docs/superpowers/specs/2026-09-15-stg-rl-train-design.md\`（stg-engine commit $SHA）。以训练仓这份为准继续演进。"; echo; cat $E/docs/superpowers/specs/2026-09-15-stg-rl-train-design.md; } > docs/2026-09-15-stg-rl-train-design.md
{ echo "> 复制自 stg-engine \`docs/superpowers/plans/2026-09-15-stg-rl-train.md\`（第一刀实施计划，历史记录）。"; echo; cat $E/docs/superpowers/plans/2026-09-15-stg-rl-train.md; } > docs/2026-09-15-stg-rl-train-plan.md
git add docs && git commit -m "docs: 迁入第一刀设计与实施计划

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" && git push
```

---

### Task 14（人工，不派 subagent）：Vast.ai GPU 验收

由用户租机、控制者远程协助完成；结果回填 spec §8 勾选框与训练仓 `docs/perf-baseline.md`（新建）。

- [ ] 租机：GPU 任意单卡，**CPU 逻辑核 ≥ 32**（env 吞吐主要吃 CPU），镜像带 `curl` 与 `git` 即可。
- [ ] `git clone https://github.com/Renko6626/stg-rl-train && cd stg-rl-train && bash run.sh configs/base.toml box --bench`：记录 `bench.json` 的推荐 `(num_envs, threads)` 与机器画像。
- [ ] `uv run --frozen python -m stgtrain.gpucheck configs/base.toml`：期望 `PASS`；若 `FAIL`，记录各项相对误差，再决定放宽阈值还是查 CUDA 图用法。
- [ ] 按 bench 推荐改 `configs/base.toml` 的 `env.num_envs` / `env.threads`（注意 `num_envs × num_steps` 须能被 `num_minibatches` 整除），`bash run.sh configs/base.toml accept` 连续跑 ≥ 1 小时无错；记录 `env.json` 的 `perf_summary`（SPS、各阶段占比、GPU 等 CPU 占比）。
- [ ] 中途 Ctrl+C，`bash run.sh --resume runs/<目录>` 能接着跑。
- [ ] `scp` 回 tar 包，本地 `uv run --frozen python -m stgtrain.plots runs/<目录>.tar.gz --out /tmp/plots` 出图正常。
- [ ] 卡池为空时 `train_starts` 会报「没有可用的训练起点」：验收前先把至少一张卡放进 `cards/`（可暂用 `tests/fixtures/cards/example_ring`）。
