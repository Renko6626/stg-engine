//! 定点数学核（D1）—— 纯整数、无状态、跨平台 bit 级确定（I1/I2）。
//!
//! 超越函数走烘焙表：字节 commit 进 `tables/`，`include_bytes!` + `const fn` 解码。

pub mod angle;
pub mod codec;
pub mod cordic;
pub mod easing;
pub mod fx;
pub mod geom;
pub mod isqrt;
pub mod trig;

pub use angle::Angle;
pub use cordic::atan2;
pub use easing::{Easing, ease};
pub use fx::Fx;
pub use geom::{len_sq, polar_to_vec};
pub use isqrt::isqrt;
pub use trig::{cos, sin, sincos};
