//! How long a capsid's scene takes to build with every chain its own colour - the case
//! that used to build a colour array and a ribbon pass per chain. Runs when
//! `GEIS_TEST_STRUCTURES` holds 1uf2.cif; otherwise skips.

use geis_lib::geometry::representation::{build_scene, Assignment, SceneOptions, Style};
use geis_lib::geometry::style::SceneStyle;
use geis_lib::structure::{self, Unit};
use std::{path::PathBuf, time::Instant};

#[test]
fn a_capsid_coloured_chain_by_chain_builds_in_reasonable_time() {
    let Some(dir) = std::env::var("GEIS_TEST_STRUCTURES")
        .ok()
        .map(PathBuf::from)
    else {
        eprintln!("skipping: GEIS_TEST_STRUCTURES is not set");
        return;
    };
    let path = dir.join("1uf2.cif");
    if !path.exists() {
        return;
    }
    let s = structure::load_file_as(&path, Unit::Assembly).expect("loads");
    let assignments: Vec<Assignment> = s
        .chains
        .iter()
        .enumerate()
        .map(|(index, chain)| {
            let hue = index as f32 / s.chains.len() as f32;
            Assignment {
                chain: Some(chain.id.clone()),
                style: Style {
                    solid: Some([hue, 1.0 - hue, 0.5]),
                    ..Style::default()
                },
            }
        })
        .collect();
    for coarse in [true, false] {
        let start = Instant::now();
        let scene = build_scene(
            &s,
            SceneOptions::default(),
            &assignments,
            &SceneStyle::default(),
            coarse,
        );
        eprintln!(
            "{} chains, coarse {coarse}: {:.2}s, {} triangles",
            s.chains.len(),
            start.elapsed().as_secs_f32(),
            scene.mesh.triangle_count()
        );
    }
}
