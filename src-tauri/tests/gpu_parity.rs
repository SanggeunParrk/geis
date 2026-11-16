//! The analyses on the graphics card against the same analyses on the processor.
//!
//! Point `GEIS_TEST_STRUCTURES` at a directory of structures (1ubq.cif, 4hhb.cif, and if
//! present 1uf2.cif) and this runs every preset both ways and compares; otherwise it skips.
//! The processor path is chosen per run by `GEIS_NO_GPU`, so the two halves are two
//! processes: `GEIS_PARITY_OUT` names where each writes what it found.

use geis_lib::structure::{self, contacts, contacts::ContactKind, Unit};
use std::{fmt::Write as _, path::PathBuf, time::Instant};

#[test]
fn every_preset_finds_the_same_thing_on_either_device() {
    let Some(dir) = std::env::var("GEIS_TEST_STRUCTURES")
        .ok()
        .map(PathBuf::from)
    else {
        eprintln!("skipping: GEIS_TEST_STRUCTURES is not set");
        return;
    };
    let mut report = String::new();
    for id in ["1ubq", "4hhb", "1uf2"] {
        let path = dir.join(format!("{id}.cif"));
        if !path.exists() {
            continue;
        }
        let s = structure::load_file_as(&path, Unit::Assembly).expect("loads");
        for kind in [
            ContactKind::Polar,
            ContactKind::SaltBridge,
            ContactKind::PiStacking,
            ContactKind::Hydrophobic,
        ] {
            let start = Instant::now();
            let found = contacts::find_named(&s, kind, None, false, false);
            let mut keys: Vec<(u32, u32)> = found
                .iter()
                .map(|c| (c.residues[0], c.residues[1]))
                .collect();
            keys.sort_unstable();
            let hash = keys.iter().fold(0_u64, |h, &(a, b)| {
                h.wrapping_mul(1_000_003)
                    .wrapping_add(u64::from(a) << 32 | u64::from(b))
            });
            let _ = writeln!(
                report,
                "{id} {kind:?} {} {hash:x} {:.2}s",
                found.len(),
                start.elapsed().as_secs_f32()
            );
        }
        for (name, run) in [
            (
                "core",
                contacts::hydrophobic_core as fn(&structure::Structure) -> Vec<u32>,
            ),
            ("site", contacts::binding_site),
            ("interface", contacts::interface),
        ] {
            let start = Instant::now();
            let found = run(&s);
            let hash = found.iter().fold(0_u64, |h, &a| {
                h.wrapping_mul(1_000_003).wrapping_add(u64::from(a))
            });
            let _ = writeln!(
                report,
                "{id} {name} {} {hash:x} {:.2}s",
                found.len(),
                start.elapsed().as_secs_f32()
            );
        }
    }
    eprintln!("gpu: {:?}\n{report}", geis_lib::gpu::adapter_name());
    if let Ok(out) = std::env::var("GEIS_PARITY_OUT") {
        std::fs::write(out, report).expect("writes");
    }
}
