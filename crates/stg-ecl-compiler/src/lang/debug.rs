//! ECL 编译器调试信息侧载（Task 4）——`CompileOptions { debug_info: DebugInfo::Full }` 时构建的符号表，作为
//! [`CompiledEcl`] 的可选 `.debug` 字段输出。**永不出现在 `stg-core` 或运行期
//! `EclImage` 中**：EclImage 本身在有无调试信息时逐字节完全相同。
//!
//! 侧载表格使用单一的 `Box<[u8]>` 字符串池（UTF-8 字节拼接），`DebugSubMeta`/
//! `DebugParamMeta` 通过 `(offset, len)` 索引池中的名字。查询方法返回 `&str` 切片，
//! 生命周期绑定到 `EclDebugSymbols` 实例。

use crate::lang::ast::{Program, Ty};
use crate::lang::typeck::TypedInfo;
use std::collections::BTreeMap;
use stg_core::ecl::image::{EclImage, EclValueType, SubKind};

/// 一条 sub 的调试元数据：名字（字符串池索引）、运行期 `SubKind`、参数范围、
/// 绝对 PC 区间 `[pc_start, pc_end)`、源码定位（文件 + 行/列）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugSubMeta {
    name_offset: u32,
    name_len: u16,
    kind: SubKind,
    param_start: u16,
    param_count: u8,
    pc_start: u32,
    pc_end: u32,
    file_offset: u32,
    file_len: u16,
    line: u32,
    col: u32,
}

impl DebugSubMeta {
    /// 该 sub 的运行期种类（`Root`/`Async`/`CallOnly`）。
    pub fn kind(&self) -> SubKind {
        self.kind
    }

    /// 该 sub 在 `EclImage.code()` 中的绝对起始 PC。
    pub fn pc_start(&self) -> u32 {
        self.pc_start
    }

    /// 该 sub 在 `EclImage.code()` 中的绝对结束 PC（独占上界）。
    pub fn pc_end(&self) -> u32 {
        self.pc_end
    }
}

/// 一条参数的调试元数据：名字（字符串池索引）、类型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugParamMeta {
    name_offset: u32,
    name_len: u16,
    ty: EclValueType,
}

/// 由 `EclDebugSymbols` 查询方法返回的一个 PC → (sub, 文件, 行, 列) 映射结果。
///
/// 生命周期绑定到产生它的 `EclDebugSymbols`。
#[derive(Clone, Debug)]
pub struct PcSourceSpan<'a> {
    sub_name: &'a str,
    file: &'a str,
    line: u32,
    col: u32,
}

impl<'a> PcSourceSpan<'a> {
    /// 拥有该 PC 的 sub 名称（来自字符串池）。
    pub fn sub_name(&self) -> &'a str {
        self.sub_name
    }

    /// 源文件名（调用 `compile_with_options` 时传入的 `file` 参数）。
    pub fn file(&self) -> &'a str {
        self.file
    }

    /// 源码行号（1-based）。
    pub fn line(&self) -> u32 {
        self.line
    }

    /// 源码列号（1-based）。
    pub fn col(&self) -> u32 {
        self.col
    }
}

/// 可选的调试符号侧载——`CompileOptions { debug_info: DebugInfo::Full }` 模式下编译时生成，**不在运行期使用**。
///
/// 所有名称字符串保存在单一的 `Box<[u8]>` 池中，通过 `(offset, len)` 索引。
/// sub 元数据数组与 `EclImage` 的规范顺序（按名称字典序）一致。
#[derive(Clone, Debug)]
pub struct EclDebugSymbols {
    string_pool: Box<[u8]>,
    subs: Box<[DebugSubMeta]>,
    params: Box<[DebugParamMeta]>,
}

impl EclDebugSymbols {
    /// 按名称查找 sub 调试元数据。
    ///
    /// 对 `CallOnly` sub（如辅助函数 `helper`）同样有效——这些 sub 不在 `EclImage` 的
    /// entry 表中，但调试符号表保留其全部信息。
    pub fn symbol(&self, name: &str) -> Option<&DebugSubMeta> {
        self.subs
            .iter()
            .find(|sub| self.str_at(sub.name_offset, sub.name_len) == name)
    }

    /// 返回给定 sub 的第 `index` 个参数的名称（0-based），越界返回 `None`。
    pub fn param_name(&self, sub: &DebugSubMeta, index: usize) -> Option<&str> {
        if index >= sub.param_count as usize {
            return None;
        }
        let p = &self.params[sub.param_start as usize + index];
        Some(self.str_at(p.name_offset, p.name_len))
    }

    /// 将绝对 PC 映射到所属 sub 的源码定位。
    ///
    /// 返回包含 sub 名称、文件、行/列的 [`PcSourceSpan`]；若 PC 落在任何 sub 区间之外
    /// （空镜像 / 越界）则返回 `None`。
    pub fn source_at(&self, pc: u32) -> Option<PcSourceSpan<'_>> {
        let sub = self
            .subs
            .iter()
            .find(|s| pc >= s.pc_start && pc < s.pc_end)?;
        Some(PcSourceSpan {
            sub_name: self.str_at(sub.name_offset, sub.name_len),
            file: self.str_at(sub.file_offset, sub.file_len),
            line: sub.line,
            col: sub.col,
        })
    }

    /// 从字符串池中提取 UTF-8 字符串切片。
    fn str_at(&self, offset: u32, len: u16) -> &str {
        let start = offset as usize;
        let end = start + len as usize;
        std::str::from_utf8(&self.string_pool[start..end])
            .expect("debug string pool contains valid UTF-8")
    }
}

/// 从编译管线产物构建调试符号表。**只应在 `CompileOptions { debug_info: DebugInfo::Full }` 模式下调用**。
///
/// 管线总序：`parse → typeck → slots → codegen → [build image] → (if Full) this`。
/// 本函数消费 `Program`（源码 AST，含 `SubDef.span`）、`TypedInfo`（带型信息，
/// 含参数名列表）、以及最终产出的 `EclImage`（规范顺序子程序列表），
/// 产出紧凑的 `EclDebugSymbols`。
///
/// 字符串池布局：
/// 1. `file` 名（一次写入，所有 sub 共享同一份偏移+长度）
/// 2. 每个 sub 的名称（规范顺序）
/// 3. 每个 param 的名称（sub 内声明序）
pub(crate) fn build_debug_symbols(
    prog: &Program,
    ti: &TypedInfo,
    image: &EclImage,
    file: &str,
) -> EclDebugSymbols {
    // ── 按名称建立源码 AST / 类型信息的快速查找 ──────────────────────────
    let mut name_to_subdef = BTreeMap::new();
    for s in &prog.subs {
        name_to_subdef.insert(s.name.as_str(), s);
    }
    let mut name_to_typed = BTreeMap::new();
    for s in &ti.subs {
        name_to_typed.insert(s.name.as_str(), s);
    }

    // 规范顺序 = 名字典序（与 `ImageBuilder::build` 的 `order.sort_by(|l, r| ...)` 一致）。
    let mut canonical_names: Vec<&str> = ti.subs.iter().map(|s| s.name.as_str()).collect();
    canonical_names.sort();

    // ── 构建字符串池 ─────────────────────────────────────────────────
    let mut pool: Vec<u8> = Vec::new();

    // 文件名字符串（所有 sub 共享同一偏移+长度）。
    let file_offset = pool.len() as u32;
    let file_len = file.len() as u16;
    pool.extend_from_slice(file.as_bytes());

    let mut subs_meta = Vec::with_capacity(canonical_names.len());
    let mut params_meta = Vec::new();

    for (canonical_index, name) in canonical_names.iter().enumerate() {
        // 通过规范索引获取 `SubId` 与 `RuntimeSubMeta`（含 `code_entry`）。
        let Some(sub_id) = image.sub_id(canonical_index as u16) else {
            continue;
        };
        let Some(meta) = image.sub_meta(sub_id) else {
            continue;
        };
        let pc_start = meta.code_entry();

        // PC 终点：下一个 sub 的 `code_entry`，或镜像总长度（最后一个 sub）。
        let pc_end = if canonical_index + 1 < canonical_names.len() {
            image
                .sub_id((canonical_index + 1) as u16)
                .and_then(|next_id| image.sub_meta(next_id))
                .map(|next_meta| next_meta.code_entry())
                .unwrap_or_else(|| image.code().len() as u32)
        } else {
            image.code().len() as u32
        };

        let Some(sub_def) = name_to_subdef.get(name) else {
            continue;
        };
        let Some(typed_sub) = name_to_typed.get(name) else {
            continue;
        };

        // sub 名称入池。
        let name_offset = pool.len() as u32;
        let name_len = name.len() as u16;
        pool.extend_from_slice(name.as_bytes());

        let param_start = params_meta.len() as u16;
        let param_count = typed_sub.params.len() as u8;

        // 参数名称入池。
        for (param_name, param_ty) in &typed_sub.params {
            let p_off = pool.len() as u32;
            let p_len = param_name.len() as u16;
            pool.extend_from_slice(param_name.as_bytes());
            params_meta.push(DebugParamMeta {
                name_offset: p_off,
                name_len: p_len,
                ty: match param_ty {
                    Ty::Int => EclValueType::Int,
                    Ty::Fx => EclValueType::Fx,
                    Ty::Angle => EclValueType::Angle,
                },
            });
        }

        subs_meta.push(DebugSubMeta {
            name_offset,
            name_len,
            kind: meta.kind(),
            param_start,
            param_count,
            pc_start,
            pc_end,
            file_offset,
            file_len,
            line: sub_def.span.line,
            col: sub_def.span.col,
        });
    }

    EclDebugSymbols {
        string_pool: pool.into_boxed_slice(),
        subs: subs_meta.into_boxed_slice(),
        params: params_meta.into_boxed_slice(),
    }
}

#[cfg(test)]
mod tests {
    use crate::lang::{CompileOptions, DebugInfo, compile_with_options};
    use stg_core::ecl::image::SubKind;

    /// 查询不存在的 sub 应返回 `None`。
    #[test]
    fn symbol_lookup_unknown_returns_none() {
        let src = "sub main() {}";
        let out = compile_with_options(
            src,
            "empty.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
        )
        .unwrap();
        assert!(out.debug.is_some());
        let dbg = out.debug.unwrap();
        assert!(dbg.symbol("nonexistent").is_none());
    }

    /// `CallOnly` sub 的 `kind()` 应返回 `SubKind::CallOnly`。
    #[test]
    fn call_only_sub_has_call_only_kind() {
        let src = "sub helper(x: int) {} sub main() { helper(1); }";
        let out = compile_with_options(
            src,
            "t.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
        )
        .unwrap();
        let dbg = out.debug.unwrap();
        let helper = dbg.symbol("helper").expect("helper 应存在");
        assert_eq!(helper.kind(), SubKind::CallOnly);
    }

    /// root sub 的 `kind()` 应返回 `SubKind::Root`。
    #[test]
    fn root_sub_has_root_kind() {
        let src = "sub main() {}";
        let out = compile_with_options(
            src,
            "t.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
        )
        .unwrap();
        let dbg = out.debug.unwrap();
        let main = dbg.symbol("main").expect("main 应存在");
        assert_eq!(main.kind(), SubKind::Root);
    }

    /// async sub 的 `kind()` 应返回 `SubKind::Async`。
    #[test]
    fn async_sub_has_async_kind() {
        let src = "async sub task() {} sub main() { spawn task(); }";
        let out = compile_with_options(
            src,
            "t.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
        )
        .unwrap();
        let dbg = out.debug.unwrap();
        let task = dbg.symbol("task").expect("task 应存在");
        assert_eq!(task.kind(), SubKind::Async);
    }

    /// `param_name` 越界时应返回 `None`。
    #[test]
    fn param_name_out_of_range_returns_none() {
        let src = "sub helper(x: int) {} sub main() { helper(1); }";
        let out = compile_with_options(
            src,
            "t.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
        )
        .unwrap();
        let dbg = out.debug.unwrap();
        let helper = dbg.symbol("helper").expect("helper 应存在");
        assert_eq!(dbg.param_name(helper, 0), Some("x"));
        assert_eq!(dbg.param_name(helper, 1), None);
    }

    /// `source_at` 对越界 PC 应返回 `None`。
    #[test]
    fn source_at_out_of_range_returns_none() {
        let src = "sub main() {}";
        let out = compile_with_options(
            src,
            "t.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
        )
        .unwrap();
        let dbg = out.debug.unwrap();
        assert!(dbg.source_at(u32::MAX).is_none());
    }

    /// PC 区间 `[pc_start, pc_end)` 应非空且连续。
    #[test]
    fn pc_ranges_are_non_empty_and_contiguous() {
        let src = "sub a() { wait(1); } sub b() { wait(2); } sub main() { a(); b(); }";
        let out = compile_with_options(
            src,
            "t.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
        )
        .unwrap();
        let dbg = out.debug.unwrap();
        for sub in dbg.subs.iter() {
            assert!(
                sub.pc_start < sub.pc_end,
                "pc_start({}) < pc_end({})",
                sub.pc_start,
                sub.pc_end
            );
        }
        // 相邻 sub 的区间应无缝衔接。
        for pair in dbg.subs.windows(2) {
            assert_eq!(
                pair[0].pc_end,
                pair[1].pc_start,
                "sub '{}' 的 pc_end 应等于下一个 sub 的 pc_start",
                dbg.str_at(pair[0].name_offset, pair[0].name_len)
            );
        }
    }

    /// `EclDebugSymbols` 的 `Debug + Clone` 特征可用（用于 panic 消息与快照）。
    #[test]
    fn debug_symbols_is_debug_and_clone() {
        let src = "sub main() {}";
        let out = compile_with_options(
            src,
            "t.ecl",
            CompileOptions {
                debug_info: DebugInfo::Full,
            },
            &[],
        )
        .unwrap();
        let dbg = out.debug.unwrap();
        let _formatted = format!("{dbg:?}");
        let _cloned = dbg.clone();
    }
}
