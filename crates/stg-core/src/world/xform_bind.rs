//! Bind once, materialize spawn values, and sample only on operation entry.
use super::{STATUS_BAD_ARGS, WorldBody};
use crate::bullets::{BulletHandle, BulletInit};
use crate::tables::WorldTables;
use crate::xform::expr::XformEvalError;
use crate::xform::program::*;
use crate::xform::*;

impl WorldBody {
    pub fn bind_xform(
        &self,
        programs: &XformPrograms,
        id: XformProgramId,
        args: &[i32],
        owner: XformEvalOwner,
        tables: &WorldTables,
    ) -> Result<XformBinding, XformEvalError> {
        let program = programs.get(id).ok_or(XformEvalError::InvalidProgram)?;
        if args.len() != program.init.argc as usize {
            return Err(XformEvalError::BadArgs);
        }
        let mut binding = XformBinding {
            program_plus_one: id.get() + 1,
            ..Default::default()
        };
        binding.captures[..args.len()].copy_from_slice(args);
        let mut available = [false; 8];
        available[..args.len()].fill(true);
        for sample in &program.init.bind {
            binding.captures[sample.index as usize] = self.eval_xform_expr(
                &sample.expr,
                XformCaptureView::Values(&binding.captures),
                XformEvalContext::Caller(owner),
            )?;
            available[sample.index as usize] = true;
        }
        // Reject already-known malformed values before replacing a shooter binding.
        for params in &program.init.params {
            let mut values = [None; 2];
            for (k, p) in params.args.iter().enumerate() {
                values[k] = match p {
                    XformParam::Constant(v) => Some(*v),
                    XformParam::Cached(i) if available[*i as usize] => {
                        Some(binding.captures[*i as usize])
                    }
                    _ => None,
                };
            }
            Self::validate_xform_values(
                program.init.slots[params.slot as usize].op,
                values,
                tables,
            )?;
        }
        Ok(binding)
    }

    pub fn create_bullet_with_bound_xform(
        &mut self,
        init: BulletInit,
        binding: &XformBinding,
        programs: &XformPrograms,
        tables: &WorldTables,
    ) -> BulletHandle {
        match self.materialize_bound_xform(&init, binding, programs, tables) {
            Ok((slots, used)) => {
                let h = self.create_bullet_with_xform(init, &slots[..used]);
                if let Some(i) = self.bullets.get(h) {
                    self.xforms.program_states[self.bullets.transform_head[i] as usize] =
                        XformSegmentState {
                            program_plus_one: binding.program_plus_one,
                            ..Default::default()
                        };
                }
                h
            }
            Err(_) => {
                self.xform_bad_args();
                BulletHandle::NULL
            }
        }
    }

    fn materialize_bound_xform(
        &self,
        init: &BulletInit,
        binding: &XformBinding,
        programs: &XformPrograms,
        tables: &WorldTables,
    ) -> Result<([XformSlot; 16], usize), XformEvalError> {
        let program = programs
            .get_plus_one(binding.program_plus_one)
            .ok_or(XformEvalError::InvalidProgram)?;
        let mut values = binding.captures;
        for sample in &program.init.spawn {
            values[sample.index as usize] = self.eval_xform_expr(
                &sample.expr,
                XformCaptureView::Values(&values),
                XformEvalContext::Spawn(init),
            )?;
        }
        let mut slots = [XformSlot::default(); 16];
        slots[..program.init.slots.len()].copy_from_slice(&program.init.slots);
        for params in &program.init.params {
            let slot = &mut slots[params.slot as usize];
            let mut resolved = [None; 2];
            for (k, p) in params.args.iter().enumerate() {
                resolved[k] = match p {
                    XformParam::Constant(v) => Some(*v),
                    XformParam::Cached(i) => Some(values[*i as usize]),
                    XformParam::Effect(_) => None,
                };
            }
            Self::validate_xform_values(slot.op, resolved, tables)?;
            if slot.op == OP_SET_SPRITE {
                if let [Some(shape), Some(color)] = resolved {
                    slot.args[0] = shape.wrapping_add(color);
                }
            } else {
                for (k, value) in resolved.into_iter().enumerate() {
                    if let Some(value) = value {
                        slot.args[k] = value;
                    }
                }
            }
        }
        for (k, &logical) in program.init.persistent.iter().enumerate() {
            slots[program.data_start as usize + k / 2].args[k % 2] = values[logical as usize];
        }
        Ok((slots, program.physical_slots()))
    }

    pub(crate) fn resolve_xform_slot(
        &mut self,
        i: usize,
        programs: &XformPrograms,
        tables: &WorldTables,
    ) -> Result<XformSlot, XformEvalError> {
        let seg = self.bullets.transform_head[i];
        let next = self.bullets.xform_next[i];
        let state = self.xforms.program_states[seg as usize];
        let mut slot = self.xforms.seg_slots(seg)[next as usize];
        if state.program_plus_one == 0 || state.sampled_slot_plus_one == next + 1 {
            return Ok(slot);
        }
        let program = programs
            .get_plus_one(state.program_plus_one)
            .ok_or(XformEvalError::InvalidProgram)?;
        if program.boundaries & (1 << next) == 0 {
            return Err(XformEvalError::InvalidProgram);
        }
        let handle = BulletHandle {
            index: i as u16,
            generation: self.bullets.generation[i],
        };
        if let Some(params) = program.init.params.iter().find(|p| p.slot == next) {
            let captures = XformCaptureView::Segment {
                slots: self.xforms.seg_slots(seg),
                data_start: program.data_start,
                count: program.persistent_count() as u8,
            };
            let mut resolved = [None; 2];
            for (k, p) in params.args.iter().enumerate() {
                resolved[k] = Some(match p {
                    XformParam::Effect(e) => {
                        self.eval_xform_expr(e, captures, XformEvalContext::Effect(handle))?
                    }
                    // Mixed-phase sprite composition still needs its semantic fixed axis.
                    XformParam::Cached(logical) if slot.op == OP_SET_SPRITE => {
                        if params
                            .args
                            .iter()
                            .any(|p| matches!(p, XformParam::Effect(_)))
                        {
                            let compact = program
                                .init
                                .persistent
                                .iter()
                                .position(|v| v == logical)
                                .ok_or(XformEvalError::InvalidCapture)?;
                            captures.get(compact as u8)?
                        } else {
                            continue;
                        }
                    }
                    XformParam::Constant(v) if slot.op == OP_SET_SPRITE => *v,
                    _ => slot.args[k],
                });
            }
            // A fully materialized sprite needs no further semantic reconstruction.
            if slot.op == OP_SET_SPRITE
                && !params
                    .args
                    .iter()
                    .any(|p| matches!(p, XformParam::Effect(_)))
            {
                self.xforms.program_states[seg as usize].sampled_slot_plus_one = next + 1;
                return Ok(slot);
            }
            Self::validate_xform_values(slot.op, resolved, tables)?;
            if slot.op == OP_SET_SPRITE {
                slot.args[0] = resolved[0]
                    .ok_or(XformEvalError::BadArgs)?
                    .wrapping_add(resolved[1].ok_or(XformEvalError::BadArgs)?);
            } else {
                for (k, value) in resolved.into_iter().enumerate() {
                    // LOOP's count is mutable execution state after creation.
                    if !(slot.op == OP_LOOP && k == 1)
                        && let Some(value) = value
                    {
                        slot.args[k] = value;
                    }
                }
            }
        }
        if slot.op == OP_LOOP
            && (!(0..16).contains(&slot.args[0]) || program.boundaries & (1 << slot.args[0]) == 0)
        {
            return Err(XformEvalError::BadArgs);
        }
        self.xforms.seg_slots_mut(seg)[next as usize] = slot;
        self.xforms.program_states[seg as usize].sampled_slot_plus_one = next + 1;
        Ok(slot)
    }

    fn validate_xform_values(
        op: u8,
        values: [Option<i32>; 2],
        tables: &WorldTables,
    ) -> Result<(), XformEvalError> {
        let stride = tables.color_stride as i32;
        let shape_ok = |v: i32| {
            stride > 0
                && v >= 0
                && v % stride == 0
                && (v as usize) < tables.appearances.len() / stride as usize * stride as usize
        };
        let color_ok = |v: i32| (0..stride).contains(&v);
        let bad = match op {
            OP_SET_SPRITE => {
                values[0].is_some_and(|v| !shape_ok(v))
                    || values[1].is_some_and(|v| !color_ok(v))
                    || matches!(values, [Some(shape), Some(color)] if !tables.appearances[(shape + color) as usize].valid)
            }
            OP_SET_SHAPE => values[0].is_some_and(|v| !shape_ok(v)),
            OP_SET_COLOR => values[0].is_some_and(|v| !color_ok(v)),
            OP_WAIT_SIGNAL => {
                values[0].is_some_and(|v| !(0..super::SIGNAL_CHANNELS as i32).contains(&v))
            }
            op if is_step(op) => {
                values[1].is_some_and(|v| crate::math::Easing::from_id((v >> 16) as u8).is_none())
            }
            _ => false,
        };
        if bad {
            Err(XformEvalError::BadArgs)
        } else {
            Ok(())
        }
    }

    pub(crate) fn xform_bad_args(&mut self) {
        self.diag.contract_viol = self.diag.contract_viol.wrapping_add(1);
        self.last_status = STATUS_BAD_ARGS;
    }
}

#[cfg(test)]
#[path = "xform_bind_tests.rs"]
mod tests;
