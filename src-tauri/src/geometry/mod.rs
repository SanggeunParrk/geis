//! Turning a structure into drawable geometry.
//!
//! This lives in Rust rather than in the renderer because two consumers need the same
//! answer: the interactive viewer, and the offline Blender renderer. A cartoon built in
//! the webview cannot be handed to a subprocess, and reimplementing it in the render
//! script would give two definitions of what a ribbon looks like - which drift, and the
//! day they do, a world thumbnail no longer matches the viewer it was made from.
//!
//! Three kinds of geometry come out, because they are drawn three different ways:
//!
//!   `SphereSet` / `CylinderSet` - instance lists. The viewer ray-casts impostors against
//!                                 them; Blender instances real primitives at the same
//!                                 transforms.
//!   `Mesh`                      - actual triangles, for cartoon ribbons. The only one of
//!                                 the three that has to be tessellated.
//!
//! Colour lives here too, for the same reason: the AlphaFold confidence bands must mean
//! the same thing in the viewport and in a rendered figure.

pub mod cartoon;
pub mod color;
pub mod envelope;
pub mod gltf;
pub mod mesh;
pub mod palette;
pub mod preset;
pub mod representation;
pub mod style;
pub mod tessellate;

pub use color::{ColorScheme, LegendEntry};
pub use mesh::{CylinderSet, Mesh, Scene, SphereSet};
pub use palette::{Categorical, SwatchGroup};
pub use preset::{named_colours, presets, NamedColour, StylePreset};
pub use representation::{
    build_scene, build_scene_with, Assignment, Feature, Representation, SceneOptions, Style,
};
pub use style::{Light, LightKind, Material, SceneStyle};
pub use tessellate::{tessellate, Quality};
