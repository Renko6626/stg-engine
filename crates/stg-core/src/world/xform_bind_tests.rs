use super::*;
use crate::ecl::image::{EclImage, ImageParts, SubInit, SubKind};
use crate::input::InputFrame;
use crate::math::{Angle, Fx};
use crate::step::{World, step};
use crate::tables::TABLES_V0;
use crate::xform::expr::{XformBinary, XformExpr, XformInstr, XformRead, XformUnary};

fn image(init: XformProgramInit) -> EclImage {
    EclImage::try_from_parts(ImageParts {
        xforms: XformPrograms::try_new(vec![init]).unwrap(),
        code: vec![0],
        subs: vec![SubInit::new(0, SubKind::Root, vec![])],
        entries: vec![],
        root: Some(0),
        marks: vec![],
        content_hash: TABLES_V0.content_hash,
    })
    .unwrap()
}
fn slot(op: u8, wait: u16, args: [i32; 2]) -> XformSlot {
    XformSlot {
        op,
        wait,
        args,
        _pad: 0,
    }
}
fn init() -> BulletInit {
    BulletInit {
        x: Fx::ZERO,
        y: Fx::from_int(40),
        vx: Fx::ZERO,
        vy: Fx::ZERO,
        speed: Fx::ZERO,
        angle: Angle(0),
        ang_vel: 0,
        accel: Fx::ZERO,
        ax: Fx::ZERO,
        ay: Fx::ZERO,
        sprite: 0,
        radius: Fx::ZERO,
        delay: 0,
        life: 65535,
        flags: 0,
        grazed_by: 0,
        transform_head: XFORM_NONE,
        xform_wait: 0,
        xform_next: 0,
        born_frame: 0,
    }
}
fn bind(w: &mut World, image: &EclImage, args: &[i32]) -> XformBinding {
    w.body
        .bind_xform(
            image.xforms(),
            image.xforms().program_id(0).unwrap(),
            args,
            XformEvalOwner::Stage { born_frame: 0 },
            &TABLES_V0,
        )
        .unwrap()
}
fn tick(w: &mut World, image: &EclImage) {
    let f = w.body.frame;
    step(w, &TABLES_V0, image, &InputFrame::empty(f));
}
fn program(slots: Vec<XformSlot>, params: Vec<XformOpParams>) -> XformProgramInit {
    XformProgramInit {
        slots,
        argc: 0,
        bind: vec![],
        spawn: vec![],
        params,
        persistent: vec![],
    }
}

#[test]
fn xform_samples_bind_spawn_and_effect_at_distinct_times() {
    let mut p = program(
        vec![
            slot(OP_SET_SPEED, 2, [0, 0]),
            slot(OP_SET_SPEED, 2, [0, 0]),
            slot(OP_SET_SPEED, 0, [0, 0]),
        ],
        vec![
            XformOpParams {
                slot: 0,
                args: vec![XformParam::Cached(0)],
            },
            XformOpParams {
                slot: 1,
                args: vec![XformParam::Cached(1)],
            },
            XformOpParams {
                slot: 2,
                args: vec![XformParam::Effect(XformExpr::Read(XformRead::Global(20)))],
            },
        ],
    );
    p.bind.push(XformSample {
        index: 0,
        expr: XformExpr::Read(XformRead::Global(20)),
    });
    p.spawn.push(XformSample {
        index: 1,
        expr: XformExpr::Read(XformRead::Global(20)),
    });
    let image = image(p);
    let mut w = World::new(1);
    w.body.set_var(20, 65536);
    let b = bind(&mut w, &image, &[]);
    w.body.set_var(20, 2 * 65536);
    let h = w
        .body
        .create_bullet_with_bound_xform(init(), &b, image.xforms(), &TABLES_V0);
    assert_ne!(h, BulletHandle::NULL);
    w.body.set_var(20, 3 * 65536);
    tick(&mut w, &image);
    assert_eq!(w.body.bullets.speed[h.index as usize], Fx::from_int(1));
    tick(&mut w, &image);
    tick(&mut w, &image);
    assert_eq!(w.body.bullets.speed[h.index as usize], Fx::from_int(2));
    tick(&mut w, &image);
    tick(&mut w, &image);
    assert_eq!(w.body.bullets.speed[h.index as usize], Fx::from_int(3));
}

#[test]
fn xform_step_reentry_resamples_without_resetting_loop_count() {
    let mut p = program(
        vec![
            slot(OP_STEP_SPEED, 2, [0, 2]),
            XformSlot::default(),
            slot(OP_LOOP, 0, [0, 2]),
        ],
        vec![XformOpParams {
            slot: 0,
            args: vec![
                XformParam::Effect(
                    XformExpr::try_code(vec![
                        XformInstr::Read(XformRead::Global(20)),
                        XformInstr::Constant(1),
                        XformInstr::Unary(XformUnary::Rand),
                        XformInstr::Binary(XformBinary::Add),
                    ])
                    .unwrap(),
                ),
                XformParam::Constant(2),
            ],
        }],
    );
    p.argc = 1;
    p.persistent = vec![0];
    let image = image(p);
    let mut w = World::new(1);
    w.body.set_var(20, 4 * 65536);
    let b = bind(&mut w, &image, &[123]);
    let h = w
        .body
        .create_bullet_with_bound_xform(init(), &b, image.xforms(), &TABLES_V0);
    let i = h.index as usize;
    let seg = w.body.bullets.transform_head[i];
    tick(&mut w, &image);
    w.body.set_var(20, 8 * 65536);
    tick(&mut w, &image);
    tick(&mut w, &image);
    assert_eq!(w.body.bullets.speed[i], Fx::from_int(4));
    assert_eq!(w.body.xforms.seg_slots(seg)[2].args[1], 1);
    tick(&mut w, &image);
    tick(&mut w, &image);
    tick(&mut w, &image);
    assert_eq!(w.body.bullets.speed[i], Fx::from_int(8));
    assert_eq!(w.body.bullets.xform_next[i], 16);
    assert_eq!(w.body.xforms.seg_slots(seg)[3].args[0], 123);
    assert_eq!(w.body.diag.contract_viol, 0);
    let mut reference = World::new(1);
    reference.body.rand_range(1); // One draw on each STEP entry, none during interpolation.
    reference.body.rand_range(1);
    assert_eq!(w.body.rand_range(1000), reference.body.rand_range(1000));
}

#[test]
fn xform_wait_signal_sampling_and_persistent_data_survive_restore() {
    let mut p = program(
        vec![
            slot(OP_WAIT_SIGNAL, 0, [0, 0]),
            slot(OP_SET_SPEED, 0, [0, 0]),
            slot(OP_LOOP, 0, [0, 2]),
        ],
        vec![
            XformOpParams {
                slot: 0,
                args: vec![XformParam::Effect(XformExpr::Read(XformRead::Global(20)))],
            },
            XformOpParams {
                slot: 1,
                args: vec![XformParam::Effect(XformExpr::Read(XformRead::Capture(0)))],
            },
        ],
    );
    p.argc = 1;
    p.persistent = vec![0];
    let image = image(p);
    let mut w = World::new(1);
    let b = bind(&mut w, &image, &[3 * 65536]);
    let h = w
        .body
        .create_bullet_with_bound_xform(init(), &b, image.xforms(), &TABLES_V0);
    tick(&mut w, &image);
    w.body.set_var(20, 1);
    let bytes = w.save_bytes(&image);
    let mut restored = World::load_bytes(&bytes, &TABLES_V0, &image).unwrap();
    let mut copied = World::new(9);
    w.copy_into(&mut copied);
    for (n, ch) in [1, 0, 0, 1].into_iter().enumerate() {
        for state in [&mut w, &mut restored, &mut copied] {
            crate::step::step_with_director(
                state,
                &TABLES_V0,
                &image,
                &InputFrame::empty(state.body.frame),
                |body| body.pulse_signal(ch),
            );
        }
        assert_eq!(w.checksum(), restored.checksum());
        assert_eq!(w.checksum(), copied.checksum());
        assert_eq!(
            w.body.bullets.speed[h.index as usize],
            if n == 0 { Fx::ZERO } else { Fx::from_int(3) }
        );
        assert_eq!(
            w.body.bullets.xform_next[h.index as usize],
            if n == 3 { 16 } else { 0 }
        );
    }
    // Reusing this segment through the static entry clears program metadata and payload.
    let seg = w.body.bullets.transform_head[h.index as usize];
    w.body.bullets.free(h);
    w.body.xforms.free(seg);
    let fresh = w
        .body
        .create_bullet_with_xform(init(), &[slot(OP_SET_SPEED, 0, [65536, 0])]);
    assert_eq!(w.body.bullets.transform_head[fresh.index as usize], seg);
    assert_eq!(
        w.body.xforms.program_states[seg as usize],
        XformSegmentState::default()
    );
    assert_eq!(w.body.xforms.seg_slots(seg)[2], XformSlot::default());
}

#[test]
fn xform_program_rejects_data_jumps_and_unavailable_captures() {
    let mut p = program(vec![slot(OP_LOOP, 0, [2, 0])], vec![]);
    p.argc = 3;
    p.persistent = vec![0, 1, 2]; // data starts at 1; 2 is payload, not a boundary
    assert!(XformPrograms::try_new(vec![p]).is_err());
    let p = program(
        vec![slot(OP_SET_SPEED, 0, [0, 0])],
        vec![XformOpParams {
            slot: 0,
            args: vec![XformParam::Effect(XformExpr::Read(XformRead::Capture(0)))],
        }],
    );
    assert!(XformPrograms::try_new(vec![p]).is_err());
}
