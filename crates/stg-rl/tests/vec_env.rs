use stg_rl::env::*;
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
    }
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
