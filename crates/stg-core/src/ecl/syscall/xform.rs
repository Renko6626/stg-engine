//! Task-to-core adaptation. Argument slices never escape this call.
use super::*;
use crate::xform::program::{XformBinding, XformEvalOwner};

pub(super) fn bind_program(
    task: &Task,
    ctx: &mut VmCtx,
    raw: i32,
    off: i32,
    argc: i32,
) -> Result<Option<XformBinding>, u8> {
    let raw = u16::try_from(raw).map_err(|_| FAULT_BAD_OP)?;
    let id = ctx.ecl.xforms().program_id(raw).ok_or(FAULT_BAD_OP)?;
    if !(0..=8).contains(&argc) || off < 0 || off as usize + argc as usize > LOCALS {
        return Err(FAULT_BAD_OP);
    }
    let owner = match task.owner_kind {
        OWNER_ENEMY => XformEvalOwner::Enemy {
            handle: EnemyHandle {
                index: task.owner_index,
                generation: task.owner_gen,
            },
            born_frame: task.born_frame,
        },
        OWNER_BULLET => XformEvalOwner::Bullet {
            handle: BulletHandle {
                index: task.owner_index,
                generation: task.owner_gen,
            },
            born_frame: task.born_frame,
        },
        _ => XformEvalOwner::Stage {
            born_frame: task.born_frame,
        },
    };
    match ctx.body.bind_xform(
        ctx.ecl.xforms(),
        id,
        &task.locals[off as usize..off as usize + argc as usize],
        owner,
        ctx.tables,
    ) {
        Ok(binding) => Ok(Some(binding)),
        Err(_) => {
            ctx.body.xform_bad_args();
            Ok(None)
        }
    }
}

pub(super) fn sys_sh_xform_program(task: &mut Task, ctx: &mut VmCtx) -> Result<(), u8> {
    let argc = pop(task)?;
    let off = pop(task)?;
    let program = pop(task)?;
    let id = pop(task)?;
    if !(0..SHOOTERS_PER_TASK as i32).contains(&id) {
        ctx.body.xform_bad_args();
        return Ok(());
    }
    if let Some(binding) = bind_program(task, ctx, program, off, argc)? {
        let sh = &mut ctx.tasks.shooters[ctx.self_index as usize][id as usize];
        sh.binding = binding;
        sh.xform_off = 0;
        sh.xform_cnt = 0;
    }
    Ok(())
}
