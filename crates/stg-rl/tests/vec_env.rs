use std::collections::HashMap;
use std::sync::Arc;

use stg_core::math::Fx;
use stg_rl::encode;
use stg_rl::env::*;
use stg_rl::layout::{ITEMS_CAP, LASERS_CAP, off};
use stg_rl::vec_env::*;

/// 测试用缓冲 = 公开的 `OwnedBuffers`（与 harness rl-bench 共用，不再各写一份）。
type Owned = OwnedBuffers;

/// 测试专用的规范化快照（扩展 trait：`OwnedBuffers` 是外部类型，不能加固有方法）。
trait Digest {
    fn digest(&self) -> Vec<u8>;
}

impl Digest for Owned {
    /// 有效数据的规范化快照：CSR 只比前缀（后缀是暂存区残留，不属于输出）。
    fn digest(&self) -> Vec<u8> {
        let nb = *self.bullets_offsets.last().unwrap() as usize * 30;
        let ni = *self.items_offsets.last().unwrap() as usize * 18;
        let mut v = Vec::new();
        for s in [
            &self.player[..],
            &self.bullets[..nb],
            &self.items[..ni],
            &self.done[..],
        ] {
            v.extend_from_slice(s);
        }
        for s in [
            &self.enemies_count,
            &self.bullets_offsets,
            &self.items_offsets,
            &self.lasers_count,
            &self.bullets_total,
            &self.bullets_dropped,
            &self.events,
            &self.ep_frames,
            &self.warmup_retries,
            &self.start_index,
        ] {
            for x in s.iter() {
                v.extend_from_slice(&x.to_le_bytes());
            }
        }
        let est = stg_rl::layout::ENEMIES.stride;
        for (i, &c) in self.enemies_count.iter().enumerate() {
            v.extend_from_slice(&self.enemies[i * 256 * est..][..c as usize * est]);
        }
        // lasers 是定长行区（每 env 恰 LASERS_CAP 行，不压实）：只比有效前缀，尾行是残留。
        let lst = stg_rl::layout::LASERS.stride;
        for (i, &c) in self.lasers_count.iter().enumerate() {
            v.extend_from_slice(&self.lasers[i * LASERS_CAP * lst..][..c as usize * lst]);
        }
        for s in [&self.frame, &self.phase] {
            for x in s.iter() {
                v.extend_from_slice(&x.to_le_bytes());
            }
        }
        v
    }
}

fn game_cfg(seed: u64) -> EnvConfig {
    let units: Vec<(String, String)> = stg_rl::bundled::bundled_sources("game")
        .unwrap()
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    EnvConfig {
        images: vec![compile(&units).unwrap()],
        starts: vec![
            Start {
                image: 0,
                mark: 10,
                rank: 2,
                weight: 1.0,
            },
            Start {
                image: 0,
                mark: 15,
                rank: 2,
                weight: 2.0,
            },
        ],
        frame_skip: 1,
        max_frames: 150,
        warmup_max: 60,
        end_on: vec![
            EndOn::PhaseEnded,
            EndOn::SpellCaptured,
            EndOn::SpellFailed,
            EndOn::StageCleared,
        ],
        bullets_cap: 256,
        seed,
    }
}

fn actions(step: u32, n: usize) -> Vec<u32> {
    (0..n as u32)
        .map(|i| (step.wrapping_mul(2654435761) ^ i.wrapping_mul(40503)) & 0x7F)
        .collect()
}

#[test]
fn deterministic_across_thread_counts() {
    let n = 16;
    let (mut a, mut b) = (
        VecEnv::new(game_cfg(5), n, 1).unwrap(),
        VecEnv::new(game_cfg(5), n, 8).unwrap(),
    );
    let (mut ba, mut bb) = (Owned::new(n, 256), Owned::new(n, 256));
    a.reset(&mut ba.view()).unwrap();
    b.reset(&mut bb.view()).unwrap();
    assert_eq!(ba.digest(), bb.digest());
    let mut saw_done = false;
    for s in 0..300 {
        let act = actions(s, n);
        a.step(&act, &mut ba.view()).unwrap();
        b.step(&act, &mut bb.view()).unwrap();
        assert_eq!(ba.digest(), bb.digest(), "step {s}");
        saw_done |= ba.done.iter().any(|&d| d != 0);
    }
    assert!(
        saw_done,
        "300 步随机动作应至少结束过一局（否则本测试没覆盖自动 reset）"
    );
}

#[test]
fn different_seed_differs() {
    let n = 4;
    let (mut a, mut b) = (
        VecEnv::new(game_cfg(5), n, 2).unwrap(),
        VecEnv::new(game_cfg(6), n, 2).unwrap(),
    );
    let (mut ba, mut bb) = (Owned::new(n, 256), Owned::new(n, 256));
    a.reset(&mut ba.view()).unwrap();
    b.reset(&mut bb.view()).unwrap();
    for s in 0..120 {
        let act = actions(s, n);
        a.step(&act, &mut ba.view()).unwrap();
        b.step(&act, &mut bb.view()).unwrap();
    }
    assert_ne!(ba.digest(), bb.digest());
}

#[test]
fn csr_offsets_are_consistent() {
    let n = 8;
    let mut e = VecEnv::new(game_cfg(9), n, 4).unwrap();
    let mut buf = Owned::new(n, 256);
    e.reset(&mut buf.view()).unwrap();
    // 判别力：全 0 行的退化压实也能满足下面所有「单调 / 上界 / total-dropped」断言，
    // 故必须有一处正面积压断言。记录首次非空步，跑满 200 步后再断言（不 flaky）。
    let mut first_nonempty: Option<u32> = None;
    for s in 0..200 {
        e.step(&actions(s, n), &mut buf.view()).unwrap();
        assert_eq!(buf.bullets_offsets[0], 0);
        for i in 0..n {
            let cnt = buf.bullets_offsets[i + 1] - buf.bullets_offsets[i];
            assert!(cnt >= 0 && cnt as usize <= 256);
            assert_eq!(cnt, buf.bullets_total[i] - buf.bullets_dropped[i]);
            assert!(buf.items_offsets[i + 1] >= buf.items_offsets[i]);
            // 内置 game 内容包不含 `laser()`，本场景 lasers_count 必为 0；
            // 带激光场景的非零断言见 `lasers_rows_match_direct_encode_and_count`。
            assert_eq!(buf.lasers_count[i], 0);
        }
        if first_nonempty.is_none() && buf.bullets_offsets[n] > 0 {
            first_nonempty = Some(s);
        }
    }
    assert!(
        first_nonempty.is_some(),
        "200 步内 mark 10/15 段应有弹被压实（实测首次非空步: {first_nonempty:?}）"
    );
}

#[test]
fn compacted_rows_match_direct_encode() {
    let n = 3usize;
    let cap = 256usize;
    let cfg = game_cfg(7);
    let mut ve = VecEnv::new(cfg.clone(), n, 2).unwrap();
    let mut buf = Owned::new(n, cap);

    // 3 个独立 Env：相同 EnvConfig、env_index 0/1/2、各自 BootCache。
    let cfg = Arc::new(cfg);
    let mut direct: Vec<Env> = (0..n)
        .map(|i| Env::new(cfg.clone(), i as u32, Arc::new(BootCache::new())))
        .collect();

    let mut scratch = encode::BulletScratch::new();
    let mut bullet_rows = vec![0u8; cap * 30];
    let mut item_rows = vec![0u8; ITEMS_CAP * 18];

    // 两侧都「构造即 reset + 再显式 reset」：这份**双 reset 的对称是刻意的**——`VecEnv::reset`
    // 与 `Env::new` 的组合同样会让每个 env 的 counter +1。改动任一侧须同步，否则 counter 错位、
    // 种子流分叉，本对拍会红（见 crate::vec_env::VecEnv::reset 文档）。
    ve.reset(&mut buf.view()).unwrap();
    for e in direct.iter_mut() {
        e.reset();
    }

    let mut saw_rows = false;
    for s in 0..200usize {
        let act = actions(s as u32, n);
        ve.step(&act, &mut buf.view()).unwrap();
        for (i, e) in direct.iter_mut().enumerate() {
            let _ = e.step(act[i]);
            let bs = encode::write_bullets(e.world(), cap, &mut bullet_rows, &mut scratch);
            let ni = encode::write_items(e.world(), &mut item_rows);

            let b0 = buf.bullets_offsets[i] as usize;
            let b1 = buf.bullets_offsets[i + 1] as usize;
            let i0 = buf.items_offsets[i] as usize;
            let i1 = buf.items_offsets[i + 1] as usize;
            assert_eq!(
                b1 - b0,
                bs.count,
                "env {i} step {s}: 压实行数 != 直接编码行数"
            );
            assert_eq!(
                i1 - i0,
                ni,
                "env {i} step {s}: item 压实行数 != 直接编码行数"
            );
            assert_eq!(
                &buf.bullets[b0 * 30..b1 * 30],
                &bullet_rows[..bs.count * 30],
                "env {i} step {s}: bullet 压实字节 != 直接编码"
            );
            assert_eq!(
                &buf.items[i0 * 18..i1 * 18],
                &item_rows[..ni * 18],
                "env {i} step {s}: item 压实字节 != 直接编码"
            );
            saw_rows |= bs.count > 0 || ni > 0;
        }
    }
    assert!(
        saw_rows,
        "200 步内应至少有一 env 某步压实行数 > 0（否则本测试对压实内容是瞎的）"
    );
}

/// T5：lasers 行区是**非 CSR 压实**的定长区（每 env 恰 `LASERS_CAP` 行），step 后与
/// `encode::write_lasers` 直接编码逐字节对拍；`lasers_count` 必须等于返回行数。最小 ECL
/// 每 60 帧建一条 warn=30/active=120/fade=16 的激光 ⇒ 220 步内必有非零帧。判别力：计数写成
/// 0、行区错位、把 lasers 误走 CSR 都会红。
#[test]
fn lasers_rows_match_direct_encode_and_count() {
    let src = r#"
sub main() {
    loop {
        _ = laser(4, 0.0fx, 100.0fx, 90deg, 500.0fx, 32.0fx, 30, 120, 16);
        wait(60);
    }
}
"#;
    let image = compile(&[("t5_laser.ecl".to_string(), src.to_string())]).unwrap();
    let cfg = EnvConfig {
        images: vec![image],
        starts: vec![Start {
            image: 0,
            mark: 0,
            rank: 1,
            weight: 1.0,
        }],
        frame_skip: 1,
        max_frames: 1000,
        warmup_max: 0,
        end_on: vec![],
        bullets_cap: 32,
        seed: 1,
    };
    let n = 1;
    // 双 reset 对称：`VecEnv::reset` + `Env::new` 的组合与 `VecEnv::new` + 显式 reset 一致。
    let mut ve = VecEnv::new(cfg.clone(), n, 1).unwrap();
    let mut direct = Env::new(Arc::new(cfg), 0, Arc::new(BootCache::new()));
    let mut buf = Owned::new(n, 32);
    ve.reset(&mut buf.view()).unwrap();
    direct.reset();

    let lst = stg_rl::layout::LASERS.stride;
    let mut laser_rows = vec![0u8; LASERS_CAP * lst];
    let mut saw = false;
    for s in 0..220u32 {
        ve.step(&[0u32], &mut buf.view()).unwrap();
        let _ = direct.step(0);
        let k = encode::write_lasers(direct.world(), &mut laser_rows);
        assert_eq!(
            buf.lasers_count[0], k as i32,
            "step {s}: lasers_count != 直接编码行数"
        );
        assert_eq!(
            &buf.lasers[..k * lst],
            &laser_rows[..k * lst],
            "step {s}: 激光行字节 != 直接编码"
        );
        saw |= k > 0;
    }
    assert!(saw, "220 步内应至少有一帧 lasers_count > 0");
    // 行里可读：half_h = width/2 = 16px（Q16.16 raw），state 为 0/1/2。
    let off = off::laser::HALF_H;
    let half_h = i32::from_le_bytes(buf.lasers[off..off + 4].try_into().unwrap());
    assert_eq!(half_h, Fx::from_int(16).raw(), "half_h 应等于 width/2");
    assert!(buf.lasers[off::laser::STATE] <= 2, "state 0/1/2");
}

#[test]
fn rejects_bad_buffers_actions_weights() {
    let n = 4;
    let mut e = VecEnv::new(game_cfg(1), n, 2).unwrap();
    let mut small = Owned::new(n, 128); // bullets 缓冲按 cap=128 分配，env cap 是 256
    assert!(e.reset(&mut small.view()).is_err());
    let mut buf = Owned::new(n, 256);
    e.reset(&mut buf.view()).unwrap();
    assert!(e.step(&[0; 3], &mut buf.view()).is_err());
    assert!(e.set_start_weights(vec![1.0]).is_err(), "长度须等于起点数");
    assert!(e.set_start_weights(vec![1.0, -1.0]).is_err());
    assert!(
        e.set_start_weights(vec![0.0, 3.0]).is_ok(),
        "允许单个为 0，总和须 > 0"
    );
    assert!(VecEnv::new(game_cfg(1), 0, 2).is_err());
}

/// 端到端对拍（Task 4 Review Focus 4）：内联 ECL 建三只敌——一只 `move_to` 插值 30 帧、
/// 一只匀速漂移、一只在场上等 20 帧后瞬移 +100px——`VecEnv` 跑 40 步，逐步按 `id` 匹配上一步
/// 的行，核对 Tier 0 `vx`/`vy`（取自池 `dx`/`dy`）与坐标差的关系：
/// - 正常帧（位移 ≤16px）：vx/vy 逐位等于坐标差；
/// - 瞬移帧（坐标差 ~100px）：vx/vy **不**等于坐标差（瞬移不计入 dx/dy）；
/// - 匀速敌新出现的第一步：vx/vy 就是它的速度（1.5fx，笛卡尔立即设）。
#[test]
fn enemy_rows_vx_vy_match_dxdy_across_move_to_const_vel_and_teleport() {
    let src = r#"
async sub drift_lerp() {
    move_to(30, 40.0fx, 50.0fx, 0);
    loop { wait(1); }
}

async sub drift_const() {
    move_vel_xy(0, 1.5fx, 0.0fx, 0);
    loop { wait(1); }
}

async sub drift_teleport() {
    wait(20);
    move_to(0, $self_x + 100.0fx, $self_y, 0);
    loop { wait(1); }
}

sub main() {
    _ = spawn_enemy(0.0fx, 50.0fx, 1000, 0, 0, 0, drift_lerp);
    _ = spawn_enemy(-60.0fx, 50.0fx, 1000, 0, 0, 0, drift_const);
    _ = spawn_enemy(60.0fx, 50.0fx, 1000, 0, 0, 0, drift_teleport);
    loop { wait(1); }
}
"#;
    let image = compile(&[("t4_dxdy.ecl".to_string(), src.to_string())]).unwrap();
    let cfg = EnvConfig {
        images: vec![image],
        starts: vec![Start {
            image: 0,
            mark: 0,
            rank: 1,
            weight: 1.0,
        }],
        frame_skip: 1,
        max_frames: 1000,
        warmup_max: 0,
        end_on: vec![],
        bullets_cap: 32,
        seed: 1,
    };
    let n = 1;
    let mut ve = VecEnv::new(cfg, n, 1).unwrap();
    let mut buf = Owned::new(n, 32);
    ve.reset(&mut buf.view()).unwrap();

    let st = stg_rl::layout::ENEMIES.stride;
    let rd_i32 = |b: &[u8], o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let rd_u32 = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());

    /// env 0 的敌人行 → `id -> (x, y, vx, vy)`（Q16.16 原值）。
    fn snapshot(
        rows: &[u8],
        count: i32,
        st: usize,
        rd_i32: &dyn Fn(&[u8], usize) -> i32,
        rd_u32: &dyn Fn(&[u8], usize) -> u32,
    ) -> HashMap<u32, (i32, i32, i32, i32)> {
        let mut m = HashMap::new();
        for k in 0..count as usize {
            let r = &rows[k * st..(k + 1) * st];
            let id = rd_u32(r, off::enemy::ID);
            let x = rd_i32(r, off::enemy::X);
            let y = rd_i32(r, off::enemy::Y);
            let vx = rd_i32(r, off::enemy::VX);
            let vy = rd_i32(r, off::enemy::VY);
            m.insert(id, (x, y, vx, vy));
        }
        m
    }

    let sixteen_px = Fx::from_int(16).raw();
    let const_vel_x = 98_304i32; // 1.5fx = 1.5 * 65536，精确整数
    // 匀速敌出生于 (-60, 50)。ECL 的敌主任务"出生当帧不跑"（docs/ecl-lang.md 五条坑之一）：
    // `drift_const` 里的 `move_vel_xy` 要到出生后下一帧才首次执行，故这只敌**第一次出现在
    // 观测里那一帧**（= spawn_enemy 那一帧）vx/vy 必为 0（池新分配槽的 dx/dy 精确清零，
    // exhaustive Init 的直接验证）；速度要到*下一步*才体现——那一步已经落进下面 `Some` 分支的
    // 通用坐标差核对，同时额外显式核对它等于我们设的 1.5fx。
    let const_vel_spawn_x = Fx::from_int(-60).raw();
    let mut const_vel_id: Option<u32> = None;
    let mut checked_uniform_velocity_value = false;

    let mut prev = snapshot(
        &buf.enemies[0..256 * st],
        buf.enemies_count[0],
        st,
        &rd_i32,
        &rd_u32,
    );
    let mut checked_new_appearance = false;
    let mut checked_normal_frame = false;
    let mut checked_teleport_frame = false;

    for s in 0..40u32 {
        ve.step(&[0u32], &mut buf.view()).unwrap();
        let cur = snapshot(
            &buf.enemies[0..256 * st],
            buf.enemies_count[0],
            st,
            &rd_i32,
            &rd_u32,
        );
        for (&id, &(x, y, vx, vy)) in cur.iter() {
            match prev.get(&id) {
                None => {
                    // 新出现的敌：本帧就是它的 spawn 帧，本帧位移恒为 0（主任务出生当帧不跑，
                    // 还没来得及发任何运动指令）——vx/vy 必须精确等于它本帧的（零）位移，
                    // 直接验证新分配槽的 dx/dy 没有携带上一任槽主的陈值。
                    assert_eq!(
                        (vx, vy),
                        (0, 0),
                        "新出现的敌首帧 vx/vy 必为 0，id {id} 帧 {s}"
                    );
                    checked_new_appearance = true;
                    if x == const_vel_spawn_x && y == Fx::from_int(50).raw() {
                        const_vel_id = Some(id);
                    }
                }
                Some(&(px, py, _, _)) => {
                    let (dx, dy) = (x - px, y - py);
                    if dx.abs() <= sixteen_px && dy.abs() <= sixteen_px {
                        assert_eq!(
                            (vx, vy),
                            (dx, dy),
                            "正常帧 vx/vy 应等于坐标差，id {id} 帧 {s}"
                        );
                        checked_normal_frame = true;
                        if Some(id) == const_vel_id && vx != 0 {
                            // 匀速敌：命令一旦生效（下一帧），vx 应精确等于我们设的 1.5fx。
                            assert_eq!(vx, const_vel_x, "匀速敌 vx 应等于设定速度，帧 {s}");
                            assert_eq!(vy, 0, "匀速敌 vy 应恒为 0，帧 {s}");
                            checked_uniform_velocity_value = true;
                        }
                    } else {
                        assert_ne!(
                            (vx, vy),
                            (dx, dy),
                            "瞬移帧 vx/vy 不应等于坐标差，id {id} 帧 {s}"
                        );
                        assert!(
                            dx.abs() > Fx::from_int(50).raw(),
                            "瞬移坐标差应 ~100px，实得 {dx}（raw），帧 {s}"
                        );
                        checked_teleport_frame = true;
                    }
                }
            }
        }
        prev = cur;
    }
    assert!(checked_new_appearance, "从未验证过新出现敌首帧 vx/vy = 0");
    assert!(
        checked_uniform_velocity_value,
        "从未验证过匀速敌 vx == 设定速度 1.5fx"
    );
    assert!(checked_normal_frame, "从未验证过正常帧 vx/vy == 坐标差");
    assert!(checked_teleport_frame, "从未验证过瞬移帧 vx/vy != 坐标差");
}

// ── 判定点写口 set_hit_radius_extra（2026-09-25）────────────────────────────

fn player_hit_r(b: &Owned, env: usize) -> i32 {
    let st = stg_rl::layout::PLAYER.stride;
    let o = env * st + off::player::HIT_R;
    i32::from_le_bytes(b.player[o..o + 4].try_into().unwrap())
}

/// extra 全 0 = 不调写 API：与从没调过的 VecEnv 逐位相同（默认行为不变）。
#[test]
fn hit_extra_zero_is_bitwise_noop() {
    let n = 8;
    let (mut a, mut b) = (
        VecEnv::new(game_cfg(3), n, 2).unwrap(),
        VecEnv::new(game_cfg(3), n, 2).unwrap(),
    );
    b.set_hit_radius_extra(&vec![0.0; n]).unwrap();
    let (mut ba, mut bb) = (Owned::new(n, 256), Owned::new(n, 256));
    a.reset(&mut ba.view()).unwrap();
    b.reset(&mut bb.view()).unwrap();
    for s in 0..200 {
        let act = actions(s, n);
        a.step(&act, &mut ba.view()).unwrap();
        b.step(&act, &mut bb.view()).unwrap();
        assert_eq!(ba.digest(), bb.digest(), "step {s}");
    }
}

/// 只给 env 0 追加 3 px：观测里 env 0 的判定 = 表值 + 3，其余不动；跨 reset 保持；写回 0 恢复表值。
#[test]
fn hit_extra_applies_per_env_and_persists_across_reset() {
    let n = 4;
    let mut e = VecEnv::new(game_cfg(4), n, 2).unwrap();
    let mut b = Owned::new(n, 256);
    e.reset(&mut b.view()).unwrap();
    let base = player_hit_r(&b, 0);
    let mut extra = vec![0.0; n];
    extra[0] = 3.0;
    e.set_hit_radius_extra(&extra).unwrap();
    e.step(&actions(0, n), &mut b.view()).unwrap();
    assert_eq!(player_hit_r(&b, 0), base + Fx::from_int(3).raw());
    assert_eq!(player_hit_r(&b, 1), base);
    e.reset(&mut b.view()).unwrap();
    assert_eq!(
        player_hit_r(&b, 0),
        base + Fx::from_int(3).raw(),
        "新局照样生效"
    );
    e.set_hit_radius_extra(&vec![0.0; n]).unwrap();
    e.step(&actions(1, n), &mut b.view()).unwrap();
    assert_eq!(player_hit_r(&b, 0), base, "写回 0 恢复表值");
}

#[test]
fn hit_extra_rejects_bad_input() {
    let n = 2;
    let mut e = VecEnv::new(game_cfg(1), n, 1).unwrap();
    assert!(e.set_hit_radius_extra(&[1.0]).is_err(), "长度须等于 env 数");
    assert!(e.set_hit_radius_extra(&[f64::NAN, 0.0]).is_err());
    assert!(e.set_hit_radius_extra(&[2000.0, 0.0]).is_err());
}
