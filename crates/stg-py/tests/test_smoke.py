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
