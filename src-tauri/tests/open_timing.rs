//! Where the time goes in opening a capsid, stage by stage. Runs when
//! `GEIS_TEST_STRUCTURES` holds 1uf2.cif; otherwise skips. For finding work still done on
//! one core: each stage prints its wall time beside the processor time it used.

use geis_lib::geometry::representation::{
    build_scene, Assignment, Representation, SceneOptions, Style,
};
use geis_lib::geometry::style::SceneStyle;
use geis_lib::structure::{self, Unit};
use std::{path::PathBuf, time::Instant};

fn cpu() -> f64 {
    // The process's processor time so far, user and system, from `ps`-free sources: the
    // Instant of a thread is not it, so ask the kernel through /usr/bin/time's own call.
    let mut usage = std::mem::MaybeUninit::<[i64; 18]>::zeroed();
    extern "C" {
        fn getrusage(who: i32, usage: *mut [i64; 18]) -> i32;
    }
    unsafe {
        getrusage(0, usage.as_mut_ptr());
        let u = usage.assume_init();
        (u[0] as f64 + u[1] as f64 / 1e6) + (u[2] as f64 + u[3] as f64 / 1e6)
    }
}

fn stage<T>(name: &str, work: impl FnOnce() -> T) -> T {
    let (wall, before) = (Instant::now(), cpu());
    let value = work();
    let (wall, used) = (wall.elapsed().as_secs_f64(), cpu() - before);
    eprintln!(
        "{name:<34} {wall:>6.2}s wall {used:>6.2}s cpu  x{:.1}",
        used / wall.max(1e-6)
    );
    value
}

#[test]
fn opening_a_capsid_stage_by_stage() {
    let Some(dir) = std::env::var("GEIS_TEST_STRUCTURES")
        .ok()
        .map(PathBuf::from)
    else {
        return;
    };
    let path = dir.join("1uf2.cif");
    if !path.exists() {
        return;
    }
    stage("read deposited", || {
        structure::load_file_as(&path, Unit::Deposited).unwrap()
    });
    let s = stage("read assembly", || {
        structure::load_file_as(&path, Unit::Assembly).unwrap()
    });
    let cartoon = vec![Assignment::everything(Style::default())];
    let both = vec![Assignment::everything(Style {
        representations: vec![Representation::Cartoon, Representation::Sticks],
        ..Style::default()
    })];
    let spheres = vec![Assignment::everything(Style {
        representations: vec![Representation::Spacefill],
        ..Style::default()
    })];
    for (name, assignments, coarse) in [
        ("scene cartoon coarse", &cartoon, true),
        ("scene cartoon", &cartoon, false),
        ("scene cartoon+sticks", &both, false),
        ("scene spacefill", &spheres, false),
    ] {
        let mut scene = stage(name, || {
            build_scene(
                &s,
                SceneOptions::default(),
                assignments,
                &SceneStyle::default(),
                coarse,
            )
        });
        stage("  settle", || scene.settle());
        let bytes = stage("  encode", || scene.encode());
        eprintln!("  {} MB", bytes.len() >> 20);
    }
}

#[test]
fn envelopes_of_a_protein_and_of_a_capsid() {
    let Some(dir) = std::env::var("GEIS_TEST_STRUCTURES")
        .ok()
        .map(PathBuf::from)
    else {
        return;
    };
    for (id, unit) in [("4hhb", Unit::Deposited), ("1uf2", Unit::Assembly)] {
        let path = dir.join(format!("{id}.cif"));
        if !path.exists() {
            continue;
        }
        let s = structure::load_file_as(&path, unit).unwrap();
        let envelope = stage(&format!("envelope {id}"), || {
            let atoms: Vec<usize> = (0..s.atom_count()).collect();
            geis_lib::geometry::envelope::envelope_of(&s, &atoms, &|_| [0.8; 3])
        });
        eprintln!(
            "  {} vertices, {} triangles",
            envelope.positions.len() / 3,
            envelope.indices.len() / 3
        );
    }
}
