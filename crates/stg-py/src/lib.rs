//! `stg_rl._native` —— PyO3 abi3 薄壳（spec `docs/superpowers/specs/2026-09-15-stg-rl-env-design.md`）。
//!
//! 只做三件事：把 Python 值翻译成 `stg-rl` 的类型、把 17 个调用方缓冲的 numpy 视图借成
//! `BufferSet`、在批量 step 期间用 `py.detach` 释放 GIL。所有世界语义住 `stg-rl`。
//!
//! 缓冲区一律**一维**（多维形状由 Python 包装层 `reshape` 出视图），Rust 只见扁平切片；
//! dtype / C 连续校验在取数组与 `as_slice_mut` 两处做，报错消息含缓冲键名。

use std::collections::HashMap;

use numpy::{Element, PyArray1, PyArrayMethods, PyReadwriteArray1};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use stg_core::tables::TABLES_V0;
use stg_rl::bundled;
use stg_rl::env::{self, EndOn, EnvConfig, Start};
use stg_rl::layout;
use stg_rl::vec_env::{self, BufferSet};

create_exception!(_native, CompileError, PyException);

/// 编译好的只读镜像（Python 持有；多个 env 共享同一份 `Arc<EclImage>`）。
#[pyclass]
struct Image(env::Image);

/// 从 dict 按固定键取一个可写一维数组；缺失 / dtype 不符 / 只读 / 已被借用 ⇒ `ValueError`
/// （消息含键名）。
///
/// **不得**改回 `extract::<PyReadwriteArray1>()`：其内部 `readwrite()` =
/// `try_readwrite().unwrap()`，只读或同一数组挂两个键时会 Rust panic，以 `PanicException`
/// （`BaseException` 子类，`except Exception` 抓不住）穿透到 Python。这里先转成 `PyArray1`
/// （纯类型转换，不 panic），再用 `try_readwrite()` 把 `NotWriteable` / `AlreadyBorrowed`
/// 映射成 `ValueError`。
fn take<'py, T: Element>(
    bufs: &Bound<'py, PyDict>,
    key: &str,
) -> PyResult<PyReadwriteArray1<'py, T>> {
    let obj = bufs
        .get_item(key)?
        .ok_or_else(|| PyValueError::new_err(format!("buffers 缺少键 {key:?}")))?;
    let arr = obj
        .extract::<Bound<'py, PyArray1<T>>>()
        .map_err(|e| PyValueError::new_err(format!("buffer {key:?}: {e}")))?;
    arr.try_readwrite()
        .map_err(|e| PyValueError::new_err(format!("buffer {key:?}: {e}")))
}

/// 17 个 numpy 可写数组的持有者——活得比它借出的 `BufferSet` 久。
struct OwnedBufs<'py> {
    frame: PyReadwriteArray1<'py, u32>,
    phase: PyReadwriteArray1<'py, u32>,
    player: PyReadwriteArray1<'py, u8>,
    enemies: PyReadwriteArray1<'py, u8>,
    enemies_count: PyReadwriteArray1<'py, i32>,
    bullets: PyReadwriteArray1<'py, u8>,
    bullets_offsets: PyReadwriteArray1<'py, i32>,
    items: PyReadwriteArray1<'py, u8>,
    items_offsets: PyReadwriteArray1<'py, i32>,
    lasers_count: PyReadwriteArray1<'py, i32>,
    bullets_total: PyReadwriteArray1<'py, i32>,
    bullets_dropped: PyReadwriteArray1<'py, i32>,
    events: PyReadwriteArray1<'py, i32>,
    done: PyReadwriteArray1<'py, u8>,
    ep_frames: PyReadwriteArray1<'py, i32>,
    warmup_retries: PyReadwriteArray1<'py, i32>,
    start_index: PyReadwriteArray1<'py, i32>,
}

impl<'py> OwnedBufs<'py> {
    fn new(bufs: &Bound<'py, PyDict>) -> PyResult<Self> {
        Ok(Self {
            frame: take::<u32>(bufs, "frame")?,
            phase: take::<u32>(bufs, "phase")?,
            player: take::<u8>(bufs, "player")?,
            enemies: take::<u8>(bufs, "enemies")?,
            enemies_count: take::<i32>(bufs, "enemies_count")?,
            bullets: take::<u8>(bufs, "bullets")?,
            bullets_offsets: take::<i32>(bufs, "bullets_offsets")?,
            items: take::<u8>(bufs, "items")?,
            items_offsets: take::<i32>(bufs, "items_offsets")?,
            lasers_count: take::<i32>(bufs, "lasers_count")?,
            bullets_total: take::<i32>(bufs, "bullets_total")?,
            bullets_dropped: take::<i32>(bufs, "bullets_dropped")?,
            events: take::<i32>(bufs, "events")?,
            done: take::<u8>(bufs, "done")?,
            ep_frames: take::<i32>(bufs, "ep_frames")?,
            warmup_retries: take::<i32>(bufs, "warmup_retries")?,
            start_index: take::<i32>(bufs, "start_index")?,
        })
    }

    /// 把各数组借成互不相交的 `&mut` 切片。非 C 连续 ⇒ `ValueError`（消息含键名）。
    fn as_set(&mut self) -> Result<BufferSet<'_>, String> {
        fn sl<'a, T: Element>(
            arr: &'a mut PyReadwriteArray1<'_, T>,
            key: &str,
        ) -> Result<&'a mut [T], String> {
            arr.as_slice_mut()
                .map_err(|_| format!("buffer {key:?}: 需要 C 连续数组"))
        }
        Ok(BufferSet {
            frame: sl(&mut self.frame, "frame")?,
            phase: sl(&mut self.phase, "phase")?,
            player: sl(&mut self.player, "player")?,
            enemies: sl(&mut self.enemies, "enemies")?,
            enemies_count: sl(&mut self.enemies_count, "enemies_count")?,
            bullets: sl(&mut self.bullets, "bullets")?,
            bullets_offsets: sl(&mut self.bullets_offsets, "bullets_offsets")?,
            items: sl(&mut self.items, "items")?,
            items_offsets: sl(&mut self.items_offsets, "items_offsets")?,
            lasers_count: sl(&mut self.lasers_count, "lasers_count")?,
            bullets_total: sl(&mut self.bullets_total, "bullets_total")?,
            bullets_dropped: sl(&mut self.bullets_dropped, "bullets_dropped")?,
            events: sl(&mut self.events, "events")?,
            done: sl(&mut self.done, "done")?,
            ep_frames: sl(&mut self.ep_frames, "ep_frames")?,
            warmup_retries: sl(&mut self.warmup_retries, "warmup_retries")?,
            start_index: sl(&mut self.start_index, "start_index")?,
        })
    }
}

/// 批量 env（`Send`，rayon 专属池；GIL 在 `step` 期间释放）。
#[pyclass]
struct NativeVecEnv(vec_env::VecEnv);

#[pymethods]
impl NativeVecEnv {
    #[new]
    #[allow(clippy::too_many_arguments)] // Python 侧构造参数逐个显式，拆结构体反而绕
    fn new(
        num_envs: usize,
        threads: usize,
        images: Vec<PyRef<'_, Image>>,
        starts: Vec<(usize, i32, i32, f64)>,
        frame_skip: u32,
        max_frames: u32,
        warmup_max: u32,
        end_on: Vec<String>,
        bullets_cap: usize,
        seed: u64,
    ) -> PyResult<Self> {
        let images = images.into_iter().map(|i| i.0.clone()).collect();
        let starts = starts
            .into_iter()
            .map(|(image, mark, rank, weight)| Start {
                image,
                mark,
                rank,
                weight,
            })
            .collect();
        let end_on = end_on
            .into_iter()
            .map(|name| match name.as_str() {
                "phase_ended" => Ok(EndOn::PhaseEnded),
                "spell_captured" => Ok(EndOn::SpellCaptured),
                "spell_failed" => Ok(EndOn::SpellFailed),
                "stage_cleared" => Ok(EndOn::StageCleared),
                other => Err(PyValueError::new_err(format!(
                    "未知 end_on 事件 {other:?}（合法：phase_ended/spell_captured/spell_failed/stage_cleared）"
                ))),
            })
            .collect::<PyResult<Vec<_>>>()?;
        let cfg = EnvConfig {
            images,
            starts,
            frame_skip,
            max_frames,
            warmup_max,
            end_on,
            bullets_cap,
            seed,
        };
        vec_env::VecEnv::new(cfg, num_envs, threads)
            .map(Self)
            .map_err(PyValueError::new_err)
    }

    fn reset(&mut self, py: Python<'_>, bufs: &Bound<'_, PyDict>) -> PyResult<()> {
        let mut owned = OwnedBufs::new(bufs)?;
        let mut set = owned.as_set().map_err(PyValueError::new_err)?;
        py.detach(|| self.0.reset(&mut set))
            .map_err(PyValueError::new_err)
    }

    fn step(
        &mut self,
        py: Python<'_>,
        actions: Bound<'_, PyArray1<u32>>,
        bufs: &Bound<'_, PyDict>,
    ) -> PyResult<()> {
        // 先 `try_readonly()` 再 `as_slice()`：不依赖会 panic 的 `PyReadonlyArray1` 提取。
        let actions = actions
            .try_readonly()
            .map_err(|e| PyValueError::new_err(format!("actions: {e}")))?;
        let actions = actions
            .as_slice()
            .map_err(|_| PyValueError::new_err("actions: 需要 C 连续 uint32 数组"))?;
        let mut owned = OwnedBufs::new(bufs)?;
        let mut set = owned.as_set().map_err(PyValueError::new_err)?;
        py.detach(|| self.0.step(actions, &mut set))
            .map_err(PyValueError::new_err)
    }

    fn set_start_weights(&mut self, w: Vec<f64>) -> PyResult<()> {
        self.0.set_start_weights(w).map_err(PyValueError::new_err)
    }
}

/// proto v1 HELLO JSON（`bullets_cap` 须在 `1..=BULLETS_CAP_MAX`）。
#[pyfunction]
fn hello(bullets_cap: usize) -> PyResult<String> {
    if !(1..=layout::BULLETS_CAP_MAX).contains(&bullets_cap) {
        return Err(PyValueError::new_err(format!(
            "bullets_cap {bullets_cap} 越界（合法 1..={}）",
            layout::BULLETS_CAP_MAX
        )));
    }
    Ok(layout::hello_json(bullets_cap))
}

/// `(表名, stride, [(字段名, 偏移, 类型名)])` 的行。
type LayoutTable = (String, usize, Vec<(String, usize, String)>);

/// `layout_tables()` 的返回：proto v1 五张表的字段布局。
#[pyfunction]
fn layout_tables() -> Vec<LayoutTable> {
    layout::TABLES
        .iter()
        .map(|t| {
            (
                t.name.to_string(),
                t.stride,
                t.fields
                    .iter()
                    .map(|f| (f.name.to_string(), f.off, f.ty.name().to_string()))
                    .collect(),
            )
        })
        .collect()
}

/// 表容量常量（Python 包装层据此算缓冲长度，避免与 Rust 常量各写一份）。
#[pyfunction]
fn caps() -> HashMap<&'static str, usize> {
    HashMap::from([
        ("enemies", layout::ENEMIES_CAP),
        ("lasers", layout::LASERS_CAP),
        ("items", layout::ITEMS_CAP),
        ("bullets_default", layout::BULLETS_CAP_DEFAULT),
        ("bullets_max", layout::BULLETS_CAP_MAX),
    ])
}

/// events 列名（列序即契约）。
#[pyfunction]
fn event_columns() -> Vec<&'static str> {
    layout::EVENT_COLUMNS.to_vec()
}

/// wheel 身份四元组：包版本 / 引擎版 / 表内容哈希 / git 短 SHA。
#[pyfunction]
fn build_info() -> HashMap<&'static str, String> {
    HashMap::from([
        ("version", env!("CARGO_PKG_VERSION").to_string()),
        ("engine_ver", stg_core::ENGINE_VER.to_string()),
        ("tables_hash", format!("{:016x}", TABLES_V0.content_hash)),
        ("git_sha", env!("STG_GIT_SHA").to_string()),
    ])
}

/// 内置 ECL 内容包（文件名 + 源码），按文件名排序。未知包 ⇒ `ValueError`。
#[pyfunction]
fn bundled_sources(pack: &str) -> PyResult<Vec<(String, String)>> {
    bundled::bundled_sources(pack)
        .map(|units| {
            units
                .iter()
                .map(|(name, src)| (name.to_string(), src.to_string()))
                .collect()
        })
        .ok_or_else(|| PyValueError::new_err(format!("未知内容包 {pack:?}")))
}

/// 编译 ECL 源码单元 → 只读镜像；编译失败 ⇒ `CompileError`（消息含文件名 / 行列）。
#[pyfunction]
fn compile_sources(units: Vec<(String, String)>) -> PyResult<Image> {
    env::compile(&units)
        .map(Image)
        .map_err(CompileError::new_err)
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("CompileError", m.py().get_type::<CompileError>())?;
    m.add_class::<Image>()?;
    m.add_class::<NativeVecEnv>()?;
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    m.add_function(wrap_pyfunction!(layout_tables, m)?)?;
    m.add_function(wrap_pyfunction!(caps, m)?)?;
    m.add_function(wrap_pyfunction!(event_columns, m)?)?;
    m.add_function(wrap_pyfunction!(build_info, m)?)?;
    m.add_function(wrap_pyfunction!(bundled_sources, m)?)?;
    m.add_function(wrap_pyfunction!(compile_sources, m)?)?;
    Ok(())
}
