//! stg-rl env 刀 Task 1：`World::reseed` 与直接开机逐帧等价（spec §8）。
//!
//! **为何落在本 crate 而非 stg-core**：等价判别需要一枚**开机后真的消费 RNG** 的 main
//! （`rand(100)`），而 stg-core 不能依赖编译器（P1 断层线），其 `mod tests` 也没有手拼
//! RNG-syscall 字节码的镜像 helper。故用编译器产出镜像，断言不变。

use stg_core::input::InputFrame;
use stg_core::player::Loadout;
use stg_core::step::{World, step};
use stg_core::tables::TABLES_V0;

/// reseed 等价（stg-rl env 刀 spec §8）：seed=0 开机模板 + reseed(s) 与直接 new_game_at(s)
/// 在 `(seed, rank, mark)` 各维度上抽样校验和相同。若红 ⇒ new_game_at 在开机期消费了 RNG /
/// 写了 seed 相关状态，stg-rl 须放弃模板缓存。
///
/// 抽样而非逐帧：`checksum()` 是字段级哈希全槽（debug 约 15.6 ms/次，1.14 MiB World），
/// 逐帧 3600 次要 ~50 s。f=0 已覆盖 spec 点名的失效模式（开机消费 RNG / 写 seed 相关状态），
/// 后续帧的分歧在相同确定性 step 下持续存在，必被下一次抽样或末帧捕获。
#[test]
fn reseed_after_template_equals_direct_boot() {
    // `mark(10)` 落点之后 main 仍每帧 `rand(100)` 真消费 RNG，覆盖中段启动路径（spec §8）。
    let src = "sub main() { mark(10) { } loop { var b: int = rand(100); wait(1); } }";
    let image = stg_ecl_compiler::lang::compile(src, "t.ecl").expect("probe 脚本必须编过");
    for &(seed, rank, mark) in &[(1u64, 0i32, 0i32), (0xDEAD_BEEF, 2, 10), (u64::MAX, 3, 0)] {
        let ld = Loadout {
            character: 1,
            ..Loadout::default()
        };
        let mut a = World::new_game_at(0, rank, mark, ld, &image).unwrap();
        a.reseed(seed);
        let mut b = World::new_game_at(seed, rank, mark, ld, &image).unwrap();
        assert_eq!(a.seed(), seed);
        for f in 0..600 {
            if f == 0 || f % 60 == 0 {
                assert_eq!(
                    a.checksum(),
                    b.checksum(),
                    "seed {seed:#x} rank {rank} mark {mark} frame {f}"
                );
            }
            let input = InputFrame::empty(a.frame());
            step(&mut a, &TABLES_V0, &image, &input);
            step(&mut b, &TABLES_V0, &image, &input);
        }
        assert_eq!(
            a.checksum(),
            b.checksum(),
            "seed {seed:#x} rank {rank} mark {mark} frame 600"
        );
    }
}
