//! Opt-in host-side timings. These never enter World, observations or RNG state.
//! Worker durations are sums of elapsed intervals, not batch wall time.

use std::time::{Duration, Instant};

#[inline]
pub(crate) fn start<const ENABLED: bool>() -> Option<Instant> {
    if ENABLED { Some(Instant::now()) } else { None }
}

#[inline]
pub(crate) fn elapsed(start: Option<Instant>) -> Duration {
    start.map_or(Duration::ZERO, |t| t.elapsed())
}

macro_rules! timings {
    ($(#[$meta:meta])* $name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default)]
        pub struct $name { $(pub $field: $ty,)* }

        impl std::ops::AddAssign for $name {
            fn add_assign(&mut self, rhs: Self) {
                $(self.$field += rhs.$field;)*
            }
        }
    };
}

timings!(EnvProfile {
    total: Duration,
    core_step: Duration,
    reset: Duration,
    core_frames: u64,
    resets: u64,
});

timings!(BulletProfile {
    gather: Duration,
    nearest: Duration,
    reorder: Duration,
    rows: Duration,
});

timings!(ObservationProfile {
    small: Duration,
    lasers: Duration,
    items: Duration,
    bullets: BulletProfile,
});

timings!(WorkProfile {
    env: EnvProfile,
    observation: ObservationProfile,
});

timings!(
    /// Disjoint batch wall intervals plus separately accumulated worker intervals.
    BatchProfile {
    total: Duration,
    prepare: Duration,
    parallel_step_observe: Duration,
    prefix: Duration,
    split: Duration,
    scatter: Duration,
    aggregate: Duration,
    workers: WorkProfile,
});
