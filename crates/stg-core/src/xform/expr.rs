//! Bounded, pure expression bytecode shared by xform instances.

pub const MAX_EXPR_INSTRUCTIONS: usize = 128;
pub const EXPR_STACK: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XformRead {
    Frame,
    PlayerX,
    PlayerY,
    SelfX,
    SelfY,
    SelfVx,
    SelfVy,
    SelfSpeed,
    SelfAngle,
    SelfHp,
    SelfHpMax,
    SelfAge,
    SelfEnemy,
    AimPlayer,
    Capture(u8),
    Global(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XformUnary {
    Neg,
    Not,
    Bool,
    IntToFx,
    FxToInt,
    Sin,
    Cos,
    Global,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XformBinary {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    MulFx,
    DivFx,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Atan2,
    Dist,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XformInstr {
    Constant(i32),
    Read(XformRead),
    Unary(XformUnary),
    Binary(XformBinary),
    JumpIfZero(u8),
    Jump(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XformExpr {
    Constant(i32),
    Read(XformRead),
    Code(Box<[XformInstr]>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XformEvalError {
    InvalidProgram,
    InvalidCapture,
    InvalidOwner,
    DivisionByZero,
    BadGlobal,
    BadArgs,
}

impl XformExpr {
    pub fn try_code(code: Vec<XformInstr>) -> Result<Self, XformEvalError> {
        let result = Self::Code(code.into_boxed_slice());
        result.validate(8)?;
        Ok(result)
    }

    pub(crate) fn validate(&self, captures: usize) -> Result<(), XformEvalError> {
        let read_ok = |r: XformRead| !matches!(r, XformRead::Capture(k) if k as usize >= captures);
        match self {
            Self::Constant(_) => Ok(()),
            Self::Read(r) => {
                if read_ok(*r) {
                    Ok(())
                } else {
                    Err(XformEvalError::InvalidCapture)
                }
            }
            Self::Code(code) => {
                if code.is_empty() || code.len() > MAX_EXPR_INSTRUCTIONS {
                    return Err(XformEvalError::InvalidProgram);
                }
                let mut depths = [None; MAX_EXPR_INSTRUCTIONS + 1];
                depths[0] = Some(0usize);
                for (pc, ins) in code.iter().enumerate() {
                    if matches!(ins, XformInstr::Read(r) if !read_ok(*r)) {
                        return Err(XformEvalError::InvalidCapture);
                    }
                    if matches!(ins, XformInstr::Jump(t) | XformInstr::JumpIfZero(t) if *t as usize <= pc || *t as usize > code.len())
                    {
                        return Err(XformEvalError::InvalidProgram);
                    }
                    let Some(depth) = depths[pc] else { continue };
                    let (take, put) = match ins {
                        XformInstr::Constant(_) | XformInstr::Read(_) => (0, 1),
                        XformInstr::Unary(_) => (1, 1),
                        XformInstr::Binary(_) => (2, 1),
                        XformInstr::JumpIfZero(_) => (1, 0),
                        XformInstr::Jump(_) => (0, 0),
                    };
                    if depth < take || depth - take + put > EXPR_STACK {
                        return Err(XformEvalError::InvalidProgram);
                    }
                    let next_depth = depth - take + put;
                    let mut merge = |target: usize| -> Result<(), XformEvalError> {
                        if depths[target].is_some_and(|old| old != next_depth) {
                            return Err(XformEvalError::InvalidProgram);
                        }
                        depths[target] = Some(next_depth);
                        Ok(())
                    };
                    match ins {
                        XformInstr::Jump(t) => merge(*t as usize)?,
                        XformInstr::JumpIfZero(t) => {
                            merge(*t as usize)?;
                            merge(pc + 1)?;
                        }
                        _ => merge(pc + 1)?,
                    }
                }
                if depths[code.len()] == Some(1) {
                    Ok(())
                } else {
                    Err(XformEvalError::InvalidProgram)
                }
            }
        }
    }

    pub fn remap_captures(&mut self, mut map: impl FnMut(u8) -> u8) {
        let mut remap = |r: &mut XformRead| {
            if let XformRead::Capture(k) = r {
                *k = map(*k);
            }
        };
        match self {
            Self::Read(r) => remap(r),
            Self::Code(code) => {
                for ins in code {
                    if let XformInstr::Read(r) = ins {
                        remap(r);
                    }
                }
            }
            Self::Constant(_) => {}
        }
    }

    pub fn captures(&self) -> impl Iterator<Item = u8> + '_ {
        let mut found = [false; 256];
        let mut visit = |r: XformRead| {
            if let XformRead::Capture(k) = r {
                found[k as usize] = true;
            }
        };
        match self {
            Self::Read(r) => visit(*r),
            Self::Code(code) => {
                for ins in code {
                    if let XformInstr::Read(r) = ins {
                        visit(*r);
                    }
                }
            }
            _ => {}
        }
        found
            .into_iter()
            .enumerate()
            .filter_map(|(k, used)| used.then_some(k as u8))
    }

    pub(crate) fn eval(
        &self,
        mut read: impl FnMut(XformRead) -> Result<i32, XformEvalError>,
    ) -> Result<i32, XformEvalError> {
        match self {
            Self::Constant(v) => return Ok(*v),
            Self::Read(r) => return read(*r),
            _ => {}
        }
        let Self::Code(code) = self else {
            unreachable!()
        };
        let mut stack = [0i32; EXPR_STACK];
        let (mut sp, mut pc) = (0usize, 0usize);
        while pc < code.len() {
            match code[pc] {
                XformInstr::Constant(v) => {
                    stack[sp] = v;
                    sp += 1;
                }
                XformInstr::Read(r) => {
                    stack[sp] = read(r)?;
                    sp += 1;
                }
                XformInstr::Unary(op) => {
                    let v = stack[sp - 1];
                    stack[sp - 1] = match op {
                        XformUnary::Neg => v.wrapping_neg(),
                        XformUnary::Not => (v == 0) as i32,
                        XformUnary::Bool => (v != 0) as i32,
                        XformUnary::IntToFx => v.wrapping_mul(65536),
                        XformUnary::FxToInt => v / 65536,
                        XformUnary::Sin => crate::math::sin(crate::math::Angle(v as u16)).raw(),
                        XformUnary::Cos => crate::math::cos(crate::math::Angle(v as u16)).raw(),
                        XformUnary::Global => read_global(v, &mut read)?,
                    };
                }
                XformInstr::Binary(op) => {
                    let r = stack[sp - 1];
                    let l = stack[sp - 2];
                    sp -= 1;
                    stack[sp - 1] = match op {
                        XformBinary::Add => l.wrapping_add(r),
                        XformBinary::Sub => l.wrapping_sub(r),
                        XformBinary::Mul => l.wrapping_mul(r),
                        XformBinary::Div => {
                            if r == 0 {
                                return Err(XformEvalError::DivisionByZero);
                            } else {
                                l.wrapping_div(r)
                            }
                        }
                        XformBinary::Rem => {
                            if r == 0 {
                                return Err(XformEvalError::DivisionByZero);
                            } else {
                                l.wrapping_rem(r)
                            }
                        }
                        XformBinary::MulFx => ((l as i64 * r as i64) >> 16) as i32,
                        XformBinary::DivFx => {
                            if r == 0 {
                                return Err(XformEvalError::DivisionByZero);
                            } else {
                                (((l as i64) << 16) / r as i64) as i32
                            }
                        }
                        XformBinary::Eq => (l == r) as i32,
                        XformBinary::Ne => (l != r) as i32,
                        XformBinary::Lt => (l < r) as i32,
                        XformBinary::Le => (l <= r) as i32,
                        XformBinary::Gt => (l > r) as i32,
                        XformBinary::Ge => (l >= r) as i32,
                        XformBinary::Atan2 => crate::math::atan2(
                            crate::math::Fx::from_raw(l),
                            crate::math::Fx::from_raw(r),
                        )
                        .raw() as i32,
                        XformBinary::Dist => crate::math::isqrt(crate::math::len_sq(
                            crate::math::Fx::from_raw(l),
                            crate::math::Fx::from_raw(r),
                        ) as u64)
                        .min(i32::MAX as u32) as i32,
                    };
                }
                XformInstr::JumpIfZero(t) => {
                    sp -= 1;
                    if stack[sp] == 0 {
                        pc = t as usize;
                        continue;
                    }
                }
                XformInstr::Jump(t) => {
                    pc = t as usize;
                    continue;
                }
            }
            pc += 1;
        }
        Ok(stack[0])
    }
}

// Global's index is an expression value, not a capture number.
fn read_global(
    index: i32,
    read: &mut impl FnMut(XformRead) -> Result<i32, XformEvalError>,
) -> Result<i32, XformEvalError> {
    if !(0..crate::world::GLOBALS_CAP as i32).contains(&index) {
        return Err(XformEvalError::BadGlobal);
    }
    read(XformRead::Global(index as u16))
}

/// Fold a validated expression without exposing any world reads.
pub fn evaluate_constant(expr: &XformExpr) -> Option<i32> {
    expr.validate(0).ok()?;
    expr.eval(|_| Err(XformEvalError::BadArgs)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xform_expr_numeric_and_short_circuit() {
        use XformInstr::*;
        let e = XformExpr::try_code(vec![
            Constant(3 * 65536),
            Constant(32768),
            Binary(XformBinary::MulFx),
        ])
        .unwrap();
        assert_eq!(e.eval(|_| unreachable!()), Ok(98304));
        let e =
            XformExpr::try_code(vec![Constant(-7), Constant(3), Binary(XformBinary::Div)]).unwrap();
        assert_eq!(e.eval(|_| unreachable!()), Ok(-2));
        let e = XformExpr::try_code(vec![
            Constant(16384),
            Unary(XformUnary::Sin),
            Unary(XformUnary::FxToInt),
        ])
        .unwrap();
        assert_eq!(e.eval(|_| unreachable!()), Ok(1));

        let e = XformExpr::try_code(vec![
            Constant(0),
            JumpIfZero(6),
            Constant(1),
            Constant(0),
            Binary(XformBinary::Div),
            Jump(7),
            Constant(0),
        ])
        .unwrap();
        assert_eq!(e.eval(|_| unreachable!()), Ok(0));
    }

    #[test]
    fn xform_expr_rejects_bad_control_flow_and_stack() {
        use XformInstr::*;
        assert!(XformExpr::try_code(vec![Constant(0), Jump(0)]).is_err());
        assert!(XformExpr::try_code(vec![Constant(1), Constant(2)]).is_err());
        assert!(XformExpr::try_code(vec![Constant(1); 33]).is_err());
    }
}
