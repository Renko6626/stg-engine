//! ecl/image.rs —— 只读脚本镜像（`EclImage`）：字节码 + 入口表 + 内容哈希（占位）。
//!
//! 静态数据，**不进 `World`**（I7：World 内无引用/指针）；`&EclImage` 参数穿线与
//! `&WorldTables` 同款——`step`/`step_with_director` 按帧传参消费，两机同镜像由二进制
//! 同一性/哈希保证，不随快照回滚。无脚本场景传 [`EclImage::empty`]（零任务即零成本，
//! T2 金向量一号逐位不变门）。

use crate::ecl::task::LOCALS;

#[repr(transparent)]
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    crate::checksum::Checksum,
    crate::save::SaveBytes,
)]
pub struct SubId(u16);

impl SubId {
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryId(u16);

impl EntryId {
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedEntry<'a> {
    image: &'a EclImage,
    id: EntryId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveError {
    RootRequiresStartMain,
    UnknownEntry,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EclValueType {
    Int = 0,
    Fx = 1,
    Angle = 2,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubKind {
    Root = 0,
    Async = 1,
    CallOnly = 2,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeSubMeta {
    code_entry: u32,
    param_start: u16,
    param_count: u8,
    kind: SubKind,
}

impl RuntimeSubMeta {
    pub const fn code_entry(self) -> u32 {
        self.code_entry
    }

    pub const fn kind(self) -> SubKind {
        self.kind
    }

    pub const fn param_count(self) -> u8 {
        self.param_count
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeEntryMeta {
    name_offset: u32,
    name_len: u16,
    sub: SubId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubInit {
    code_entry: u32,
    kind: SubKind,
    params: Vec<EclValueType>,
}

impl SubInit {
    pub fn new(code_entry: u32, kind: SubKind, params: Vec<EclValueType>) -> Self {
        Self {
            code_entry,
            kind,
            params,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryInit {
    name: String,
    sub: u16,
}

impl EntryInit {
    pub fn new(name: impl Into<String>, sub: u16) -> Self {
        Self {
            name: name.into(),
            sub,
        }
    }
}

pub struct ImageParts {
    pub code: Vec<u32>,
    pub subs: Vec<SubInit>,
    pub entries: Vec<EntryInit>,
    pub root: Option<u16>,
    /// 中段启动标记表：`(id, ip)` 对，`id` 是脚本作者写的 `mark(id)` 编号（正整数），`ip` 是
    /// 该标记在 `code` 里的落点垫片首指令（全局绝对字索引）。调用方（`stg-ecl-compiler` 的
    /// `ImageBuilder::build`）负责按 `id` 严格升序排好——`try_from_parts` 只校验，不排序
    /// （整局流程刀 spec §2；Task 6 `new_game_at` 经 [`EclImage::resolve_mark`] 消费）。
    pub marks: Vec<(i32, u32)>,
    pub content_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageBuildError {
    TooManySubs {
        actual: usize,
    },
    TooManyEntries {
        actual: usize,
    },
    CodeTooLong {
        words: usize,
    },
    NamePoolTooLarge {
        bytes: usize,
    },
    NameTooLong {
        name: String,
        bytes: usize,
    },
    InvalidEntryName {
        name: String,
    },
    EntriesNotStrictlySorted,
    MissingRoot,
    MultipleRoots,
    RootIndexMismatch,
    RootHasParameters,
    CodeEntryOutOfRange {
        sub: usize,
        code_entry: u32,
    },
    TooManyParameters {
        sub: usize,
        actual: usize,
    },
    ParameterTableTooLarge {
        actual: usize,
    },
    EntrySubOutOfRange {
        entry: usize,
        sub: u16,
    },
    /// `marks` 里存在 `id <= 0`（`mark` 编号契约是正整数，`typeck::validate_marks` 本该
    /// 在编译期已挡下，这里是运行时镜像契约的第二道防线）。
    MarkIdNonPositive,
    /// `marks` 未按 `id` 严格升序排列（含重复 id——`build()` 端的编译期查重理应已挡下，
    /// 这里同上是契约防线，不是唯一权威判定点）。
    MarksNotStrictlySorted,
    /// `marks` 里某条落点 `ip >= code.len()`（垫片首指令必须落在合法指令边界内）。
    MarkIpOutOfBounds,
    EntryKindMismatch {
        entry: usize,
        kind: SubKind,
    },
    MissingAsyncEntry {
        sub: usize,
    },
    DuplicateAsyncEntry {
        sub: usize,
    },
    DuplicateSubName {
        name: String,
    },
    InvalidRootName {
        name: String,
    },
    ReservedMainKind {
        kind: SubKind,
    },
    InvalidBuilderRef,
    UndefinedSub {
        name: String,
    },
    DuplicateDefinition {
        name: String,
    },
    WrongTargetKind {
        target: String,
        expected: SubKind,
        actual: SubKind,
    },
    WrongTargetArity {
        target: String,
        expected: u8,
        actual: u8,
    },
    OperandOverflow,
}

/// 脚本镜像：`code` 是全部子程序共享的扁平字流（`Task.pc` 是其**绝对**字索引）。
/// 构造期将可读的初始化记录压紧成不可变运行表，运行期不再持有名字 `String` 或参数 `Vec`。
#[derive(Debug, PartialEq, Eq)]
pub struct EclImage {
    code: Box<[u32]>,
    subs: Box<[RuntimeSubMeta]>,
    param_types: Box<[EclValueType]>,
    entries: Box<[RuntimeEntryMeta]>,
    entry_names: Box<[u8]>,
    root: Option<SubId>,
    /// 中段启动标记表：按 `id` 严格升序（`try_from_parts` 校验），`resolve_mark` 二分查找。
    marks: Box<[(i32, u32)]>,
    content_hash: u64,
}

impl EclImage {
    pub fn empty() -> EclImage {
        EclImage {
            code: Box::new([]),
            subs: Box::new([]),
            param_types: Box::new([]),
            entries: Box::new([]),
            entry_names: Box::new([]),
            root: None,
            marks: Box::new([]),
            content_hash: 0,
        }
    }

    pub fn try_from_parts(parts: ImageParts) -> Result<Self, ImageBuildError> {
        const U16_DOMAIN: usize = u16::MAX as usize + 1;

        if parts.subs.len() > U16_DOMAIN {
            return Err(ImageBuildError::TooManySubs {
                actual: parts.subs.len(),
            });
        }
        if parts.entries.len() > U16_DOMAIN {
            return Err(ImageBuildError::TooManyEntries {
                actual: parts.entries.len(),
            });
        }
        if u32::try_from(parts.code.len()).is_err() {
            return Err(ImageBuildError::CodeTooLong {
                words: parts.code.len(),
            });
        }

        // 标记表校验（Task 4；整局流程刀 spec §2）：id 全正 → 严格升序 → 落点在合法指令
        // 边界内。故意放在"空镜像哨兵"分支之前——空 `code` 时任何非空 `marks` 的落点必然
        // 越界，让它照常在这里被拒，不必在哨兵分支里另开一条特判。
        if parts.marks.iter().any(|m| m.0 <= 0) {
            return Err(ImageBuildError::MarkIdNonPositive);
        }
        if parts.marks.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(ImageBuildError::MarksNotStrictlySorted);
        }
        if parts.marks.iter().any(|m| m.1 as usize >= parts.code.len()) {
            return Err(ImageBuildError::MarkIpOutOfBounds);
        }

        if parts.code.is_empty()
            && parts.subs.is_empty()
            && parts.entries.is_empty()
            && parts.root.is_none()
        {
            return Ok(Self {
                content_hash: parts.content_hash,
                ..Self::empty()
            });
        }

        let roots: Vec<usize> = parts
            .subs
            .iter()
            .enumerate()
            .filter_map(|(index, sub)| (sub.kind == SubKind::Root).then_some(index))
            .collect();
        let root_index = match roots.as_slice() {
            [] => return Err(ImageBuildError::MissingRoot),
            [root] => *root,
            _ => return Err(ImageBuildError::MultipleRoots),
        };
        if parts.root.map(usize::from) != Some(root_index) {
            return Err(ImageBuildError::RootIndexMismatch);
        }
        if !parts.subs[root_index].params.is_empty() {
            return Err(ImageBuildError::RootHasParameters);
        }

        let mut param_total = 0usize;
        for (sub_index, sub) in parts.subs.iter().enumerate() {
            if sub.code_entry as usize >= parts.code.len() {
                return Err(ImageBuildError::CodeEntryOutOfRange {
                    sub: sub_index,
                    code_entry: sub.code_entry,
                });
            }
            if sub.params.len() > LOCALS {
                return Err(ImageBuildError::TooManyParameters {
                    sub: sub_index,
                    actual: sub.params.len(),
                });
            }
            param_total = param_total
                .checked_add(sub.params.len())
                .ok_or(ImageBuildError::ParameterTableTooLarge { actual: usize::MAX })?;
        }
        if param_total > U16_DOMAIN {
            return Err(ImageBuildError::ParameterTableTooLarge {
                actual: param_total,
            });
        }

        for entry in &parts.entries {
            if entry.name.len() > u16::MAX as usize {
                return Err(ImageBuildError::NameTooLong {
                    name: entry.name.clone(),
                    bytes: entry.name.len(),
                });
            }
            if entry.name == "main" || !is_valid_identifier(&entry.name) {
                return Err(ImageBuildError::InvalidEntryName {
                    name: entry.name.clone(),
                });
            }
        }
        if parts
            .entries
            .windows(2)
            .any(|pair| pair[0].name >= pair[1].name)
        {
            return Err(ImageBuildError::EntriesNotStrictlySorted);
        }

        let mut async_entry_seen = vec![false; parts.subs.len()];
        for (entry_index, entry) in parts.entries.iter().enumerate() {
            let Some(sub) = parts.subs.get(entry.sub as usize) else {
                return Err(ImageBuildError::EntrySubOutOfRange {
                    entry: entry_index,
                    sub: entry.sub,
                });
            };
            if sub.kind != SubKind::Async {
                return Err(ImageBuildError::EntryKindMismatch {
                    entry: entry_index,
                    kind: sub.kind,
                });
            }
            if async_entry_seen[entry.sub as usize] {
                return Err(ImageBuildError::DuplicateAsyncEntry {
                    sub: entry.sub as usize,
                });
            }
            async_entry_seen[entry.sub as usize] = true;
        }
        for (sub_index, sub) in parts.subs.iter().enumerate() {
            if sub.kind == SubKind::Async && !async_entry_seen[sub_index] {
                return Err(ImageBuildError::MissingAsyncEntry { sub: sub_index });
            }
        }

        let mut param_types = Vec::with_capacity(param_total);
        let mut runtime_subs = Vec::with_capacity(parts.subs.len());
        for sub in &parts.subs {
            let param_start = if sub.params.is_empty() {
                0
            } else {
                u16::try_from(param_types.len()).map_err(|_| {
                    ImageBuildError::ParameterTableTooLarge {
                        actual: param_total,
                    }
                })?
            };
            let param_count =
                u8::try_from(sub.params.len()).map_err(|_| ImageBuildError::TooManyParameters {
                    sub: runtime_subs.len(),
                    actual: sub.params.len(),
                })?;
            param_types.extend_from_slice(&sub.params);
            runtime_subs.push(RuntimeSubMeta {
                code_entry: sub.code_entry,
                param_start,
                param_count,
                kind: sub.kind,
            });
        }

        let mut entry_names = Vec::new();
        let mut runtime_entries = Vec::with_capacity(parts.entries.len());
        for entry in parts.entries {
            let name_offset = u32::try_from(entry_names.len()).map_err(|_| {
                ImageBuildError::NamePoolTooLarge {
                    bytes: entry_names.len(),
                }
            })?;
            let name_len =
                u16::try_from(entry.name.len()).map_err(|_| ImageBuildError::NameTooLong {
                    name: entry.name.clone(),
                    bytes: entry.name.len(),
                })?;
            let new_len = entry_names
                .len()
                .checked_add(entry.name.len())
                .ok_or(ImageBuildError::NamePoolTooLarge { bytes: usize::MAX })?;
            if new_len > u32::MAX as usize {
                return Err(ImageBuildError::NamePoolTooLarge { bytes: new_len });
            }
            entry_names.extend_from_slice(entry.name.as_bytes());
            runtime_entries.push(RuntimeEntryMeta {
                name_offset,
                name_len,
                sub: SubId(entry.sub),
            });
        }

        Ok(Self {
            code: parts.code.into_boxed_slice(),
            subs: runtime_subs.into_boxed_slice(),
            param_types: param_types.into_boxed_slice(),
            entries: runtime_entries.into_boxed_slice(),
            entry_names: entry_names.into_boxed_slice(),
            root: Some(SubId(u16::try_from(root_index).map_err(|_| {
                ImageBuildError::TooManySubs {
                    actual: parts.subs.len(),
                }
            })?)),
            marks: parts.marks.into_boxed_slice(),
            content_hash: parts.content_hash,
        })
    }

    pub fn code(&self) -> &[u32] {
        &self.code
    }

    pub const fn content_hash(&self) -> u64 {
        self.content_hash
    }

    pub fn sub_count(&self) -> usize {
        self.subs.len()
    }

    pub fn sub_id(&self, raw: u16) -> Option<SubId> {
        self.subs.get(raw as usize).map(|_| SubId(raw))
    }

    pub const fn root(&self) -> Option<SubId> {
        self.root
    }

    pub fn sub_meta(&self, sub: SubId) -> Option<&RuntimeSubMeta> {
        self.subs.get(sub.0 as usize)
    }

    pub fn param_types(&self, sub: SubId) -> Option<&[EclValueType]> {
        let meta = self.subs.get(sub.0 as usize)?;
        let start = meta.param_start as usize;
        let end = start.checked_add(meta.param_count as usize)?;
        self.param_types.get(start..end)
    }

    pub fn resolve_entry(&self, name: &str) -> Result<ResolvedEntry<'_>, ResolveError> {
        if name == "main" {
            return Err(ResolveError::RootRequiresStartMain);
        }
        let index = self
            .entries
            .binary_search_by(|entry| self.entry_name(*entry).cmp(name))
            .map_err(|_| ResolveError::UnknownEntry)?;
        Ok(ResolvedEntry {
            image: self,
            id: EntryId(u16::try_from(index).expect("validated entry count fits u16 domain")),
        })
    }

    /// 中段启动标记表：id → main 内落点(全局绝对字索引)。整局流程刀 spec §2。
    pub fn resolve_mark(&self, id: i32) -> Option<u32> {
        self.marks
            .binary_search_by_key(&id, |m| m.0)
            .ok()
            .map(|i| self.marks[i].1)
    }

    fn entry_name(&self, entry: RuntimeEntryMeta) -> &str {
        let start = entry.name_offset as usize;
        let end = start + entry.name_len as usize;
        std::str::from_utf8(&self.entry_names[start..end])
            .expect("constructor stores validated ASCII entry names")
    }
}

impl<'a> ResolvedEntry<'a> {
    pub const fn id(self) -> EntryId {
        self.id
    }

    pub fn sub(self) -> SubId {
        self.image.entries[self.id.0 as usize].sub
    }

    pub fn meta(self) -> &'a RuntimeSubMeta {
        self.image
            .sub_meta(self.sub())
            .expect("entry sub validated")
    }

    /// Returns `true` if the entry ID is within the image's entry table.
    /// Used by `spawn_entry` to reject out-of-range entry IDs.
    pub(crate) fn is_valid_entry_id(self) -> bool {
        (self.id.0 as usize) < self.image.entries.len()
    }

    /// 背书镜像(binding 层 coherence 守卫用;字段模块私有,crate 内经此读)。
    pub(crate) fn image(&self) -> &'a EclImage {
        self.image
    }
}

// Test-only: construct a ResolvedEntry from a raw entry index.
// Used by binding tests to exercise InvalidEntryId.
#[cfg(test)]
impl<'a> ResolvedEntry<'a> {
    #[allow(dead_code)]
    pub(crate) fn test_from_raw(image: &'a EclImage, raw: u16) -> Self {
        ResolvedEntry {
            image,
            id: EntryId(raw),
        }
    }
}

pub(crate) fn is_valid_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
pub(crate) fn test_image(
    code: Vec<u32>,
    subs: Vec<SubInit>,
    entries: Vec<EntryInit>,
    root: Option<u16>,
) -> EclImage {
    EclImage::try_from_parts(ImageParts {
        code,
        subs,
        entries,
        root,
        marks: vec![],
        content_hash: 0,
    })
    .expect("test image must satisfy the runtime image contract")
}

impl Default for EclImage {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecl::task::LOCALS;

    #[test]
    fn image_freezes_compact_tables_and_resolves_async_names() {
        let image = EclImage::try_from_parts(ImageParts {
            code: vec![0, 0, 0],
            subs: vec![
                SubInit::new(0, SubKind::Async, vec![EclValueType::Fx]),
                SubInit::new(1, SubKind::Root, vec![]),
                SubInit::new(2, SubKind::CallOnly, vec![]),
            ],
            entries: vec![EntryInit::new("bullet_task", 0)],
            root: Some(1),
            marks: vec![],
            content_hash: 0,
        })
        .unwrap();

        assert_eq!(image.code(), &[0, 0, 0]);
        assert_eq!(image.resolve_entry("bullet_task").unwrap().id().get(), 0);
        assert_eq!(
            image.resolve_entry("main"),
            Err(ResolveError::RootRequiresStartMain)
        );
        assert_eq!(
            image.resolve_entry("helper"),
            Err(ResolveError::UnknownEntry)
        );
        assert_eq!(
            image.sub_meta(image.root().unwrap()).unwrap().kind(),
            SubKind::Root
        );
    }

    #[test]
    fn runtime_records_are_compact() {
        assert_eq!(std::mem::size_of::<RuntimeSubMeta>(), 8);
        assert_eq!(std::mem::size_of::<RuntimeEntryMeta>(), 8);
    }

    fn parts_with_entries(entries: Vec<EntryInit>) -> ImageParts {
        ImageParts {
            code: vec![0, 0, 0],
            subs: vec![
                SubInit::new(0, SubKind::Async, vec![]),
                SubInit::new(1, SubKind::Async, vec![]),
                SubInit::new(2, SubKind::Root, vec![]),
            ],
            entries,
            root: Some(2),
            marks: vec![],
            content_hash: 7,
        }
    }

    #[test]
    fn rejects_duplicate_and_unsorted_entry_names() {
        let duplicate = EclImage::try_from_parts(parts_with_entries(vec![
            EntryInit::new("same", 0),
            EntryInit::new("same", 1),
        ]));
        assert_eq!(duplicate, Err(ImageBuildError::EntriesNotStrictlySorted));

        let unsorted = EclImage::try_from_parts(parts_with_entries(vec![
            EntryInit::new("zeta", 0),
            EntryInit::new("alpha", 1),
        ]));
        assert_eq!(unsorted, Err(ImageBuildError::EntriesNotStrictlySorted));
    }

    #[test]
    fn rejects_non_ascii_or_invalid_ecl_entry_identifiers() {
        for name in ["弹幕", "9worker", "with-dash", ""] {
            let result = EclImage::try_from_parts(parts_with_entries(vec![
                EntryInit::new(name, 0),
                EntryInit::new("valid", 1),
            ]));
            assert_eq!(
                result,
                Err(ImageBuildError::InvalidEntryName {
                    name: name.to_owned()
                })
            );
        }
    }

    #[test]
    fn rejects_async_entry_named_main() {
        let result = EclImage::try_from_parts(parts_with_entries(vec![
            EntryInit::new("main", 0),
            EntryInit::new("worker", 1),
        ]));
        assert_eq!(
            result,
            Err(ImageBuildError::InvalidEntryName {
                name: "main".to_owned(),
            })
        );
    }

    #[test]
    fn accepts_only_the_exact_empty_sentinel_without_a_root() {
        assert_eq!(
            EclImage::try_from_parts(ImageParts {
                code: vec![],
                subs: vec![],
                entries: vec![],
                root: None,
                marks: vec![],
                content_hash: 0,
            }),
            Ok(EclImage::empty())
        );

        let nonempty = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs: vec![SubInit::new(0, SubKind::CallOnly, vec![])],
            entries: vec![],
            root: None,
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(nonempty, Err(ImageBuildError::MissingRoot));
    }

    #[test]
    fn rejects_multiple_roots_root_mismatch_and_root_parameters() {
        let multiple = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs: vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Root, vec![]),
            ],
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(multiple, Err(ImageBuildError::MultipleRoots));

        let mismatch = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs: vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::CallOnly, vec![]),
            ],
            entries: vec![],
            root: Some(1),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(mismatch, Err(ImageBuildError::RootIndexMismatch));

        let params = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs: vec![SubInit::new(0, SubKind::Root, vec![EclValueType::Int])],
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(params, Err(ImageBuildError::RootHasParameters));
    }

    #[test]
    fn rejects_entries_that_do_not_name_exactly_one_async_sub() {
        let wrong_kind = EclImage::try_from_parts(ImageParts {
            code: vec![0, 0],
            subs: vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(1, SubKind::CallOnly, vec![]),
            ],
            entries: vec![EntryInit::new("helper", 1)],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(
            wrong_kind,
            Err(ImageBuildError::EntryKindMismatch {
                entry: 0,
                kind: SubKind::CallOnly,
            })
        );

        let missing =
            EclImage::try_from_parts(parts_with_entries(vec![EntryInit::new("worker_a", 0)]));
        assert_eq!(missing, Err(ImageBuildError::MissingAsyncEntry { sub: 1 }));

        let duplicate = EclImage::try_from_parts(parts_with_entries(vec![
            EntryInit::new("worker_a", 0),
            EntryInit::new("worker_b", 0),
        ]));
        assert_eq!(
            duplicate,
            Err(ImageBuildError::DuplicateAsyncEntry { sub: 0 })
        );
    }

    #[test]
    fn rejects_parameter_count_and_flattened_table_overflow() {
        let per_sub = EclImage::try_from_parts(ImageParts {
            code: vec![0, 0],
            subs: vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(1, SubKind::CallOnly, vec![EclValueType::Int; LOCALS + 1]),
            ],
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(
            per_sub,
            Err(ImageBuildError::TooManyParameters {
                sub: 1,
                actual: LOCALS + 1,
            })
        );

        let mut subs = Vec::with_capacity(u16::MAX as usize + 1);
        subs.push(SubInit::new(0, SubKind::Root, vec![]));
        subs.push(SubInit::new(
            0,
            SubKind::CallOnly,
            vec![EclValueType::Int; 3],
        ));
        subs.extend(
            (2..=u16::MAX).map(|_| SubInit::new(0, SubKind::CallOnly, vec![EclValueType::Int])),
        );
        let overflow = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs,
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(
            overflow,
            Err(ImageBuildError::ParameterTableTooLarge { actual: 65_537 })
        );
    }

    #[test]
    fn pins_u16_sub_and_entry_count_domains() {
        let mut max_subs = Vec::with_capacity(u16::MAX as usize + 1);
        max_subs.push(SubInit::new(0, SubKind::Root, vec![]));
        max_subs.extend((1..=u16::MAX).map(|_| SubInit::new(0, SubKind::CallOnly, vec![])));
        let image = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs: max_subs,
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        })
        .unwrap();
        assert_eq!(image.sub_id(u16::MAX).unwrap().get(), u16::MAX);

        let too_many_subs = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs: (0..65_537)
                .map(|_| SubInit::new(0, SubKind::CallOnly, vec![]))
                .collect(),
            entries: vec![],
            root: None,
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(
            too_many_subs,
            Err(ImageBuildError::TooManySubs { actual: 65_537 })
        );

        let too_many_entries = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs: vec![SubInit::new(0, SubKind::Root, vec![])],
            entries: (0..65_537).map(|_| EntryInit::new("x", 0)).collect(),
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(
            too_many_entries,
            Err(ImageBuildError::TooManyEntries { actual: 65_537 })
        );
    }

    #[test]
    fn zero_length_param_ranges_canonicalize_start_and_nonempty_end_may_equal_65536() {
        let mut subs = Vec::with_capacity(u16::MAX as usize + 1);
        subs.push(SubInit::new(0, SubKind::Root, vec![]));
        subs.extend(
            (1..u16::MAX).map(|_| SubInit::new(0, SubKind::CallOnly, vec![EclValueType::Int])),
        );
        subs.push(SubInit::new(
            0,
            SubKind::CallOnly,
            vec![EclValueType::Int; 2],
        ));
        let image = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs,
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        })
        .unwrap();
        assert!(image.param_types(image.root().unwrap()).unwrap().is_empty());
        assert_eq!(
            image.param_types(image.sub_id(u16::MAX).unwrap()).unwrap(),
            &[EclValueType::Int, EclValueType::Int]
        );
    }

    #[test]
    fn rejects_code_entry_and_entry_sub_out_of_range() {
        let code_entry = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs: vec![SubInit::new(1, SubKind::Root, vec![])],
            entries: vec![],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(
            code_entry,
            Err(ImageBuildError::CodeEntryOutOfRange {
                sub: 0,
                code_entry: 1,
            })
        );

        let entry_sub = EclImage::try_from_parts(ImageParts {
            code: vec![0],
            subs: vec![SubInit::new(0, SubKind::Root, vec![])],
            entries: vec![EntryInit::new("worker", 1)],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(
            entry_sub,
            Err(ImageBuildError::EntrySubOutOfRange { entry: 0, sub: 1 })
        );
    }

    #[test]
    fn rejects_names_longer_than_u16() {
        let name = "a".repeat(u16::MAX as usize + 1);
        let result = EclImage::try_from_parts(ImageParts {
            code: vec![0, 0],
            subs: vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(1, SubKind::Async, vec![]),
            ],
            entries: vec![EntryInit::new(name.clone(), 1)],
            root: Some(0),
            marks: vec![],
            content_hash: 0,
        });
        assert_eq!(
            result,
            Err(ImageBuildError::NameTooLong {
                name,
                bytes: u16::MAX as usize + 1,
            })
        );
    }

    // ── Task 4：中段启动标记表 ───────────────────────────────────────────

    fn parts_for_marks(marks: Vec<(i32, u32)>) -> ImageParts {
        ImageParts {
            code: vec![0, 0, 0],
            subs: vec![SubInit::new(0, SubKind::Root, vec![])],
            entries: vec![],
            root: Some(0),
            marks,
            content_hash: 0,
        }
    }

    #[test]
    fn resolve_mark_finds_registered_ip() {
        let image = EclImage::try_from_parts(parts_for_marks(vec![(3, 1)]))
            .expect("单条合法 mark（id=3, ip=1 < code.len()=3）应通过契约校验");
        assert_eq!(image.resolve_mark(3), Some(1));
        assert_eq!(image.resolve_mark(4), None, "未注册的 id 不该命中");
        assert_eq!(image.resolve_mark(0), None, "id=0 非法，自然也查不到");
    }

    #[test]
    fn marks_must_be_sorted_and_in_bounds() {
        let unsorted = EclImage::try_from_parts(parts_for_marks(vec![(5, 0), (3, 0)]));
        assert_eq!(unsorted, Err(ImageBuildError::MarksNotStrictlySorted));

        let code_len = 3usize;
        let out_of_bounds =
            EclImage::try_from_parts(parts_for_marks(vec![(3, code_len as u32 + 10)]));
        assert_eq!(out_of_bounds, Err(ImageBuildError::MarkIpOutOfBounds));
    }

    #[test]
    fn mark_id_must_be_positive() {
        let result = EclImage::try_from_parts(parts_for_marks(vec![(0, 1)]));
        assert_eq!(result, Err(ImageBuildError::MarkIdNonPositive));
    }
}
