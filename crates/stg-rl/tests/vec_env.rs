use std::sync::Arc;

use stg_rl::encode;
use stg_rl::env::*;
use stg_rl::layout::ITEMS_CAP;
use stg_rl::vec_env::*;

struct Owned {
    frame: Vec<u32>,
    phase: Vec<u32>,
    player: Vec<u8>,
    enemies: Vec<u8>,
    enemies_count: Vec<i32>,
    bullets: Vec<u8>,
    bullets_offsets: Vec<i32>,
    items: Vec<u8>,
    items_offsets: Vec<i32>,
    lasers_count: Vec<i32>,
    bullets_total: Vec<i32>,
    bullets_dropped: Vec<i32>,
    events: Vec<i32>,
    done: Vec<u8>,
    ep_frames: Vec<i32>,
    warmup_retries: Vec<i32>,
    start_index: Vec<i32>,
}

impl Owned {
    fn new(n: usize, cap: usize) -> Owned {
        let s = buffer_sizes(n, cap);
        Owned {
            frame: vec![0; s.frame],
            phase: vec![0; s.phase],
            player: vec![0; s.player],
            enemies: vec![0; s.enemies],
            enemies_count: vec![0; s.enemies_count],
            bullets: vec![0; s.bullets],
            bullets_offsets: vec![0; s.bullets_offsets],
            items: vec![0; s.items],
            items_offsets: vec![0; s.items_offsets],
            lasers_count: vec![0; s.lasers_count],
            bullets_total: vec![0; s.bullets_total],
            bullets_dropped: vec![0; s.bullets_dropped],
            events: vec![0; s.events],
            done: vec![0; s.done],
            ep_frames: vec![0; s.ep_frames],
            warmup_retries: vec![0; s.warmup_retries],
            start_index: vec![0; s.start_index],
        }
    }
    fn view(&mut self) -> BufferSet<'_> {
        BufferSet {
            frame: &mut self.frame,
            phase: &mut self.phase,
            player: &mut self.player,
            enemies: &mut self.enemies,
            enemies_count: &mut self.enemies_count,
            bullets: &mut self.bullets,
            bullets_offsets: &mut self.bullets_offsets,
            items: &mut self.items,
            items_offsets: &mut self.items_offsets,
            lasers_count: &mut self.lasers_count,
            bullets_total: &mut self.bullets_total,
            bullets_dropped: &mut self.bullets_dropped,
            events: &mut self.events,
            done: &mut self.done,
            ep_frames: &mut self.ep_frames,
            warmup_retries: &mut self.warmup_retries,
            start_index: &mut self.start_index,
        }
    }
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
        for (i, &c) in self.enemies_count.iter().enumerate() {
            v.extend_from_slice(&self.enemies[i * 256 * 38..][..c as usize * 38]);
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

    let mut scratch: Vec<(i64, u16)> = Vec::new();
    let mut bullet_rows = vec![0u8; cap * 30];
    let mut item_rows = vec![0u8; ITEMS_CAP * 18];

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
