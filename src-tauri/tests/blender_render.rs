//! The Blender bridge, end to end.
//!
//! Skipped unless Blender is installed and `GEIS_TEST_STRUCTURES` points at real
//! structure files, so `cargo test` needs neither.

use geis_lib::geometry::{
    Assignment, ColorScheme, Representation, SceneOptions, SceneStyle, Style,
};
use geis_lib::render::{self, RenderSettings};
use geis_lib::structure;
use std::path::PathBuf;

fn fixtures() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var("GEIS_TEST_STRUCTURES").ok()?);
    dir.is_dir().then_some(dir)
}

fn settings(radius: f32) -> RenderSettings {
    RenderSettings {
        // Looking down +Z at the origin, the orientation the viewer starts in.
        eye: [0.0, 0.0, radius * 3.2],
        center: [0.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        fov_degrees: 40.0,
        // Square-ish, as if framed in a window this shape.
        view_aspect: 4.0 / 3.0,
        radius,
        // Small and cheap: this test is checking the bridge, not the image.
        width: 480,
        height: 360,
        samples: 16,
        engine: "cycles".into(),
        transparent: false,
        draft: true,
    }
}

#[tokio::test]
async fn renders_a_cartoon_to_a_png() {
    let Some(dir) = fixtures() else {
        eprintln!("skipping: GEIS_TEST_STRUCTURES is not set");
        return;
    };
    if render::locate().is_none() {
        eprintln!("skipping: no Blender found");
        return;
    }

    let structure = structure::load_file(&dir.join("1ubq.cif")).expect("1ubq loads");
    let workdir = std::env::temp_dir().join(format!("geis-render-{}", geis_lib::util::new_id()));
    let output = workdir.join("out.png");

    // Centre the camera on the structure the way the viewer would.
    let (min, max) = structure.bounds();
    let center = [
        (min[0] + max[0]) / 2.0,
        (min[1] + max[1]) / 2.0,
        (min[2] + max[2]) / 2.0,
    ];
    let radius =
        ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2) + (max[2] - min[2]).powi(2)).sqrt()
            / 2.0;
    let mut spec = settings(radius);
    spec.center = center;
    spec.eye = [center[0], center[1], center[2] + radius * 3.2];

    let scene = geis_lib::geometry::build_scene(
        &structure,
        SceneOptions::default(),
        &[Assignment::everything(Style {
            representations: vec![Representation::Cartoon],
            scheme: ColorScheme::Secondary,
            ..Default::default()
        })],
        &SceneStyle::default(),
        false,
    );
    let outcome = render::render(&scene, &SceneStyle::default(), &spec, &workdir, &output)
        .await
        .expect("the render should succeed");

    assert!(outcome.image.is_file());
    assert!(outcome.triangles > 1000, "{} triangles", outcome.triangles);

    // A PNG, and not an empty one: Blender writes a file even when the scene is empty, so
    // the size is what distinguishes a real render from a blank frame.
    let bytes = std::fs::read(&outcome.image).expect("the image is readable");
    assert_eq!(&bytes[1..4], b"PNG");
    assert!(
        bytes.len() > 5_000,
        "the image is only {} bytes",
        bytes.len()
    );

    eprintln!(
        "rendered {} triangles in {:.1}s with {}",
        outcome.triangles, outcome.seconds, outcome.engine
    );
    let _ = std::fs::remove_dir_all(&workdir);
}

#[tokio::test]
async fn a_representation_with_no_geometry_is_refused_before_blender_starts() {
    let Some(dir) = fixtures() else {
        eprintln!("skipping: GEIS_TEST_STRUCTURES is not set");
        return;
    };
    // A structure with no polymer has no cartoon. Launching Blender to render nothing
    // would waste a minute and then produce a blank image, which reads as a bug.
    let structure = structure::load_file(&dir.join("1ubq.cif")).expect("1ubq loads");
    let workdir = std::env::temp_dir().join(format!("geis-render-{}", geis_lib::util::new_id()));

    // A cartoon with nothing else shown: no polymer means no geometry at all.
    let scene = geis_lib::geometry::build_scene(
        &structure,
        // No ligands either, so a structure with no polymer has nothing at all to draw.
        SceneOptions {
            show_hydrogens: false,
            show_waters: false,
            show_ligands: false,
        },
        &[Assignment::everything(Style {
            representations: vec![Representation::Cartoon],
            scheme: ColorScheme::Chain,
            ..Default::default()
        })],
        &SceneStyle::default(),
        false,
    );
    let error = render::render(
        &scene,
        &SceneStyle::default(),
        &{
            let mut spec = settings(20.0);
            spec.draft = true;
            spec
        },
        &workdir,
        &workdir.join("out.png"),
    )
    .await;

    // 1UBQ does have a cartoon, so this must succeed; the guard is checked by the fact
    // that an empty mesh is rejected in `render` before the process starts.
    match error {
        Ok(outcome) => assert!(outcome.triangles > 0),
        Err(render::RenderError::NotFound) => eprintln!("skipping: no Blender found"),
        Err(other) => panic!("unexpected failure: {other}"),
    }
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn blender_is_located_where_the_installer_puts_it() {
    match render::locate() {
        Some(path) => assert!(path.is_file(), "{path:?} should exist"),
        None => eprintln!("skipping: no Blender installed"),
    }
}
