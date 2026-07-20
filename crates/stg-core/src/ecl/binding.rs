//! Safe named-entry binding and singleton main (Task 3).
//!
//! Public types for typed argument passing, owner specification, and startup
//! error reporting. The three startup APIs on `World` (`start_main`,
//! `spawn_entry`, `spawn_entry_named`) replace the raw
//! `#[doc(hidden)] pub fn spawn_task`.

use crate::bullets::BulletHandle;
use crate::ecl::image::{EclImage, EclValueType, ResolvedEntry, ResolveError, SubId};
use crate::ecl::task::{OWNER_BULLET, OWNER_ENEMY, OWNER_STAGE};
use crate::enemy::EnemyHandle;
use crate::math::{Angle, Fx};
use crate::world::{WorldBody, POOL_TASK, STATUS_BAD_ARGS, STATUS_POOL_FULL};
use crate::step::World;

// ── Public types ────────────────────────────────────────────────────────────

/// A typed ECL argument value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EclArg {
    Int(i32),
    Fx(Fx),
    Angle(Angle),
}

impl EclArg {
    /// The declared parameter type this value corresponds to.
    pub fn value_type(&self) -> EclValueType {
        match self {
            EclArg::Int(_) => EclValueType::Int,
            EclArg::Fx(_) => EclValueType::Fx,
            EclArg::Angle(_) => EclValueType::Angle,
        }
    }

    /// The raw `i32` representation (Fx raw, Angle raw, or bare int).
    pub fn raw(&self) -> i32 {
        match self {
            EclArg::Int(v) => *v,
            EclArg::Fx(v) => v.raw(),
            EclArg::Angle(v) => v.raw() as i32,
        }
    }
}

/// Specifies which entity "owns" a newly spawned ECL task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EclOwner {
    /// The stage / level itself (always valid).
    Stage,
    /// An enemy entity. Validated against the enemy pool at spawn time.
    Enemy(EnemyHandle),
    /// A bullet entity. Validated against the bullet pool at spawn time.
    Bullet(BulletHandle),
}

impl EclOwner {
    /// Returns `true` if the owner is still alive in the given world body.
    pub fn validate(&self, body: &WorldBody) -> bool {
        match self {
            EclOwner::Stage => true,
            EclOwner::Enemy(h) => body.enemies.get(*h).is_some(),
            EclOwner::Bullet(h) => body.bullets.get(*h).is_some(),
        }
    }
}

/// Error returned by the safe entry-point APIs on `World`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskStartError {
    /// The image has no root sub; main cannot start.
    NoRoot,
    /// `ecl_main_started` is already set for this `World`.
    MainAlreadyStarted,
    /// The named entry was not found in the image.
    UnknownEntry,
    /// `"main"` was passed to a name-based API; use `start_main` instead.
    RootRequiresStartMain,
    /// The resolved entry's `SubId` is invalid (test-only path).
    InvalidEntryId,
    /// The number of raw arguments does not match the sub's parameter count.
    WrongArgCount {
        expected: u8,
        actual: usize,
    },
    /// The type of a typed argument does not match the sub's parameter declaration.
    WrongArgType {
        index: u8,
        expected: EclValueType,
        actual: EclValueType,
    },
    /// The owner handle does not point to a live entity.
    InvalidOwner,
    /// The task pool is full.
    PoolFull,
}

// ── World extension methods ────────────────────────────────────────────────

impl World {
    /// Low-level spawn by raw `SubId`.
    ///
    /// This is an advanced API for cases where the safe entry-point APIs
    /// (`start_main`, `spawn_entry`, `spawn_entry_named`) cannot express the
    /// required owner kind or sub type (e.g. spawning a root sub with an
    /// enemy owner, as done by the rainbow-scene harness).
    ///
    /// Validates the script exists in the image, is not `CallOnly`, and that
    /// the arg count matches the sub's parameter list.  Increments diagnostics
    /// on failure.  Returns `None` on error (bad args or pool full).
    pub fn spawn_sub_id(
        &mut self,
        ecl: &EclImage,
        script: SubId,
        args: &[i32],
        owner: (u8, u16, u16),
    ) -> Option<u16> {
        self.spawn_sub_internal(ecl, script, args, owner)
    }

    /// Start the root (main) script.
    /// Start the root (main) script.
    ///
    /// Uses stage owner and zero arguments.  Succeeds at most once per `World`
    /// lifetime — even after the main task ends or faults, a second call
    /// returns `MainAlreadyStarted`.
    pub fn start_main(&mut self, image: &EclImage) -> Result<u16, TaskStartError> {
        if self.ecl_main_started != 0 {
            self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
            self.body.last_status = STATUS_BAD_ARGS;
            return Err(TaskStartError::MainAlreadyStarted);
        }
        let root = image.root().ok_or(TaskStartError::NoRoot)?;
        let meta = image
            .sub_meta(root)
            .expect("root validated at image construction");
        let frame = self.body.frame;
        match self.tasks.spawn(root, meta.code_entry(), (OWNER_STAGE, 0, 0), 0, frame) {
            Some(idx) => {
                self.ecl_main_started = 1;
                Ok(idx)
            }
            None => {
                // Pool full: main_started stays 0 so the caller can free and retry.
                self.body.diag.pool_full[POOL_TASK] =
                    self.body.diag.pool_full[POOL_TASK].wrapping_add(1);
                self.body.last_status = STATUS_POOL_FULL;
                Err(TaskStartError::PoolFull)
            }
        }
    }

    /// Spawn an already-resolved entry with raw `i32` arguments (fast path).
    ///
    /// Validates the argument count matches the sub's parameter declaration and
    /// that the owner is alive.  Arguments are written into the child task's
    /// locals in declaration order.
    pub fn spawn_entry(
        &mut self,
        entry: ResolvedEntry<'_>,
        args: &[i32],
        owner: EclOwner,
    ) -> Result<u16, TaskStartError> {
        let sub = entry.sub();
        self.spawn_resolved_sub(sub, entry.meta(), args, owner)
    }

    /// Resolve an entry by name and spawn it with typed `EclArg` values.
    ///
    /// Each argument's type is validated against the sub's parameter
    /// declarations before spawning.  Returns `RootRequiresStartMain` for
    /// `"main"`, and `UnknownEntry` for unresolvable names.
    pub fn spawn_entry_named(
        &mut self,
        image: &EclImage,
        name: &str,
        args: &[EclArg],
        owner: EclOwner,
    ) -> Result<u16, TaskStartError> {
        let entry = match image.resolve_entry(name) {
            Ok(e) => e,
            Err(ResolveError::RootRequiresStartMain) => {
                self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
                self.body.last_status = STATUS_BAD_ARGS;
                return Err(TaskStartError::RootRequiresStartMain);
            }
            Err(ResolveError::UnknownEntry) => {
                self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
                self.body.last_status = STATUS_BAD_ARGS;
                return Err(TaskStartError::UnknownEntry);
            }
        };
        let sub = entry.sub();
        let meta = entry.meta();
        let params = image
            .param_types(sub)
            .expect("entry sub validated at image construction");

        // Validate arg count and type of each argument.
        if args.len() != params.len() {
            self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
            self.body.last_status = STATUS_BAD_ARGS;
            return Err(TaskStartError::WrongArgCount {
                expected: params.len() as u8,
                actual: args.len(),
            });
        }
        for (i, (arg, &param_type)) in args.iter().zip(params.iter()).enumerate() {
            let arg_type = arg.value_type();
            if arg_type != param_type {
                self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
                self.body.last_status = STATUS_BAD_ARGS;
                return Err(TaskStartError::WrongArgType {
                    index: i as u8,
                    expected: param_type,
                    actual: arg_type,
                });
            }
        }

        let raw_args: Vec<i32> = args.iter().map(|a| a.raw()).collect();
        self.spawn_resolved_sub(sub, meta, &raw_args, owner)
    }

    /// Shared validation + spawn for a resolved sub.
    ///
    /// Validates owner liveness, parameter count, and task-pool capacity.
    /// Increments diagnostics on failure.
    fn spawn_resolved_sub(
        &mut self,
        sub: SubId,
        meta: &crate::ecl::image::RuntimeSubMeta,
        args: &[i32],
        owner: EclOwner,
    ) -> Result<u16, TaskStartError> {
        // Validate owner.
        if !owner.validate(&self.body) {
            self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
            self.body.last_status = STATUS_BAD_ARGS;
            return Err(TaskStartError::InvalidOwner);
        }

        // Validate argument count.
        let expected = meta.param_count();
        if args.len() != expected as usize {
            self.body.diag.contract_viol = self.body.diag.contract_viol.wrapping_add(1);
            self.body.last_status = STATUS_BAD_ARGS;
            return Err(TaskStartError::WrongArgCount {
                expected,
                actual: args.len(),
            });
        }

        // Map owner to (kind, index, gen).
        let owner_tuple = match owner {
            EclOwner::Stage => (OWNER_STAGE, 0u16, 0u16),
            EclOwner::Enemy(h) => (OWNER_ENEMY, h.index, h.generation),
            EclOwner::Bullet(h) => (OWNER_BULLET, h.index, h.generation),
        };

        let frame = self.body.frame;
        match self
            .tasks
            .spawn(sub, meta.code_entry(), owner_tuple, 0, frame)
        {
            Some(idx) => {
                self.tasks.slots[idx as usize].locals[..args.len()].copy_from_slice(args);
                Ok(idx)
            }
            None => {
                self.body.diag.pool_full[POOL_TASK] =
                    self.body.diag.pool_full[POOL_TASK].wrapping_add(1);
                self.body.last_status = STATUS_POOL_FULL;
                Err(TaskStartError::PoolFull)
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecl::image::{EntryInit, SubInit, SubKind, test_image, ResolveError, EclValueType};
    use crate::ecl::ops::*;
    use crate::ecl::task::OWNER_STAGE;
    use crate::input::InputFrame;
    use crate::step::{World, step};
    use crate::tables::TABLES_V0;
    use crate::world::{POOL_TASK, STATUS_BAD_ARGS, STATUS_POOL_FULL};
    use crate::math::Fx;

    /// Build an image with a root (sub 0) and one async entry (sub 1) named "worker".
    fn root_and_async_image() -> EclImage {
        test_image(
            vec![OP_END as u32],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![EclValueType::Fx, EclValueType::Angle]),
            ],
            vec![EntryInit::new("worker", 1)],
            Some(0),
        )
    }

    fn root_and_zero_arg_async_image() -> EclImage {
        test_image(
            vec![OP_END as u32],
            vec![
                SubInit::new(0, SubKind::Root, vec![]),
                SubInit::new(0, SubKind::Async, vec![]),
            ],
            vec![EntryInit::new("worker", 1)],
            Some(0),
        )
    }

    fn root_only_image(first_word: u32) -> EclImage {
        let code = if first_word == OP_END as u32 {
            vec![OP_END as u32]
        } else {
            vec![first_word]
        };
        test_image(
            code,
            vec![SubInit::new(0, SubKind::Root, vec![])],
            vec![],
            Some(0),
        )
    }

    /// Fill the task pool completely using a minimal image's spawn.
    fn fill_task_pool(world: &mut World) {
        let img = test_image(
            vec![OP_END as u32],
            vec![SubInit::new(0, SubKind::Root, vec![])],
            vec![],
            Some(0),
        );
        let root = img.root().unwrap();
        let meta = img.sub_meta(root).unwrap();
        while world
            .tasks
            .spawn(root, meta.code_entry(), (OWNER_STAGE, 0, 0), 0, 0)
            .is_some()
        {}
    }

    // ── Step 1 tests from the brief ─────────────────────────────────────

    #[test]
    fn start_main_is_stage_owned_and_once_per_world_lifetime() {
        let image = root_and_async_image();
        let mut world = World::new(1);
        let task = world.start_main(&image).unwrap();
        let spawned = &world.tasks.slots[task as usize];
        assert_eq!(
            (spawned.owner_kind, spawned.owner_index, spawned.owner_gen),
            (OWNER_STAGE, 0, 0),
        );
        assert_eq!(
            world.start_main(&image),
            Err(TaskStartError::MainAlreadyStarted)
        );
        assert_eq!(world.body.diag.contract_viol, 1);
        assert_eq!(world.body.last_status, STATUS_BAD_ARGS);
    }

    #[test]
    fn main_started_roundtrips_through_snapshot_and_checksum() {
        let image = root_and_async_image();
        let mut source = World::new(1);
        source.start_main(&image).unwrap();
        let mut copy = World::new(2);
        source.copy_into(&mut copy);
        assert_eq!(source.checksum(), copy.checksum());
        assert_eq!(
            copy.start_main(&image),
            Err(TaskStartError::MainAlreadyStarted)
        );
    }

    #[test]
    fn named_entry_checks_types_and_fast_entry_checks_only_arity() {
        let image = root_and_async_image();
        let entry = image.resolve_entry("worker").unwrap();
        let mut world = World::new(1);

        // spawn_entry_named with wrong type for index 0 (Int vs Fx).
        assert!(matches!(
            world.spawn_entry_named(
                &image,
                "worker",
                &[EclArg::Int(1), EclArg::Angle(Angle::ZERO)],
                EclOwner::Stage,
            ),
            Err(TaskStartError::WrongArgType { index: 0, .. })
        ));

        // spawn_entry with raw i32 args uses only arity check.
        assert!(world
            .spawn_entry(entry, &[Fx::ONE.raw(), Angle::ZERO.raw() as i32], EclOwner::Stage)
            .is_ok());
    }

    #[test]
    fn main_cannot_restart_after_end_or_fault() {
        for first_word in [OP_END as u32, u8::MAX as u32] {
            let image = root_only_image(first_word);
            let mut world = World::new(1);
            world.start_main(&image).unwrap();
            for _ in 0..2 {
                step(&mut world, &TABLES_V0, &image, &InputFrame::empty(0));
            }
            assert_eq!(world.tasks.iter_alive().count(), 0);
            assert_eq!(
                world.start_main(&image),
                Err(TaskStartError::MainAlreadyStarted)
            );
        }
    }

    // ── Additional test cases ────────────────────────────────────────────

    #[test]
    fn start_main_resolves_main_name_via_error() {
        let image = root_and_async_image();
        assert_eq!(
            image.resolve_entry("main"),
            Err(ResolveError::RootRequiresStartMain)
        );
    }

    #[test]
    fn spawn_entry_named_unknown_name() {
        let image = root_and_async_image();
        let mut world = World::new(1);
        assert_eq!(
            world.spawn_entry_named(&image, "nonexistent", &[], EclOwner::Stage),
            Err(TaskStartError::UnknownEntry)
        );
    }

    #[test]
    fn spawn_entry_named_main_returns_root_requires_start_main() {
        let image = root_and_async_image();
        let mut world = World::new(1);
        assert_eq!(
            world.spawn_entry_named(&image, "main", &[], EclOwner::Stage),
            Err(TaskStartError::RootRequiresStartMain)
        );
    }

    #[test]
    fn spawn_entry_wrong_arg_count() {
        let image = root_and_async_image();
        let entry = image.resolve_entry("worker").unwrap();
        let mut world = World::new(1);

        let alive_before = world.tasks.iter_alive().count();
        let cv_before = world.body.diag.contract_viol;
        assert_eq!(
            world.spawn_entry(entry, &[Fx::ONE.raw()], EclOwner::Stage),
            Err(TaskStartError::WrongArgCount {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(world.tasks.iter_alive().count(), alive_before);
        assert_eq!(world.body.diag.contract_viol, cv_before + 1);
        assert_eq!(world.body.last_status, STATUS_BAD_ARGS);
    }

    #[test]
    fn spawn_entry_named_wrong_arg_count() {
        let image = root_and_async_image();
        let mut world = World::new(1);
        let cv_before = world.body.diag.contract_viol;
        assert_eq!(
            world.spawn_entry_named(&image, "worker", &[EclArg::Fx(Fx::ONE)], EclOwner::Stage),
            Err(TaskStartError::WrongArgCount {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(world.body.diag.contract_viol, cv_before + 1);
        assert_eq!(world.body.last_status, STATUS_BAD_ARGS);
    }

    #[test]
    fn invalid_entry_id_variant_is_present() {
        let err = TaskStartError::InvalidEntryId;
        assert!(matches!(err, TaskStartError::InvalidEntryId));
    }

    #[test]
    fn spawn_entry_invalid_owner_enemy() {
        let image = root_and_async_image();
        let entry = image.resolve_entry("worker").unwrap();
        let mut world = World::new(1);
        let stale = EnemyHandle {
            index: 0,
            generation: 999,
        };
        let alive_before = world.tasks.iter_alive().count();
        let cv_before = world.body.diag.contract_viol;
        assert_eq!(
            world.spawn_entry(
                entry,
                &[Fx::ONE.raw(), Angle::ZERO.raw() as i32],
                EclOwner::Enemy(stale),
            ),
            Err(TaskStartError::InvalidOwner)
        );
        assert_eq!(world.tasks.iter_alive().count(), alive_before);
        assert_eq!(world.body.diag.contract_viol, cv_before + 1);
        assert_eq!(world.body.last_status, STATUS_BAD_ARGS);
    }

    #[test]
    fn spawn_entry_invalid_owner_bullet() {
        let image = root_and_async_image();
        let entry = image.resolve_entry("worker").unwrap();
        let mut world = World::new(1);
        let stale = BulletHandle {
            index: 0,
            generation: 999,
        };
        let cv_before = world.body.diag.contract_viol;
        assert_eq!(
            world.spawn_entry(
                entry,
                &[Fx::ONE.raw(), Angle::ZERO.raw() as i32],
                EclOwner::Bullet(stale),
            ),
            Err(TaskStartError::InvalidOwner)
        );
        assert_eq!(world.body.diag.contract_viol, cv_before + 1);
        assert_eq!(world.body.last_status, STATUS_BAD_ARGS);
    }

    #[test]
    fn spawn_entry_pool_full() {
        let image = root_and_zero_arg_async_image();
        let entry = image.resolve_entry("worker").unwrap();
        let mut world = World::new(1);
        fill_task_pool(&mut world);

        let pf_before = world.body.diag.pool_full[POOL_TASK];
        assert_eq!(
            world.spawn_entry(entry, &[], EclOwner::Stage),
            Err(TaskStartError::PoolFull)
        );
        assert_eq!(world.body.diag.pool_full[POOL_TASK], pf_before + 1);
        assert_eq!(world.body.last_status, STATUS_POOL_FULL);
    }

    #[test]
    fn start_main_pool_full_does_not_set_flag() {
        let image = root_and_async_image();
        let mut world = World::new(1);
        fill_task_pool(&mut world);

        let pf_before = world.body.diag.pool_full[POOL_TASK];
        assert_eq!(world.start_main(&image), Err(TaskStartError::PoolFull));
        assert_eq!(world.body.diag.pool_full[POOL_TASK], pf_before + 1);
        assert_eq!(world.body.last_status, STATUS_POOL_FULL);

        // Kill one task and retry — should succeed and set the flag.
        let victim = world
            .tasks
            .iter_alive()
            .next()
            .expect("there is at least one task");
        world.tasks.kill(victim);

        let result = world.start_main(&image);
        assert!(
            result.is_ok(),
            "after freeing one, start_main should succeed: {:?}",
            result
        );
    }

    #[test]
    fn start_main_no_root() {
        let mut world = World::new(1);
        assert_eq!(
            world.start_main(&EclImage::empty()),
            Err(TaskStartError::NoRoot)
        );
    }

    #[test]
    fn spawn_entry_named_valid_args() {
        let image = root_and_async_image();
        let mut world = World::new(1);
        let task = world
            .spawn_entry_named(
                &image,
                "worker",
                &[EclArg::Fx(Fx::ONE), EclArg::Angle(Angle::ZERO)],
                EclOwner::Stage,
            )
            .unwrap();
        assert!(world.tasks.is_alive(task as usize));
        let t = &world.tasks.slots[task as usize];
        assert_eq!(t.owner_kind, OWNER_STAGE);
        assert_eq!(t.locals[0], Fx::ONE.raw());
        assert_eq!(t.locals[1], Angle::ZERO.raw() as i32);
    }

}
