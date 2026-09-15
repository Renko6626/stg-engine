// rl-bench 密弹 workload 模板（`crates/stg-harness/src/rlbench.rs`；`__DENSITY__` 运行期替换为
// 每帧发弹数 K）。
//
// 设计：弹只在上半区（出生 y ∈ [40, 200)）从左右场边横穿，方向在水平 ±15° 内随机，4px/帧，
// 约 112 帧后越界回收（场半宽 192 + 边距 64）⇒ 稳态弹数 ≈ 112·K（少量从顶边提前出界）。
// 方向散布是刻意的：观测编码逐弹现算 atan2(vy,vx)，纯 0°/180° 会让 CORDIC 的走向全可预测、
// 派生量记忆也无从区分，测不出真实弹幕的代价；最低一颗飞到 y≈303，仍碰不到自机。bench 在此 workload 下只按 SHOT、不移动，
// 自机停在场底 y=384，弹永远碰不到它 ⇒ 不死、不 reset，计时区一直跑在「K 决定的弹量」上。
// 弹照常积分、照常进碰撞与观测编码，测到的开销是真实的。
const RICE: int = 64;

sub main() {
    loop {
        for i in 0..__DENSITY__ {
            var y: fx = (40 + rand(160)) as fx;
            var d: angle = (rand(5461) as angle) - 15deg;
            if rand(2) == 0 {
                _ = fire(RICE, i % 16, -192.0fx, y, 4.0fx, d, none, none);
            } else {
                _ = fire(RICE, i % 16, 192.0fx, y, 4.0fx, 180deg + d, none, none);
            }
        }
        wait(1);
    }
}
