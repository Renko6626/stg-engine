//! stg-godot——WorldBridge gdext cdylib（spec 2026-07-24 §3/§12，薄壳厚核）。
//! 断层线以上：f32 只活在 frame.rs；Godot 只加载 .so 不编译（M2）。

use godot::prelude::*;

pub mod boot;
pub mod bridge;
pub mod frame;
pub mod puppets;
pub mod save;

struct StgGodotExtension;

#[gdextension]
unsafe impl ExtensionLibrary for StgGodotExtension {}
