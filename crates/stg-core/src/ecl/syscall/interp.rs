use super::*;

/// Decode typed interpolation calls; all validation precedes state mutation.
pub(super) fn sys_interp(task: &mut Task, ctx: &mut VmCtx<'_>, no: u16) -> Result<(), u8> {
    use crate::ecl::interp::{Accel, Accum, Hermite, INTERP_SLOTS_PER_KIND};
    let count = match no {
        SYS_ACCUM_START => 4,
        SYS_ACCEL_START => 5,
        SYS_HERMITE_START => 6,
        _ => 1,
    };
    let mut args = [0i32; 6];
    for i in (0..count).rev() {
        args[i] = pop(task)?;
    }
    let slot = args[0];
    let reading = count == 1;
    if !(0..INTERP_SLOTS_PER_KIND as i32).contains(&slot)
        || (!reading && !(0..=u16::MAX as i32).contains(&args[count - 1]))
    {
        ctx.body.diag.contract_viol = ctx.body.diag.contract_viol.wrapping_add(1);
        ctx.body.last_status = crate::world::STATUS_BAD_ARGS;
        return if reading { push(task, 0) } else { Ok(()) };
    }
    let bank = &mut ctx.tasks.interps[ctx.self_index as usize];
    let slot = slot as usize;
    match no {
        SYS_ACCUM_START => {
            bank.accum[slot] = Accum {
                value: args[1],
                delta: args[2],
                remaining: args[3] as u16,
            }
        }
        SYS_ACCEL_START => {
            bank.accel[slot] = Accel {
                value: args[1],
                velocity: args[2],
                acceleration: args[3],
                remaining: args[4] as u16,
            }
        }
        SYS_HERMITE_START => {
            bank.hermite[slot] = Hermite {
                value: if args[5] == 0 { args[2] } else { args[1] },
                start: args[1],
                end: args[2],
                tangent_start: args[3],
                tangent_end: args[4],
                elapsed: 0,
                duration: args[5] as u16,
            }
        }
        SYS_ACCUM_VALUE => return push(task, bank.accum[slot].value),
        SYS_ACCEL_VALUE => return push(task, bank.accel[slot].value),
        SYS_HERMITE_VALUE => return push(task, bank.hermite[slot].value),
        _ => unreachable!(),
    }
    Ok(())
}
