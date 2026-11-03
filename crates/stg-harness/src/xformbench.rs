//! Optional equal-trajectory xform benchmark; setup and checking stay outside timers.
use std::process::ExitCode;
use std::time::Instant;
use stg_core::ecl::image::EclImage;
use stg_core::input::InputFrame;
use stg_core::math::Fx;
use stg_core::step::{World, step};
use stg_core::tables::TABLES_V0;
use stg_ecl_compiler::lang::compile;

struct Case {
    name: &'static str,
    image: EclImage,
    initial: Box<World>,
    world: Box<World>,
}
fn advance(w: &mut World, image: &EclImage) {
    let f = w.body.frame();
    step(w, &TABLES_V0, image, &InputFrame::empty(f));
}
fn check(w: &World) -> Result<(), String> {
    let view = w.view();
    if view.bullets().iter_alive().count() != 2048
        || view.diag().task_faults != 0
        || view.diag().contract_viol != 0
    {
        return Err(format!(
            "invalid benchmark state: bullets={}, faults={}, violations={}",
            view.bullets().iter_alive().count(),
            view.diag().task_faults,
            view.diag().contract_viol
        ));
    }
    Ok(())
}
fn same_motion(cases: &[Case]) -> Result<(), String> {
    let reference = cases[0].world.view().bullets();
    for case in &cases[1..] {
        let b = case.world.view().bullets();
        for i in reference.iter_alive() {
            if !b.is_alive(i)
                || b.x()[i] != reference.x()[i]
                || b.y()[i] != reference.y()[i]
                || b.polar(i) != reference.polar(i)
            {
                return Err(format!("{} differs at bullet {i}", case.name));
            }
        }
    }
    Ok(())
}
fn make_case(group: &str, name: &'static str, expr: &str) -> Result<Case, String> {
    let target = if name == "constant" {
        "2fx".to_owned()
    } else {
        format!(
            "{}({expr})",
            if group == "spawn" {
                "at_spawn"
            } else {
                "at_effect"
            }
        )
    };
    let body = match group {
        "effect" => format!("@1 set_speed({target});").repeat(16),
        "spawn" => format!("set_speed({target});"),
        _ => format!("step_speed({target},1000);"),
    };
    let source = format!(
        "xformdef T {{ {body} }} sub main() {{ sh_reset(0); sh_offset_abs(0,0fx,40fx); sh_speed(0,{}fx,0fx); sh_count(0,128,16); sh_xform(0,T); sh_fire(0); }}",
        if group == "step" { 0 } else { 2 }
    );
    let image = compile(&source, "xform-bench.ecl").map_err(|e| format!("{e:?}"))?;
    let mut initial = World::new(1);
    initial.body.set_var(0, 1);
    initial.body.set_var(1, 1);
    initial.body.set_var(2, 2);
    initial
        .start_main(&image)
        .map_err(|e| format!("cannot start main: {e:?}"))?;
    advance(&mut initial, &image); // frame 0: task birth gate
    if group != "spawn" {
        advance(&mut initial, &image);
        check(&initial)?;
    }
    let world = World::new(1);
    Ok(Case {
        name,
        image,
        initial,
        world,
    })
}
fn run(frames: u32) -> Result<(), String> {
    println!(
        "xform: release={}, 2048 bullets, 3 interleaved rounds; all times microseconds",
        !cfg!(debug_assertions)
    );
    println!("effect: 2048 entries/frame; spawn: 2048 births/frame; step: 2048 active ticks/frame");
    println!("setup, world restoration and equal-motion validation excluded from timing");
    for group in ["effect", "spawn", "step"] {
        let mut cases = vec![
            make_case(group, "constant", "2fx")?,
            make_case(group, "direct", "global(2) as fx")?,
            make_case(group, "combined", "(global(0)+global(1)) as fx")?,
        ];
        for round in 0..3 {
            let mut times = [Vec::new(), Vec::new(), Vec::new()];
            // Rotate the case order each round to reduce order/temperature bias.
            for frame in 0..frames + 60 {
                for position in 0..3 {
                    let k = (position + round) % 3;
                    let case = &mut cases[k];
                    if group == "spawn" || frame % 15 == 0 {
                        case.initial.copy_into(&mut case.world);
                    }
                    let started = Instant::now();
                    advance(&mut case.world, &case.image);
                    let elapsed = started.elapsed().as_secs_f64() * 1e6;
                    if frame >= 60 {
                        times[k].push(elapsed);
                    }
                    check(&case.world)?;
                }
                if frame == 0 || frame == frames + 59 {
                    same_motion(&cases)?;
                }
            }
            for (case, mut times) in cases.iter().zip(times) {
                times.sort_by(f64::total_cmp);
                let mean = times.iter().sum::<f64>() / times.len() as f64;
                println!(
                    "{group:6} round={} {:8} mean={mean:.3} p50={:.3} p99={:.3}",
                    round + 1,
                    case.name,
                    times[times.len() / 2],
                    times[(times.len() * 99 / 100).min(times.len() - 1)]
                );
                if group == "spawn" || group == "effect" {
                    let b = case.world.view().bullets();
                    if b.iter_alive().any(|i| b.polar(i).0 != Fx::from_int(2)) {
                        return Err("expected speed 2fx".into());
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn cmd_xform_bench(frames: u32) -> ExitCode {
    match run(frames.max(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xform bench failed: {e}");
            ExitCode::FAILURE
        }
    }
}
