//! Staged expression evaluation with a temporary self context and the world's RNG.
use super::WorldBody;
use crate::math::{Fx, atan2};
use crate::xform::expr::{XformEvalError, XformExpr, XformRead};
use crate::xform::program::{XformCaptureView, XformEvalContext, XformEvalOwner};

impl WorldBody {
    pub(crate) fn eval_xform_expr(
        &mut self,
        expr: &XformExpr,
        captures: XformCaptureView<'_>,
        owner: XformEvalContext<'_>,
    ) -> Result<i32, XformEvalError> {
        expr.eval(|r| self.read_xform(r, captures, owner))
    }

    fn read_xform(
        &mut self,
        read: XformRead,
        captures: XformCaptureView<'_>,
        ctx: XformEvalContext<'_>,
    ) -> Result<i32, XformEvalError> {
        use XformRead::*;
        match read {
            Capture(k) => return captures.get(k),
            RandRange(n) => {
                return Ok(if n > 0 {
                    self.rand_range(n as u32) as i32
                } else {
                    0
                });
            }
            Global(k) => {
                return self
                    .globals
                    .get(k as usize)
                    .copied()
                    .ok_or(XformEvalError::BadGlobal);
            }
            Frame => return Ok(self.frame as i32),
            PlayerX => return Ok(self.players[0].x.raw()),
            PlayerY => return Ok(self.players[0].y.raw()),
            _ => {}
        }
        if read == AimPlayer {
            let sx = self.read_xform(SelfX, captures, ctx)?;
            let sy = self.read_xform(SelfY, captures, ctx)?;
            let p = self.aim_target(Fx::from_raw(sx), Fx::from_raw(sy));
            return Ok(atan2(
                self.players[p].y - Fx::from_raw(sy),
                self.players[p].x - Fx::from_raw(sx),
            )
            .raw() as i32);
        }
        let (owner, born) = match ctx {
            XformEvalContext::Spawn(init) => {
                return Ok(match read {
                    SelfX => init.x.raw(),
                    SelfY => init.y.raw(),
                    SelfVx => init.vx.raw(),
                    SelfVy => init.vy.raw(),
                    SelfSpeed | SelfAngle => {
                        let (speed, angle) = if init.flags & crate::bullets::BULLET_POLAR_STALE != 0
                        {
                            super::motion::polar_of_vel(init.vx, init.vy, init.angle)
                        } else {
                            (init.speed, init.angle)
                        };
                        if read == SelfSpeed {
                            speed.raw()
                        } else {
                            angle.raw() as i32
                        }
                    }
                    SelfAge => self.frame.wrapping_sub(init.born_frame) as i32,
                    SelfEnemy => -1,
                    _ => 0,
                });
            }
            XformEvalContext::Effect(h) => {
                let i = self.bullets.get(h).ok_or(XformEvalError::InvalidOwner)?;
                (
                    XformEvalOwner::Bullet {
                        handle: h,
                        born_frame: self.bullets.born_frame[i],
                    },
                    self.bullets.born_frame[i],
                )
            }
            XformEvalContext::Caller(o) => {
                let born = match o {
                    XformEvalOwner::Stage { born_frame }
                    | XformEvalOwner::Enemy { born_frame, .. }
                    | XformEvalOwner::Bullet { born_frame, .. } => born_frame,
                };
                (o, born)
            }
        };
        if read == SelfAge {
            return Ok(self.frame.wrapping_sub(born) as i32);
        }
        match owner {
            XformEvalOwner::Stage { .. } => Ok(if read == SelfEnemy { -1 } else { 0 }),
            XformEvalOwner::Enemy { handle, .. } => {
                let i = self
                    .enemies
                    .get(handle)
                    .ok_or(XformEvalError::InvalidOwner)?;
                Ok(match read {
                    SelfX => self.enemies.x[i].raw(),
                    SelfY => self.enemies.y[i].raw(),
                    SelfVx => self.enemies.vx[i].raw(),
                    SelfVy => self.enemies.vy[i].raw(),
                    SelfSpeed => self.enemies.speed[i].raw(),
                    SelfAngle => self.enemies.angle[i].raw() as i32,
                    SelfHp => self.enemies.hp[i],
                    SelfHpMax => self.enemies.hp_max[i],
                    SelfEnemy => crate::enemy::pack_handle(handle),
                    _ => 0,
                })
            }
            XformEvalOwner::Bullet { handle, .. } => {
                let i = self
                    .bullets
                    .get(handle)
                    .ok_or(XformEvalError::InvalidOwner)?;
                Ok(match read {
                    SelfX => self.bullets.x[i].raw(),
                    SelfY => self.bullets.y[i].raw(),
                    SelfVx => self.bullets.vx[i].raw(),
                    SelfVy => self.bullets.vy[i].raw(),
                    SelfSpeed | SelfAngle => {
                        let (speed, angle) = self.bullets.polar(i);
                        if read == SelfSpeed {
                            speed.raw()
                        } else {
                            angle.raw() as i32
                        }
                    }
                    SelfEnemy => -1,
                    _ => 0,
                })
            }
        }
    }
}
