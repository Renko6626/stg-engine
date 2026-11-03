//! Shared programs, phase initialization and compact instance data.
use super::expr::{XformEvalError, XformExpr};
use super::{ARITY, OP_END, OP_LOOP, OP_SET_SPRITE, SLOTS_PER_SEG, XformSlot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XformProgramId(u16);
impl XformProgramId {
    pub fn get(self) -> u16 {
        self.0
    }
}

#[repr(C)]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, crate::checksum::Checksum, crate::save::SaveBytes,
)]
pub struct XformBinding {
    pub program_plus_one: u16,
    pub reserved: [u8; 2],
    pub captures: [i32; 8],
}
#[repr(C)]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, crate::checksum::Checksum, crate::save::SaveBytes,
)]
pub struct XformSegmentState {
    pub program_plus_one: u16,
    pub sampled_slot_plus_one: u8,
    pub reserved: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XformParam {
    Constant(i32),
    Cached(u8),
    Effect(XformExpr),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XformSample {
    pub index: u8,
    pub expr: XformExpr,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XformOpParams {
    pub slot: u8,
    pub args: Vec<XformParam>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XformProgramInit {
    pub slots: Vec<XformSlot>,
    pub argc: u8,
    pub bind: Vec<XformSample>,
    pub spawn: Vec<XformSample>,
    pub params: Vec<XformOpParams>,
    /// Logical capture indices retained after spawn, in compact order.
    pub persistent: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XformProgram {
    pub(crate) init: XformProgramInit,
    pub(crate) logical_count: u8,
    pub(crate) data_start: u8,
    pub(crate) boundaries: u16,
}
impl XformProgram {
    pub fn persistent_count(&self) -> usize {
        self.init.persistent.len()
    }
    pub fn physical_slots(&self) -> usize {
        self.init.slots.len() + self.init.persistent.len().div_ceil(2)
    }
    pub fn logical_count(&self) -> u8 {
        self.logical_count
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XformPrograms {
    programs: Box<[XformProgram]>,
}
pub type XformProgramError = XformEvalError;
impl XformPrograms {
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }
    pub fn len(&self) -> usize {
        self.programs.len()
    }
    pub fn program_id(&self, raw: u16) -> Option<XformProgramId> {
        ((raw as usize) < self.programs.len()).then_some(XformProgramId(raw))
    }
    pub fn get(&self, id: XformProgramId) -> Option<&XformProgram> {
        self.programs.get(id.0 as usize)
    }
    pub(crate) fn get_plus_one(&self, n: u16) -> Option<&XformProgram> {
        n.checked_sub(1).and_then(|i| self.programs.get(i as usize))
    }
    pub fn try_new(inits: Vec<XformProgramInit>) -> Result<Self, XformProgramError> {
        if inits.len() > u16::MAX as usize {
            return Err(XformEvalError::InvalidProgram);
        }
        let mut programs = Vec::with_capacity(inits.len());
        for init in inits {
            let mut available = [false; 8];
            if init.argc > 8 {
                return Err(XformEvalError::InvalidCapture);
            }
            available[..init.argc as usize].fill(true);
            let mut logical_count = init.argc;
            for sample in init.bind.iter().chain(&init.spawn) {
                sample.expr.validate(8)?;
                if sample.index >= 8
                    || available[sample.index as usize]
                    || sample.expr.captures().any(|k| !available[k as usize])
                {
                    return Err(XformEvalError::InvalidCapture);
                }
                available[sample.index as usize] = true;
                logical_count = logical_count.max(sample.index + 1);
            }
            let mut retained = [false; 8];
            for &k in &init.persistent {
                if k >= 8 || !available[k as usize] || retained[k as usize] {
                    return Err(XformEvalError::InvalidCapture);
                }
                retained[k as usize] = true;
            }
            if init.slots.len() + init.persistent.len().div_ceil(2) > SLOTS_PER_SEG {
                return Err(XformEvalError::InvalidProgram);
            }
            let mut boundaries = 0u16;
            let mut k = 0;
            while k < init.slots.len() {
                let op = init.slots[k].op;
                if !super::op_implemented(op) {
                    return Err(XformEvalError::InvalidProgram);
                }
                boundaries |= 1 << k;
                k += 1 + ARITY[op as usize] as usize;
            }
            if k != init.slots.len() {
                return Err(XformEvalError::InvalidProgram);
            }
            // The first data slot / first unused slot is a legal natural END.
            if k < 16 {
                boundaries |= 1 << k;
            }
            for slot in &init.slots {
                if slot.op == OP_LOOP {
                    let target = slot.args[0];
                    if !(0..16).contains(&target) || boundaries & (1 << target) == 0 {
                        return Err(XformEvalError::InvalidProgram);
                    }
                }
            }
            let mut seen = 0u16;
            for params in &init.params {
                let slot = params.slot as usize;
                if slot >= init.slots.len()
                    || boundaries & (1 << slot) == 0
                    || seen & (1 << slot) != 0
                    || init.slots[slot].op == OP_END
                    || params.args.len() > 2
                {
                    return Err(XformEvalError::InvalidProgram);
                }
                seen |= 1 << slot;
                for arg in &params.args {
                    match arg {
                        XformParam::Cached(k) if *k >= 8 || !available[*k as usize] => {
                            return Err(XformEvalError::InvalidCapture);
                        }
                        XformParam::Effect(e) => e.validate(init.persistent.len())?,
                        _ => {}
                    }
                }
                if init.slots[slot].op == OP_SET_SPRITE && params.args.len() != 2 {
                    return Err(XformEvalError::InvalidProgram);
                }
                if init.slots[slot].op == OP_SET_SPRITE
                    && params
                        .args
                        .iter()
                        .any(|p| matches!(p, XformParam::Effect(_)))
                    && params
                        .args
                        .iter()
                        .any(|p| matches!(p, XformParam::Cached(k) if !retained[*k as usize]))
                {
                    return Err(XformEvalError::InvalidCapture);
                }
            }
            programs.push(XformProgram {
                data_start: init.slots.len() as u8,
                init,
                logical_count,
                boundaries,
            });
        }
        Ok(Self {
            programs: programs.into_boxed_slice(),
        })
    }
}

#[derive(Clone, Copy)]
pub enum XformEvalOwner {
    Stage {
        born_frame: u32,
    },
    Enemy {
        handle: crate::enemy::EnemyHandle,
        born_frame: u32,
    },
    Bullet {
        handle: crate::bullets::BulletHandle,
        born_frame: u32,
    },
}
#[derive(Clone, Copy)]
pub enum XformEvalContext<'a> {
    Caller(XformEvalOwner),
    Spawn(&'a crate::bullets::BulletInit),
    Effect(crate::bullets::BulletHandle),
}
#[derive(Clone, Copy)]
pub enum XformCaptureView<'a> {
    Values(&'a [i32]),
    Segment {
        slots: &'a [XformSlot],
        data_start: u8,
        count: u8,
    },
}
impl XformCaptureView<'_> {
    pub(crate) fn get(self, k: u8) -> Result<i32, XformEvalError> {
        match self {
            Self::Values(values) => values.get(k as usize).copied(),
            Self::Segment {
                slots,
                data_start,
                count,
            } => {
                if k < count {
                    slots
                        .get(data_start as usize + k as usize / 2)
                        .map(|s| s.args[k as usize % 2])
                } else {
                    None
                }
            }
        }
        .ok_or(XformEvalError::InvalidCapture)
    }
}
