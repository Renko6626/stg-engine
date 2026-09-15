import json
import numpy as np
import pytest
import stg_rl


def make(n=4, threads=2, cap=256, seed=0, **kw):
    img = stg_rl.compile_bundled("game")
    return stg_rl.VecEnv(n, threads, {"game": img}, [stg_rl.Start("game", mark=15)], bullets_cap=cap, seed=seed, **kw)


def test_build_info_and_hello():
    info = stg_rl.build_info()
    assert {"version", "engine_ver", "tables_hash", "git_sha"} <= set(info)
    h = json.loads(stg_rl.hello(512))
    assert h["backend"] == f"stg-engine@{info['engine_ver']}"
    assert [t["cap"] for t in h["tables"]] == [1, 512, 256, 64, 1024]
    assert stg_rl.STRIDES == {"player": 36, "bullets": 30, "enemies": 38, "lasers": 48, "items": 18}
    assert stg_rl.EVENT_COLUMNS[0] == "died" and len(stg_rl.EVENT_COLUMNS) == 8


def test_reset_step_shapes_and_done():
    env = make()
    b = env.reset()
    assert b["player"].shape == (4, 36) and b["bullets"].shape == (4 * 256, 30)
    seen = set()
    rng = np.random.default_rng(0)
    for _ in range(600):
        env.step(rng.integers(0, 128, size=4, dtype=np.uint32))
        seen |= set(np.unique(b["done"]).tolist())
        assert b["bullets_offsets"][0] == 0 and np.all(np.diff(b["bullets_offsets"]) >= 0)
    assert seen & {1, 2, 3}, "600 步随机动作应结束过至少一局"


def test_decode_with_stgagent_matches_offsets():
    schema = pytest.importorskip("stgagent.schema")
    env = make(n=2)
    b = env.reset()
    for _ in range(120):
        env.step(np.zeros(2, dtype=np.uint32))
    hello = schema.parse_hello(stg_rl.hello(256).encode())
    t = hello.table_by_name("bullets")
    n = int(b["bullets_offsets"][-1])
    if n == 0:
        pytest.skip("此刻场上无弹")
    rows = np.frombuffer(b["bullets"][:n].tobytes(), dtype=t.dtype)
    off = stg_rl.OFFSETS["bullets"]["x"][0]
    manual = np.frombuffer(b["bullets"][:n, off:off + 4].tobytes(), dtype="<i4")
    assert np.array_equal(rows["x"], manual)


def test_errors():
    img = stg_rl.compile_bundled("game")
    with pytest.raises(ValueError):
        stg_rl.VecEnv(2, 1, {"game": img}, [stg_rl.Start("nope")])
    with pytest.raises(ValueError):
        stg_rl.VecEnv(2, 1, {"game": img}, [stg_rl.Start("game", mark=999)])
    with pytest.raises(stg_rl.CompileError) as ei:
        stg_rl.compile_sources([("bad.ecl", "sub main( {")])
    assert "bad.ecl" in str(ei.value)
    bad = stg_rl.alloc_buffers(2, 128, backend="numpy")
    with pytest.raises(ValueError):
        stg_rl.VecEnv(2, 1, {"game": img}, [stg_rl.Start("game", mark=15)], bullets_cap=256, buffers=bad)
    with pytest.raises(ValueError):
        stg_rl.hello(0)


def test_writes_land_in_caller_buffer():
    """正常分配的缓冲：step 后写入必须落到调用方缓冲（非全零）。"""
    env = make()
    b = env.reset()
    for _ in range(10):
        env.step(np.zeros(4, dtype=np.uint32))
    assert np.any(b["player"] != 0), "player 写入应落到调用方缓冲"


def test_readonly_buffer_raises_valueerror():
    """只读缓冲 ⇒ ValueError（不是 PanicException；PanicException 继承 BaseException，抓不住）。"""
    img = stg_rl.compile_bundled("game")
    bad = stg_rl.alloc_buffers(2, 256, backend="numpy")
    bad["done"].setflags(write=False)
    env = stg_rl.VecEnv(2, 1, {"game": img}, [stg_rl.Start("game", mark=15)],
                        bullets_cap=256, buffers=bad)
    with pytest.raises(ValueError):
        env.reset()


def test_aliased_buffers_raise_valueerror():
    """两个键指向同一数组 ⇒ ValueError（numpy 动态借用检查以 ValueError 表现，不 panic）。"""
    img = stg_rl.compile_bundled("game")
    bad = stg_rl.alloc_buffers(2, 256, backend="numpy")
    bad["phase"] = bad["frame"]  # 同一 uint32 数组挂两个键
    env = stg_rl.VecEnv(2, 1, {"game": img}, [stg_rl.Start("game", mark=15)],
                        bullets_cap=256, buffers=bad)
    with pytest.raises(ValueError):
        env.reset()


def test_noncontiguous_buffer_raises_valueerror():
    """行跨步（非 C 连续）缓冲区 ⇒ 构造期 ValueError，绝不静默拷贝。"""
    img = stg_rl.compile_bundled("game")
    n = 2
    bad = stg_rl.alloc_buffers(n, 256, backend="numpy")
    bad["player"] = np.zeros((2 * n, 36), np.uint8)[::2]  # 形状/dtype/size 都对，但非连续
    with pytest.raises(ValueError):
        stg_rl.VecEnv(n, 1, {"game": img}, [stg_rl.Start("game", mark=15)],
                      bullets_cap=256, buffers=bad)


def test_negative_args_raise_valueerror():
    """负数 usize/u64 参数 ⇒ ValueError（包装层拦截，而非 Rust 的 OverflowError）。"""
    with pytest.raises(ValueError):
        stg_rl.hello(-1)
    img = stg_rl.compile_bundled("game")
    with pytest.raises(ValueError):
        stg_rl.VecEnv(-1, 1, {"game": img}, [stg_rl.Start("game", mark=15)])


def test_torch_backend_matches_numpy():
    """torch 缓冲（CPU、pin=False）与 numpy 路径逐缓冲一致，且写入落在调用方张量。"""
    pytest.importorskip("torch")
    img = stg_rl.compile_bundled("game")
    starts = [stg_rl.Start("game", mark=15)]
    tb = stg_rl.alloc_buffers(4, 256, backend="torch", pin=False)
    nb = stg_rl.alloc_buffers(4, 256, backend="numpy")
    te = stg_rl.VecEnv(4, 2, {"game": img}, starts, bullets_cap=256, seed=3, buffers=tb)
    ne = stg_rl.VecEnv(4, 2, {"game": img}, starts, bullets_cap=256, seed=3, buffers=nb)
    te.reset()
    ne.reset()
    acts = np.zeros(4, dtype=np.uint32)
    for _ in range(40):
        te.step(acts)
        ne.step(acts)
    for key in ("frame", "phase", "done", "bullets_offsets", "player", "bullets"):
        assert np.array_equal(tb[key].numpy(), nb[key]), key
    assert tb["player"].abs().sum().item() > 0, "写入应落在调用方 torch 张量"


def test_caps_and_strides_come_from_native():
    """Python 缓冲布局的容量与 stride 取自原生模块（不与 Rust 常量各写一份）。"""
    assert stg_rl.CAPS == {"enemies": 256, "lasers": 64, "items": 1024,
                           "bullets_default": 1024, "bullets_max": 8192}
    b = stg_rl.alloc_buffers(3, 128, backend="numpy")
    assert b["enemies"].shape == (3, stg_rl.CAPS["enemies"], stg_rl.STRIDES["enemies"])
    assert b["items"].shape == (3 * stg_rl.CAPS["items"], stg_rl.STRIDES["items"])
    assert b["bullets"].shape == (3 * 128, stg_rl.STRIDES["bullets"])
