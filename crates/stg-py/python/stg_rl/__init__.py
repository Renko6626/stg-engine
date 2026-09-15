"""stg_rl —— stg-engine 强化学习批量 env（spec docs/superpowers/specs/2026-09-15-stg-rl-env-design.md）。

单进程：Rust 专属 rayon 池批量 step，写入本包分配的缓冲。**不要**再套 gymnasium AsyncVectorEnv /
SB3 SubprocVecEnv / make_vec_env —— 那会把整个 env 复制进子进程，吃掉全部性能。
训练侧请设置 torch.set_num_threads，避免与 rayon 抢核。
"""
from __future__ import annotations
from dataclasses import dataclass
from pathlib import Path
import numpy as np
from . import _native
from ._native import CompileError, Image, build_info, bundled_sources, event_columns

EVENT_COLUMNS = tuple(event_columns())
STRIDES = {name: stride for name, stride, _ in _native.layout_tables()}
OFFSETS = {name: {f: (off, ty) for f, off, ty in fields} for name, _, fields in _native.layout_tables()}
ENEMIES_CAP, ITEMS_CAP = 256, 1024
_END_ON = ("phase_ended", "spell_captured", "spell_failed", "stage_cleared")


def hello(bullets_cap: int = 1024) -> str:
    return _native.hello(bullets_cap)


def compile_sources(units) -> Image:
    return _native.compile_sources([(str(n), str(s)) for n, s in units])


def compile_dir(path) -> Image:
    files = sorted(Path(path).glob("*.ecl"), key=lambda p: p.name)
    if not files:
        raise ValueError(f"no .ecl files in {path}")
    return compile_sources([(p.name, p.read_text(encoding="utf-8")) for p in files])


def compile_bundled(pack: str = "game") -> Image:
    return compile_sources(bundled_sources(pack))


@dataclass
class Start:
    image: str
    mark: int = 0
    rank: int = 2
    weight: float = 1.0


def _layout(n: int, cap: int):
    """(键, 扁平长度, dtype, 视图形状)。与 Rust vec_env::buffer_sizes 一一对应。"""
    return [
        ("frame", n, np.uint32, (n,)), ("phase", n, np.uint32, (n,)),
        ("player", n * 36, np.uint8, (n, 36)),
        ("enemies", n * ENEMIES_CAP * 38, np.uint8, (n, ENEMIES_CAP, 38)), ("enemies_count", n, np.int32, (n,)),
        ("bullets", n * cap * 30, np.uint8, (n * cap, 30)), ("bullets_offsets", n + 1, np.int32, (n + 1,)),
        ("items", n * ITEMS_CAP * 18, np.uint8, (n * ITEMS_CAP, 18)), ("items_offsets", n + 1, np.int32, (n + 1,)),
        ("lasers_count", n, np.int32, (n,)), ("bullets_total", n, np.int32, (n,)), ("bullets_dropped", n, np.int32, (n,)),
        ("events", n * len(EVENT_COLUMNS), np.int32, (n, len(EVENT_COLUMNS))), ("done", n, np.uint8, (n,)),
        ("ep_frames", n, np.int32, (n,)), ("warmup_retries", n, np.int32, (n,)), ("start_index", n, np.int32, (n,)),
    ]


def alloc_buffers(num_envs: int, bullets_cap: int = 1024, backend: str = "torch", pin: bool = True) -> dict:
    """返回 {键: 形状化数组}。backend="torch" ⇒ torch 张量（pin=True 时 pinned）；"numpy" ⇒ numpy 数组。"""
    out = {}
    for key, flat, dt, shape in _layout(num_envs, bullets_cap):
        if backend == "torch":
            try:
                import torch
            except ImportError as e:
                raise ImportError("backend='torch' 需要安装 torch；测试 / CPU 调试请用 backend='numpy'") from e
            t = torch.from_numpy(np.zeros(flat, dtype=dt))
            out[key] = (t.pin_memory() if pin else t).reshape(shape)
        elif backend == "numpy":
            out[key] = np.zeros(flat, dtype=dt).reshape(shape)
        else:
            raise ValueError(f"unknown backend {backend!r}")
    return out


def _flat_numpy(buffers: dict, n: int, cap: int) -> dict:
    flat = {}
    for key, length, dt, _ in _layout(n, cap):
        if key not in buffers:
            raise ValueError(f"buffers 缺少键 {key!r}")
        a = buffers[key]
        a = a.numpy() if hasattr(a, "numpy") and not isinstance(a, np.ndarray) else a
        a = a.reshape(-1)
        if a.dtype != dt or a.size != length or not a.flags["C_CONTIGUOUS"]:
            raise ValueError(f"buffer {key!r}: 需要 {np.dtype(dt)}×{length} 连续数组，得 {a.dtype}×{a.size}")
        flat[key] = a
    return flat


class VecEnv:
    def __init__(self, num_envs: int, threads: int, images: dict, starts: list, frame_skip: int = 1,
                 max_frames: int = 3600, warmup_max: int = 120, end_on=_END_ON, bullets_cap: int = 1024,
                 seed: int = 0, buffers: dict | None = None):
        names = list(images)
        for s in starts:
            if s.image not in images:
                raise ValueError(f"Start.image {s.image!r} 不在 images 里")
        self.num_envs, self.bullets_cap = num_envs, bullets_cap
        self._native = _native.NativeVecEnv(
            num_envs, threads, [images[k] for k in names],
            [(names.index(s.image), s.mark, s.rank, float(s.weight)) for s in starts],
            frame_skip, max_frames, warmup_max, list(end_on), bullets_cap, seed)
        self.buffers = buffers if buffers is not None else alloc_buffers(num_envs, bullets_cap, backend="numpy")
        self._flat = _flat_numpy(self.buffers, num_envs, bullets_cap)

    def reset(self) -> dict:
        self._native.reset(self._flat)
        return self.buffers

    def step(self, actions) -> dict:
        a = actions.numpy() if hasattr(actions, "numpy") and not isinstance(actions, np.ndarray) else actions
        self._native.step(np.ascontiguousarray(a, dtype=np.uint32).reshape(-1), self._flat)
        return self.buffers

    def set_start_weights(self, weights) -> None:
        self._native.set_start_weights([float(w) for w in weights])
