//! stg-rl —— 强化学习批量 env 核心（spec `docs/superpowers/specs/2026-09-15-stg-rl-env-design.md`）。
//! 断层线以上：允许线程 / rayon；stg-core 语义不变。

pub mod encode;
pub mod env;
pub mod layout;
pub mod vec_env;

// 编译期断言（spec §7）：`Env` 可跨线程搬（rayon 批量 step）；`Image` 可跨线程共享
// （`Arc<EnvConfig>` 要求 `EnvConfig: Send + Sync`）。
const _: fn() = || {
    fn send<T: Send>() {}
    fn sync<T: Sync>() {}
    send::<env::Env>();
    send::<env::Image>();
    sync::<env::Image>();
};

pub mod bundled {
    include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

    /// 内置 ECL 内容包（按文件名排序）。v1 只有 `"game"`（`godot/ecl/game`）。
    pub fn bundled_sources(pack: &str) -> Option<&'static [(&'static str, &'static str)]> {
        match pack {
            "game" => Some(GAME),
            _ => None,
        }
    }
}
