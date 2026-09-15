//! Task 5 生命周期测试（spec §4/§6）：起点加权采样、种子流、模板缓存+reseed、随机预热重试、
//! events 8 列、done 0-3 与自动 reset。
use std::sync::Arc;
use stg_rl::env::*;

/// `main` 场顶炮台每帧朝自机发高速自机狙：不操作的自机 68 帧内被打死（harness `run` 实测）。
const SHOOTER: &str = r#"
const BALL: int = 48;
const COLOR_RED: int = 2;

async sub shooter_main() {
    loop {
        _ = fire(BALL, COLOR_RED, $self_x, $self_y, 6.0fx, aim_player(), none, none);
        wait(1);
    }
}

sub main() {
    _ = spawn_enemy(0.0fx, 24.0fx, 100000, 1, 0, 0, shooter_main);
    loop { wait(1); }
}
"#;

/// 30 帧后 `stage_clear(1)`：产生 `EVT_STAGE_CLEARED`（segment_end 列 = 4，harness 实测帧 31）。
const QUICK_END: &str = r#"
sub main() {
    wait(30);
    stage_clear(1);
    loop { wait(1); }
}
"#;

/// 只 `wait`：不发弹、无段落事件、永不结束。
const IDLE: &str = r#"
sub main() {
    loop { wait(1); }
}
"#;

fn img(src: &str) -> Image {
    compile(&[("t.ecl".into(), src.into())]).unwrap()
}
fn cfg(images: Vec<Image>, end_on: Vec<EndOn>) -> EnvConfig {
    EnvConfig {
        images,
        starts: vec![Start {
            image: 0,
            mark: 0,
            rank: 0,
            weight: 1.0,
        }],
        frame_skip: 1,
        max_frames: 600,
        warmup_max: 0,
        end_on,
        bullets_cap: 1024,
        seed: 7,
    }
}
fn run(env: &mut Env, frames: usize, action: u32) -> Vec<StepOut> {
    (0..frames).map(|_| env.step(action)).collect()
}

#[test]
fn validate_rejects_bad_config() {
    let good = cfg(vec![img(IDLE)], vec![]);
    assert!(validate(&good).is_ok());
    let mut c = good.clone();
    c.starts[0].mark = 999;
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.starts[0].image = 5;
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.starts[0].rank = 99;
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.bullets_cap = 0;
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.bullets_cap = 8193;
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.frame_skip = 0;
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.starts[0].weight = 0.0;
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.max_frames = 0;
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.starts.clear();
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.starts[0].weight = f64::NAN;
    assert!(validate(&c).is_err());
    let mut c = good.clone();
    c.starts[0].weight = f64::INFINITY;
    assert!(validate(&c).is_err());
    assert!(
        compile(&[("bad.ecl".into(), "sub main( {".into())])
            .unwrap_err()
            .contains("bad.ecl")
    );
}

#[test]
fn character_is_classic_kit() {
    let c = Arc::new(cfg(vec![img(IDLE)], vec![]));
    let env = Env::new(c, 0, Arc::new(BootCache::new()));
    assert_eq!(env.world().view().players()[0].character_id, 1);
}

#[test]
fn death_ends_with_done_1_and_autoresets() {
    let c = Arc::new(cfg(vec![img(SHOOTER)], vec![]));
    let mut env = Env::new(c, 0, Arc::new(BootCache::new()));
    let outs = run(&mut env, 400, 0);
    let k = outs
        .iter()
        .position(|o| o.done != DONE_NONE)
        .expect("不操作必死");
    assert_eq!(outs[k].done, DONE_DIED);
    assert_eq!(outs[k].events[0], 1, "died 列");
    assert_eq!(outs[k].ep_frames, k as i32 + 1);
    assert_eq!(
        env.world().view().players()[0].deaths,
        0,
        "已自动 reset 成新局"
    );
}

#[test]
fn segment_end_is_done_2_only_when_listed() {
    let c = Arc::new(cfg(vec![img(QUICK_END)], vec![EndOn::StageCleared]));
    let mut env = Env::new(c, 0, Arc::new(BootCache::new()));
    let o = run(&mut env, 100, 0)
        .into_iter()
        .find(|o| o.done != 0)
        .unwrap();
    assert_eq!((o.done, o.events[7]), (DONE_SEGMENT, 4));

    let c = Arc::new(cfg(vec![img(QUICK_END)], vec![]));
    let mut env = Env::new(c, 0, Arc::new(BootCache::new()));
    let outs = run(&mut env, 100, 0);
    assert!(outs.iter().all(|o| o.done == 0), "未列入 end_on 不结束");
    assert!(
        outs.iter().any(|o| o.events[7] == 4),
        "但 segment_end 列照记"
    );
}

#[test]
fn timeout_is_done_3() {
    let mut c = cfg(vec![img(IDLE)], vec![]);
    c.max_frames = 50;
    let mut env = Env::new(Arc::new(c), 0, Arc::new(BootCache::new()));
    let outs = run(&mut env, 50, 0);
    assert_eq!(outs[49].done, DONE_TIMEOUT);
    assert!(outs[..49].iter().all(|o| o.done == 0));
}

#[test]
fn frame_skip_accumulates_and_stops_at_done() {
    let mut c = cfg(vec![img(IDLE)], vec![]);
    c.frame_skip = 3;
    c.max_frames = 10;
    let mut env = Env::new(Arc::new(c), 0, Arc::new(BootCache::new()));
    let outs = run(&mut env, 4, 0);
    assert_eq!(outs[3].done, DONE_TIMEOUT);
    assert_eq!(outs[3].ep_frames, 10, "判到即停：第 4 步只跑了 1 帧");
}

#[test]
fn same_seed_same_actions_same_world_and_seed_matters() {
    let mk = |seed| {
        let mut c = cfg(vec![stg_rl_game()], vec![]);
        c.starts[0].mark = 15;
        c.warmup_max = 60;
        c.seed = seed;
        Env::new(Arc::new(c), 3, Arc::new(BootCache::new()))
    };
    let (mut a, mut b, mut d) = (mk(1), mk(1), mk(2));
    for f in 0..300u32 {
        let act = f.wrapping_mul(2654435761) & 0x7F;
        let (oa, ob) = (a.step(act), b.step(act));
        d.step(act);
        assert_eq!((oa.done, oa.events), (ob.done, ob.events));
    }
    assert_eq!(a.world().checksum(), b.world().checksum());
    assert_ne!(a.world().checksum(), d.world().checksum());
}

#[test]
fn warmup_retries_are_bounded_and_deterministic() {
    let mut c = cfg(vec![img(SHOOTER)], vec![]);
    c.warmup_max = 400; // 预热期间几乎必死 ⇒ 反复重试
    let c = Arc::new(c);
    let mut a = Env::new(c.clone(), 0, Arc::new(BootCache::new()));
    let mut b = Env::new(c, 0, Arc::new(BootCache::new()));
    let (oa, ob) = (run(&mut a, 400, 0), run(&mut b, 400, 0));
    let ra: Vec<i32> = oa
        .iter()
        .filter(|o| o.done != 0)
        .map(|o| o.warmup_retries)
        .collect();
    let rb: Vec<i32> = ob
        .iter()
        .filter(|o| o.done != 0)
        .map(|o| o.warmup_retries)
        .collect();
    assert_eq!(ra, rb);
    assert!(ra.iter().any(|&r| r > 0), "SHOOTER 预热期应触发重试");
    assert!(
        ra.iter()
            .all(|&r| (0..=MAX_WARMUP_RETRIES as i32).contains(&r))
    );
}

#[test]
fn reset_does_not_allocate_new_world_box() {
    // SHOOTER + 大 warmup_max ⇒ 预热期几乎必死，reset 内部反复从模板 copy_into；
    // 同一 Box 复用 ⇒ 地址整局存活期不变（不再每次 reset `World::new`）。
    let mut c = cfg(vec![img(SHOOTER)], vec![]);
    c.warmup_max = 400;
    let mut env = Env::new(Arc::new(c), 0, Arc::new(BootCache::new()));
    let addr = env.world() as *const _ as usize;
    for i in 0..3 {
        env.reset();
        assert_eq!(
            env.world() as *const _ as usize,
            addr,
            "第 {i} 次 reset 换了新 World Box"
        );
    }
}

#[test]
fn boot_cache_template_is_reused() {
    let c = Arc::new({
        let mut c = cfg(vec![stg_rl_game()], vec![]);
        c.starts[0].mark = 10;
        c
    });
    let cache = Arc::new(BootCache::new());
    let mut e0 = Env::new(c.clone(), 0, cache.clone());
    let _e1 = Env::new(c, 1, cache.clone());
    e0.reset();
    assert_eq!(cache.len(), 1);
}

fn stg_rl_game() -> Image {
    let units: Vec<(String, String)> = stg_rl::bundled::bundled_sources("game")
        .unwrap()
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    compile(&units).unwrap()
}
