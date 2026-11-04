//! Immediate argument staging for program-backed xform references.
use super::Gen;
use crate::SubBuilder;
use crate::lang::slots::{SubSlots, XformRegion};
use crate::lang::typeck::TypedExpr;

impl Gen<'_> {
    pub(super) fn gen_xform_program_ref(
        &mut self,
        b: &mut SubBuilder,
        slots: &SubSlots,
        name: &str,
        exprs: &[TypedExpr],
    ) {
        let XformRegion::ProgramArgs { off, argc } = slots.xform_regions[name] else {
            unreachable!()
        };
        // Keep prior argument values on the VM stack until nested calls finish.
        for e in exprs {
            self.gen_expr(b, slots, e);
        }
        for k in (0..exprs.len()).rev() {
            b.pop_l((off + k) as u8);
        }
        b.push_i(self.xform_program_ids[name] as i32);
        b.push_i(off as i32);
        b.push_i(argc as i32);
    }
}

#[cfg(test)]
mod tests {
    use crate::lang::{compile, compile_for_table};
    use stg_core::input::InputFrame;
    use stg_core::step::{World, step};
    use stg_core::tables::TABLES_V0;

    fn advance(w: &mut World, image: &stg_core::ecl::image::EclImage, count: u32) {
        for _ in 0..count {
            let f = w.body.frame();
            step(w, &TABLES_V0, image, &InputFrame::empty(f));
        }
        assert_eq!(w.view().diag().task_faults, 0);
    }
    fn speeds(w: &World) -> Vec<i32> {
        let b = w.view().bullets();
        b.iter_alive().map(|i| b.polar(i).0.raw()).collect()
    }

    #[test]
    fn xform_rand_samples_each_phase_on_the_shared_stream() {
        let src = r#"
            xformdef T {
                @2 set_speed((rand(8) + 1) as fx);
                @2 set_speed(at_spawn((rand(8) + 1) as fx));
                set_speed(at_effect((rand(8) + 1) as fx));
            }
            sub main() {
                sh_reset(0); sh_count(0, 2, 1);
                sh_xform(0, T);
                set_global(16, rand(1000));
                sh_fire(0);
                set_global(17, rand(1000));
            }
        "#;
        let image = compile(src, "xform-rand-phases.ecl").unwrap();
        let mut w = World::new(17);
        let mut reference = World::new(17);
        let bind = (reference.body.rand_range(8) + 1) as i32 * 65536;
        let before = reference.body.rand_range(1000) as i32;
        let spawn = (0..2)
            .map(|_| (reference.body.rand_range(8) + 1) as i32 * 65536)
            .collect::<Vec<_>>();
        let after = reference.body.rand_range(1000) as i32;
        let effect = (0..2)
            .map(|_| (reference.body.rand_range(8) + 1) as i32 * 65536)
            .collect::<Vec<_>>();
        w.start_main(&image).unwrap();
        advance(&mut w, &image, 2);
        assert_eq!(speeds(&w), vec![bind, bind]);
        assert_eq!(&w.view().globals()[16..18], &[before, after]);
        advance(&mut w, &image, 2);
        assert_eq!(speeds(&w), spawn);
        advance(&mut w, &image, 2);
        assert_eq!(speeds(&w), effect);
        assert_eq!(w.body.rand_range(1000), reference.body.rand_range(1000));
        assert_eq!(w.view().diag().contract_viol, 0);
    }

    #[test]
    fn xform_rand_preserves_call_order_short_circuit_and_nonpositive_bounds() {
        let src = r#"
            xformdef T {
                set_speed(at_effect((rand(8) * 0 + rand(8) + rand(0) + rand(-1)) as fx));
                set_angle(at_effect((rand(65536) - rand(65536)) as angle));
                set_color(at_effect(0 && rand(8)));
                set_shape(at_effect((1 || rand(8)) * 16));
            }
            sub main() { _ = fire(0,0,0fx,40fx,0fx,0deg,T,none); }
        "#;
        let image = compile(src, "xform-rand-order.ecl").unwrap();
        let mut w = World::new(17);
        let mut reference = World::new(17);
        reference.body.rand_range(8); // Multiplication by zero still consumes the first call.
        let speed = reference.body.rand_range(8) as i32 * 65536;
        let left = reference.body.rand_range(65536);
        let right = reference.body.rand_range(65536);
        let angle = left.wrapping_sub(right) as u16;
        w.start_main(&image).unwrap();
        advance(&mut w, &image, 2);
        assert_eq!(speeds(&w), vec![speed]);
        let b = w.view().bullets();
        let i = b.iter_alive().next().unwrap();
        assert_eq!(b.polar(i).1.raw(), angle);
        assert_eq!(b.sprite()[i], 16);
        assert_eq!(w.body.rand_range(1000), reference.body.rand_range(1000));
        assert_eq!(w.view().diag().contract_viol, 0);
    }

    #[test]
    fn xform_rand_step_and_signal_hold_samples_across_restore() {
        let src = r#"
            xformdef T {
                @4 step_speed(at_effect((rand(8) + 1) as fx), 4);
                wait_signal(at_effect(rand(2)));
                set_speed(at_effect((rand(8) + 1) as fx));
            }
            sub main() {
                _ = fire(0,0,0fx,40fx,0fx,0deg,T,none);
                wait(10); pulse_signal(0); pulse_signal(1); wait(100);
            }
        "#;
        let image = compile(src, "xform-rand-hold.ecl").unwrap();
        let mut w = World::new(17);
        let mut reference = World::new(17);
        let target = (reference.body.rand_range(8) + 1) as i32 * 65536;
        reference.body.rand_range(2); // Channel sampled once on WAIT_SIGNAL entry.
        let released = (reference.body.rand_range(8) + 1) as i32 * 65536;
        w.start_main(&image).unwrap();
        advance(&mut w, &image, 2);
        assert_eq!(speeds(&w), vec![0]);
        advance(&mut w, &image, 6);
        assert_eq!(speeds(&w), vec![target]);
        let bytes = w.save_bytes(&image);
        let mut restored = World::load_bytes(&bytes, &TABLES_V0, &image).unwrap();
        let mut copied = World::new(99);
        w.copy_into(&mut copied);
        for state in [&mut w, &mut restored, &mut copied] {
            advance(state, &image, 2);
            assert_eq!(speeds(state), vec![target]);
            advance(state, &image, 4);
            assert_eq!(speeds(state), vec![released]);
        }
        assert_eq!(w.checksum(), restored.checksum());
        assert_eq!(w.checksum(), copied.checksum());
        assert_eq!(w.body.rand_range(1000), reference.body.rand_range(1000));
        assert_eq!(w.view().diag().contract_viol, 0);
    }

    #[test]
    fn xform_scalar_steps_compile_and_run_static_and_dynamic_targets() {
        let src = r#"
            xformdef VELOCITY {
                step_vx(4fx, 4);
                step_vy(-2fx, 4);
            }
            xformdef GRAVITY(amount: fx) {
                step_ax(at_spawn(amount), 4);
                step_ay(at_effect(amount * 2fx), 4);
            }
            xformdef POLAR {
                step_accel(1fx, 4);
                step_ang_vel(-4deg, 4);
            }
            sub main() {
                _ = fire(0,0,0fx,40fx,0fx,0deg,VELOCITY,none);
                _ = fire(0,0,0fx,80fx,0fx,0deg,GRAVITY(0.5fx),none);
                _ = fire(0,0,0fx,120fx,0fx,0deg,POLAR,none);
            }
        "#;
        let image = compile(src, "scalar-steps.ecl").unwrap();
        let mut w = World::new(1);
        w.start_main(&image).unwrap();
        advance(&mut w, &image, 1); // Fire requests are consumed next frame.
        advance(&mut w, &image, 1); // Arm without advancing interpolation.
        let b = w.view().bullets();
        assert_eq!(b.vx()[0], stg_core::math::Fx::ZERO);
        assert_eq!(b.ax()[1], stg_core::math::Fx::ZERO);
        assert_eq!(b.ang_vel()[2], 0);
        advance(&mut w, &image, 2);
        let b = w.view().bullets();
        assert_eq!((b.vx()[0].raw(), b.vy()[0].raw()), (131072, -65536));
        assert_eq!((b.ax()[1].raw(), b.ay()[1].raw()), (16384, 32768));
        assert_eq!(b.accel()[2].raw(), 32768);
        assert_eq!(b.ang_vel()[2], -364);
        advance(&mut w, &image, 2);
        let b = w.view().bullets();
        assert_eq!((b.vx()[0].raw(), b.vy()[0].raw()), (262144, -131072));
        assert_eq!((b.ax()[1].raw(), b.ay()[1].raw()), (32768, 65536));
        assert_eq!(b.accel()[2].raw(), 65536);
        assert_eq!(b.ang_vel()[2], -728);
        assert_eq!(w.view().diag().contract_viol, 0);
    }

    #[test]
    fn xform_two_shooters_capture_independently_and_fire_uses_same_stages() {
        let src = r#"
            xformdef T(extra: fx) {
                @2 set_speed(at_spawn($self_speed + extra));
                set_speed(at_effect($self_speed + extra + (global(20) as fx)));
            }
            async sub report() { set_global(21, $self_speed as int); }
            sub main() {
                var value: fx = 1fx;
                sh_reset(0); sh_reset(1);
                sh_speed(0, 2fx, 0fx); sh_speed(1, 2fx, 0fx);
                sh_xform(0, T(value));
                value = 3fx; sh_xform(1, T(value));
                value = 9fx;
                sh_fire(0); sh_fire(1);
                _ = fire(0, 0, 0fx, 40fx, 2fx, 0deg, T(1fx), report);
                sh_reset(0); sh_xform(1, none);
                set_global(20, 4);
            }
        "#;
        let image = compile(src, "captures.ecl").unwrap();
        assert_eq!(image.xforms().len(), 1);
        let p = image
            .xforms()
            .get(image.xforms().program_id(0).unwrap())
            .unwrap();
        assert_eq!(p.persistent_count(), 1); // extra is used again at effect time
        let mut w = World::new(1);
        w.start_main(&image).unwrap();
        advance(&mut w, &image, 2);
        assert_eq!(speeds(&w), vec![3 * 65536, 5 * 65536, 3 * 65536]);
        advance(&mut w, &image, 2);
        assert_eq!(speeds(&w), vec![8 * 65536, 12 * 65536, 8 * 65536]);
        assert_eq!(w.view().globals()[21], 3); // the new fire ABI still attaches its task

        assert_eq!(w.view().diag().contract_viol, 0);
    }

    #[test]
    fn xform_spawn_only_capture_is_discarded_and_layout_limits_are_explicit() {
        let src = "xformdef T(extra: fx) { set_speed(at_spawn($self_speed + extra)); } sub main() { sh_xform(0,T(0.5fx)); }";
        let image = compile(src, "layout.ecl").unwrap();
        let p = image
            .xforms()
            .get(image.xforms().program_id(0).unwrap())
            .unwrap();
        assert_eq!((p.persistent_count(), p.physical_slots()), (0, 1));
        let source = |n: usize| {
            format!(
                "xformdef T(a: fx,b: fx,c: fx) {{ {} }} sub main() {{ sh_xform(0,T(1fx,2fx,3fx)); }}",
                "set_speed(at_effect(a+b+c));".repeat(n)
            )
        };
        assert!(compile(&source(14), "fits.ecl").is_ok());
        let errors = compile(&source(15), "overflow.ecl").unwrap_err();
        assert!(errors.iter().any(|e| e.msg.contains("17 槽")), "{errors:?}");
        let full = format!(
            "xformdef T {{ {} }} sub main() {{ sh_xform(0,T); }}",
            "set_speed(at_effect($self_speed));".repeat(16)
        );
        assert!(compile(&full, "full.ecl").is_ok());
    }

    #[test]
    fn xform_rejects_wrong_scope_markers_types_and_impure_calls() {
        for body in [
            "set_speed(at_spawn($self_speed) + 1fx);",
            "set_speed(at_effect(at_spawn($self_speed)));",
            "set_speed(self_fx(0));",
            "set_speed(local);",
            "set_speed(at_effect(1));",
        ] {
            let src = format!(
                "xformdef T {{ {body} }} sub main() {{ var local: fx=1fx; sh_xform(0,T); }}"
            );
            let errors = compile(&src, "bad.ecl").unwrap_err();
            assert!(
                errors
                    .iter()
                    .all(|e| e.line > 0 && e.col > 0 && !e.src_line.is_empty())
            );
        }
        let args = (0..9)
            .map(|i| format!("p{i}: fx"))
            .collect::<Vec<_>>()
            .join(",");
        let errors = compile(
            &format!("xformdef T({args}) {{ set_speed(1fx); }} sub main() {{ }}"),
            "cache.ecl",
        )
        .unwrap_err();
        assert!(errors.iter().any(|e| e.msg.contains("8 个逻辑缓存")));
    }

    #[test]
    fn xform_mixed_sprite_axes_and_effect_failure_use_real_runtime_checks() {
        let src = r#"
            xformdef T(shape: int) {
                @2 add_speed(0fx);
                set_sprite(at_spawn(shape), at_effect(global(20)));
            }
            sub main() {
                _ = fire(0,0,0fx,40fx,0fx,0deg,T(16),none);
                set_global(20,3);
            }
        "#;
        let image = compile(src, "sprite.ecl").unwrap();
        let mut w = World::new(1);
        w.start_main(&image).unwrap();
        advance(&mut w, &image, 4);
        let b = w.view().bullets();
        let i = b.iter_alive().next().unwrap();
        assert_eq!(b.sprite()[i], 19);
        let mut table = stg_core::tables::build_tables_v0();
        let mut appearances = table.appearances.to_vec();
        appearances[19].valid = false;
        table.appearances = appearances.into_boxed_slice();
        let image = compile_for_table(src, "hole.ecl", &table).unwrap();
        let mut w = World::new_with_tables(1, &table);
        w.start_main(&image).unwrap();
        for f in 0..4 {
            step(&mut w, &table, &image, &InputFrame::empty(f));
        }
        assert_eq!(w.view().diag().contract_viol, 1);
        let b = w.view().bullets();
        let i = b.iter_alive().next().unwrap();
        assert_eq!(b.sprite()[i], 0);
        assert_eq!(b.xform_next()[i], 16);
    }

    #[test]
    fn xform_spawn_and_effect_observe_player_movement_in_their_own_phases() {
        let src = r#"
            xformdef T {
                set_speed(at_spawn($player_x));
                set_accel(at_effect($player_x));
            }
            sub main() { _ = fire(0,0,0fx,40fx,0fx,0deg,T,none); }
        "#;
        let image = compile(src, "phase.ecl").unwrap();
        let mut w = World::new(1);
        w.start_main(&image).unwrap();
        advance(&mut w, &image, 1);
        let before = w.view().players()[0].x;
        let mut input = InputFrame::empty(w.body.frame());
        input.actions[0].buttons = stg_core::input::BTN_RIGHT;
        step(&mut w, &TABLES_V0, &image, &input);
        let after = w.view().players()[0].x;
        assert!(after > before);
        let b = w.view().bullets();
        let i = b.iter_alive().next().unwrap();
        assert_eq!(b.accel()[i], after);
        // Integration applies this frame's acceleration after the spawn-sampled target.
        assert_eq!(b.polar(i).0, before + after);
    }
    #[test]
    fn xform_nested_same_template_does_not_overwrite_outer_arguments() {
        let src = "xformdef T(a: int, b: int) { set_speed(a as fx); } sub main() { sh_reset(0); sh_xform(0,T(1,fire(0,0,0fx,40fx,0fx,0deg,T(2,3),none))); sh_fire(0); }";
        let image = compile(src, "nested.ecl").unwrap();
        let mut w = World::new(1);
        w.start_main(&image).unwrap();
        advance(&mut w, &image, 3);
        assert_eq!(speeds(&w), vec![2 * 65536, 65536]);
    }

    #[test]
    fn xform_mixed_sprite_rejects_each_known_invalid_axis_at_compile_time() {
        for body in [
            "set_sprite(1,at_effect(global(20)));",
            "set_sprite(at_effect(global(20)),99);",
        ] {
            let src = format!("xformdef T {{ {body} }} sub main() {{ sh_xform(0,T); }}");
            let errors = compile(&src, "bad-axis.ecl").unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|e| e.msg.contains("弹型") || e.msg.contains("色号")),
                "{errors:?}"
            );
        }
    }
}
