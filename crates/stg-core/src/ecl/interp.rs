//! Task-private explicit-value interpolation. All values are Q16.16 raw words.
//! Starts happen after the frame tick; reads never advance state.

pub const INTERP_SLOTS_PER_KIND: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum, crate::save::SaveBytes)]
pub(crate) struct Accum {
    pub value: i32,
    pub delta: i32,
    pub remaining: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum, crate::save::SaveBytes)]
pub(crate) struct Accel {
    pub value: i32,
    pub velocity: i32,
    pub acceleration: i32,
    pub remaining: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum, crate::save::SaveBytes)]
pub(crate) struct Hermite {
    pub value: i32,
    pub start: i32,
    pub end: i32,
    pub tangent_start: i32,
    pub tangent_end: i32,
    pub elapsed: u16,
    pub duration: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default, crate::checksum::Checksum, crate::save::SaveBytes)]
pub(crate) struct InterpSlots {
    pub accum: [Accum; INTERP_SLOTS_PER_KIND],
    pub accel: [Accel; INTERP_SLOTS_PER_KIND],
    pub hermite: [Hermite; INTERP_SLOTS_PER_KIND],
}

impl InterpSlots {
    pub fn tick(&mut self) {
        for s in &mut self.accum {
            if s.remaining > 0 {
                s.value = s.value.wrapping_add(s.delta);
                s.remaining -= 1;
            }
        }
        for s in &mut self.accel {
            if s.remaining > 0 {
                s.value = s.value.wrapping_add(s.velocity);
                s.velocity = s.velocity.wrapping_add(s.acceleration);
                s.remaining -= 1;
            }
        }
        for s in &mut self.hermite {
            if s.elapsed >= s.duration {
                continue;
            }
            s.elapsed += 1;
            if s.elapsed == s.duration {
                s.value = s.end;
                continue;
            }
            let t = ((s.elapsed as i64) << 16) / s.duration as i64;
            let t2 = (t * t) >> 16;
            let t3 = (t2 * t) >> 16;
            let h00 = 2 * t3 - 3 * t2 + 65536;
            let h01 = -2 * t3 + 3 * t2;
            let h10 = t3 - 2 * t2 + t;
            let h11 = t3 - t2;
            s.value = ((s.start as i64 * h00
                + s.end as i64 * h01
                + s.tangent_start as i64 * h10
                + s.tangent_end as i64 * h11)
                >> 16) as i32;
        }
    }
}
