//! Turning instance lists into triangles.
//!
//! The viewer never needs this: it ray-casts spheres in a shader and builds cylinders from
//! one instanced prism. Blender does need it - a renderer outside our process cannot run
//! our fragment shader, and it has no notion of an analytic sphere either.
//!
//! Doing it here rather than in the render script keeps per-atom radius and colour, which
//! is what instancing a single primitive in Blender would throw away, and leaves the
//! render script with one job: load a mesh and light it.

use super::mesh::{CylinderSet, Mesh, Scene, SphereSet};

/// How finely spheres and cylinders are divided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// For thumbnails and large assemblies.
    Draft,
    /// For figures.
    Fine,
}

impl Quality {
    /// Rings and segments of the sphere's latitude/longitude grid.
    fn sphere_grid(self) -> (usize, usize) {
        match self {
            Quality::Draft => (8, 12),
            Quality::Fine => (16, 24),
        }
    }

    fn cylinder_segments(self) -> usize {
        match self {
            Quality::Draft => 8,
            Quality::Fine => 16,
        }
    }
}

/// Triangle budget above which quality is reduced automatically.
///
/// A spacefill render of a large assembly at fine quality is tens of millions of
/// triangles, which is minutes of Blender doing nothing but building a mesh. Dropping to
/// draft is far better than appearing to hang.
const TRIANGLE_BUDGET: usize = 4_000_000;

/// Merges everything in a scene into one triangle mesh.
///
/// One mesh rather than three: the render script then has a single object to place, and
/// vertex colours carry the whole scene's colouring with no material per representation.
pub fn tessellate(scene: &Scene, quality: Quality) -> Mesh {
    let quality = downgrade_if_needed(scene, quality);
    let mut mesh = Mesh {
        // The cartoon is already triangles; start from it and append the rest.
        positions: scene.mesh.positions.clone(),
        normals: scene.mesh.normals.clone(),
        colors: scene.mesh.colors.clone(),
        indices: scene.mesh.indices.clone(),
        vertex_residue: scene.mesh.vertex_residue.clone(),
    };

    add_spheres(&mut mesh, &scene.spheres, quality);
    add_cylinders(&mut mesh, &scene.cylinders, quality);
    // An external renderer treats winding as the front face, so it has to be right here
    // even though our own viewport does not care.
    mesh.orient_faces();
    mesh
}

fn downgrade_if_needed(scene: &Scene, quality: Quality) -> Quality {
    if quality == Quality::Draft {
        return quality;
    }
    let (rings, segments) = quality.sphere_grid();
    let per_sphere = rings * segments * 2;
    // Each bond is two prisms, one per half in its own atom's colour, and each prism is
    // two triangles a side.
    let per_cylinder = quality.cylinder_segments() * 4;
    let estimate = scene.mesh.triangle_count()
        + scene.spheres.len() * per_sphere
        + scene.cylinders.len() * per_cylinder;
    if estimate > TRIANGLE_BUDGET {
        Quality::Draft
    } else {
        quality
    }
}

/// A latitude/longitude sphere per instance, with radial normals so it shades smoothly.
fn add_spheres(mesh: &mut Mesh, spheres: &SphereSet, quality: Quality) {
    let (rings, segments) = quality.sphere_grid();
    for sphere in 0..spheres.len() {
        let base = sphere * 4;
        let center = [
            spheres.instances[base],
            spheres.instances[base + 1],
            spheres.instances[base + 2],
        ];
        let radius = spheres.instances[base + 3];
        let color = [
            spheres.colors[sphere * 3],
            spheres.colors[sphere * 3 + 1],
            spheres.colors[sphere * 3 + 2],
        ];
        let atom = spheres.atoms[sphere];

        let first = mesh.vertex_count() as u32;
        for ring in 0..=rings {
            let phi = std::f32::consts::PI * ring as f32 / rings as f32;
            let (sin_phi, cos_phi) = phi.sin_cos();
            for segment in 0..=segments {
                let theta = std::f32::consts::TAU * segment as f32 / segments as f32;
                let (sin_theta, cos_theta) = theta.sin_cos();
                let normal = [sin_phi * cos_theta, cos_phi, sin_phi * sin_theta];
                mesh.push_vertex(
                    [
                        center[0] + normal[0] * radius,
                        center[1] + normal[1] * radius,
                        center[2] + normal[2] * radius,
                    ],
                    normal,
                    color,
                    atom,
                );
            }
        }

        let stride = segments as u32 + 1;
        for ring in 0..rings as u32 {
            for segment in 0..segments as u32 {
                let a = first + ring * stride + segment;
                let b = a + 1;
                let c = a + stride;
                let d = c + 1;
                // The poles collapse to a single point, so one triangle of each quad there
                // is degenerate. Skipping them keeps the mesh free of zero-area faces,
                // which Blender reports as errors on validate.
                if ring > 0 {
                    mesh.push_triangle(a, c, b);
                }
                if ring + 1 < rings as u32 {
                    mesh.push_triangle(b, c, d);
                }
            }
        }
    }
}

/// A prism per bond, uncapped: the spheres at the atom positions cover the joints.
fn add_cylinders(mesh: &mut Mesh, cylinders: &CylinderSet, quality: Quality) {
    let segments = quality.cylinder_segments();

    for cylinder in 0..cylinders.len() {
        let base = cylinder * super::mesh::CYLINDER_STRIDE;
        let radius = cylinders.radius(cylinder);
        let start = [
            cylinders.instances[base],
            cylinders.instances[base + 1],
            cylinders.instances[base + 2],
        ];
        let end = [
            cylinders.instances[base + 3],
            cylinders.instances[base + 4],
            cylinders.instances[base + 5],
        ];
        // Colours are still six floats per bond; only the instance stride grew when the
        // radius joined it. Indexing them with the instance stride read past the end of the
        // array on the last bond, and took the Blender render down with it.
        let color_base = cylinder * 6;
        let color_start = [
            cylinders.colors[color_base],
            cylinders.colors[color_base + 1],
            cylinders.colors[color_base + 2],
        ];
        let color_end = [
            cylinders.colors[color_base + 3],
            cylinders.colors[color_base + 4],
            cylinders.colors[color_base + 5],
        ];
        let atom = cylinders.atoms[cylinder * 2];

        let axis = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
        let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
        if length < 1e-6 {
            continue;
        }
        let w = [axis[0] / length, axis[1] / length, axis[2] / length];
        // Choosing the helper by which component is smallest avoids the degenerate cross
        // product that would collapse a bond aligned with an axis.
        let helper = if w[1].abs() < 0.99 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let u = normalize(cross(helper, w));
        let v = cross(w, u);

        // Two prisms, one per half, each flat in its atom's colour. Blender interpolates
        // vertex colours across a face, so a single prism with a colour at each end is a
        // gradient - the same mistake the viewport made, fixed the same way: the colour
        // changes at the middle, and only there.
        let middle = [
            (start[0] + end[0]) * 0.5,
            (start[1] + end[1]) * 0.5,
            (start[2] + end[2]) * 0.5,
        ];
        // The prism circumscribes the stick's circle rather than inscribing it, so its flats
        // sit on the radius and the sphere at each joint - a round mesh of its own - does
        // not show through them as a sliver. The viewport does the same; see cylinders.ts.
        let circumscribed = radius / (std::f32::consts::PI / segments as f32).cos();
        for (from, to, color) in [(start, middle, color_start), (middle, end, color_end)] {
            let first = mesh.vertex_count() as u32;
            for segment in 0..=segments {
                let angle = std::f32::consts::TAU * segment as f32 / segments as f32;
                let (sin, cos) = angle.sin_cos();
                let normal = [
                    u[0] * cos + v[0] * sin,
                    u[1] * cos + v[1] * sin,
                    u[2] * cos + v[2] * sin,
                ];
                for origin in [from, to] {
                    mesh.push_vertex(
                        [
                            origin[0] + normal[0] * circumscribed,
                            origin[1] + normal[1] * circumscribed,
                            origin[2] + normal[2] * circumscribed,
                        ],
                        normal,
                        color,
                        atom,
                    );
                }
            }
            for segment in 0..segments as u32 {
                let a = first + segment * 2;
                let b = a + 1;
                let c = a + 2;
                let d = a + 3;
                mesh.push_triangle(a, c, b);
                mesh.push_triangle(b, c, d);
            }
        }
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let length = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if length < 1e-8 {
        [0.0, 0.0, 1.0]
    } else {
        [a[0] / length, a[1] / length, a[2] / length]
    }
}

/// The mesh as a standalone binary file for the render script.
///
/// Its own small format rather than glTF or OBJ: what needs to cross is four arrays, and a
/// format with materials, scene graphs, and a JSON envelope would be more code on both
/// sides for no gain. Python reads this with `numpy.frombuffer` in three lines.
pub const MESH_FILE_MAGIC: &[u8; 4] = b"MMSH";
pub const MESH_FILE_VERSION: u32 = 1;

pub fn encode_mesh_file(mesh: &Mesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + mesh.positions.len() * 12 + mesh.indices.len() * 4);
    out.extend_from_slice(MESH_FILE_MAGIC);
    out.extend_from_slice(&MESH_FILE_VERSION.to_le_bytes());
    out.extend_from_slice(&(mesh.vertex_count() as u32).to_le_bytes());
    out.extend_from_slice(&(mesh.triangle_count() as u32).to_le_bytes());
    for value in &mesh.positions {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for value in &mesh.normals {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for value in &mesh.colors {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for value in &mesh.indices {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bond_of_a_set_is_tessellated_with_its_own_radius_and_colours() {
        // Three bonds of different radii. The last one is the one that read past the end
        // of the colour array when colours were indexed with the instance stride.
        let mut scene = super::super::mesh::Scene::default();
        for (index, radius) in [0.22_f32, 0.30, 0.36].iter().enumerate() {
            let x = index as f32 * 3.0;
            scene.cylinders.push(
                [x, 0.0, 0.0],
                [x + 1.5, 0.0, 0.0],
                *radius,
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                (index as u32 * 2, index as u32 * 2 + 1),
            );
        }
        let mesh = tessellate(&scene, Quality::Fine);
        assert!(mesh.triangle_count() > 0);
        // Every vertex is flat in its half's colour: red before the middle, blue after,
        // with nothing in between - a gradient would show up as a mixed colour here.
        for vertex in 0..mesh.vertex_count() {
            let x = mesh.positions[vertex * 3];
            let bond_start = (x / 3.0).floor().clamp(0.0, 2.0) * 3.0;
            let along = x - bond_start;
            let color = [
                mesh.colors[vertex * 3],
                mesh.colors[vertex * 3 + 1],
                mesh.colors[vertex * 3 + 2],
            ];
            // The middle ring belongs to both halves, so it is the one place either colour
            // is allowed; everywhere else the half decides.
            if (along - 0.75).abs() > 1e-3 {
                let expected = if along < 0.75 {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 0.0, 1.0]
                };
                assert_eq!(color, expected, "vertex {vertex} at {along} along its bond");
            }
        }
        // Every vertex sits at its own bond's radius from that bond's axis.
        for vertex in 0..mesh.vertex_count() {
            let position = [
                mesh.positions[vertex * 3],
                mesh.positions[vertex * 3 + 1],
                mesh.positions[vertex * 3 + 2],
            ];
            let bond = (position[0] / 3.0).floor().clamp(0.0, 2.0) as usize;
            // Vertices sit just outside the radius - circumscribed, so the flats are on it.
            let expected = [0.22_f32, 0.30, 0.36][bond]
                / (std::f32::consts::PI / Quality::Fine.cylinder_segments() as f32).cos();
            let distance = (position[1] * position[1] + position[2] * position[2]).sqrt();
            assert!(
                (distance - expected).abs() < 1e-3,
                "vertex {vertex} of bond {bond}: {distance} vs {expected}",
            );
        }
    }
    use crate::geometry::color::ColorScheme;
    use crate::geometry::{build_scene, Representation};
    use crate::structure::model::RawAtom;
    use crate::structure::Structure;

    fn two_atoms() -> Structure {
        let atoms = vec![
            RawAtom {
                name: " CA ".into(),
                element: 6,
                position: [0.0, 0.0, 0.0],
                b_factor: 30.0,
                occupancy: 1.0,
                residue_name: "ALA".into(),
                residue_number: 1,
                insertion_code: None,
                chain_id: "A".into(),
                is_hetatm: false,
                label_asym_id: String::new(),
                entity_id: String::new(),
            },
            RawAtom {
                name: " CB ".into(),
                element: 6,
                position: [1.5, 0.0, 0.0],
                b_factor: 30.0,
                occupancy: 1.0,
                residue_name: "ALA".into(),
                residue_number: 1,
                insertion_code: None,
                chain_id: "A".into(),
                is_hetatm: false,
                label_asym_id: String::new(),
                entity_id: String::new(),
            },
        ];
        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        crate::structure::bonds::build(&mut structure, &Default::default());
        structure
    }

    fn scene(representation: Representation) -> crate::geometry::Scene {
        build_scene(
            &two_atoms(),
            crate::geometry::SceneOptions::default(),
            &[crate::geometry::Assignment::everything(
                crate::geometry::Style {
                    representations: vec![representation],
                    scheme: ColorScheme::Element,
                    ..Default::default()
                },
            )],
            &crate::geometry::SceneStyle::default(),
            false,
        )
    }

    #[test]
    fn spheres_become_closed_surfaces() {
        let mesh = tessellate(&scene(Representation::Spacefill), Quality::Fine);
        assert!(mesh.triangle_count() > 100);
        // Every vertex must sit on one of the two spheres, at its own radius.
        for vertex in 0..mesh.vertex_count() {
            let position = [
                mesh.positions[vertex * 3],
                mesh.positions[vertex * 3 + 1],
                mesh.positions[vertex * 3 + 2],
            ];
            let to_first = (position[0].powi(2) + position[1].powi(2) + position[2].powi(2)).sqrt();
            let to_second =
                ((position[0] - 1.5).powi(2) + position[1].powi(2) + position[2].powi(2)).sqrt();
            assert!(
                (to_first - 1.7).abs() < 1e-3 || (to_second - 1.7).abs() < 1e-3,
                "vertex {vertex} is on neither sphere"
            );
        }
    }

    #[test]
    fn every_face_winds_the_way_its_normals_point() {
        // The bug this pins: Blender lit a whole cartoon from inside its own surface and
        // rendered a black silhouette, because 99.9% of the triangles were inside out. Our
        // own shader flips normals per fragment and never noticed.
        for representation in [
            Representation::Spacefill,
            Representation::BallAndStick,
            Representation::Sticks,
        ] {
            let mesh = tessellate(&scene(representation), Quality::Fine);
            let mut opposed = 0;
            for triangle in 0..mesh.triangle_count() {
                let corners: Vec<[f32; 3]> = (0..3)
                    .map(|corner| {
                        let vertex = mesh.indices[triangle * 3 + corner] as usize;
                        [
                            mesh.positions[vertex * 3],
                            mesh.positions[vertex * 3 + 1],
                            mesh.positions[vertex * 3 + 2],
                        ]
                    })
                    .collect();
                let mut average = [0.0f32; 3];
                for corner in 0..3 {
                    let vertex = mesh.indices[triangle * 3 + corner] as usize;
                    for (axis, total) in average.iter_mut().enumerate() {
                        *total += mesh.normals[vertex * 3 + axis];
                    }
                }
                let face = cross(sub(corners[1], corners[0]), sub(corners[2], corners[0]));
                let agreement = face[0] * average[0] + face[1] * average[1] + face[2] * average[2];
                if agreement < 0.0 {
                    opposed += 1;
                }
            }
            assert_eq!(
                opposed, 0,
                "{representation:?} has {opposed} inside-out faces"
            );
        }
    }

    #[test]
    fn no_degenerate_triangles() {
        // Blender's mesh validation rejects zero-area faces, and the poles of a
        // latitude/longitude sphere produce them if the quads there are emitted whole.
        let mesh = tessellate(&scene(Representation::BallAndStick), Quality::Fine);
        for triangle in 0..mesh.triangle_count() {
            let corners: Vec<[f32; 3]> = (0..3)
                .map(|corner| {
                    let vertex = mesh.indices[triangle * 3 + corner] as usize;
                    [
                        mesh.positions[vertex * 3],
                        mesh.positions[vertex * 3 + 1],
                        mesh.positions[vertex * 3 + 2],
                    ]
                })
                .collect();
            let edge1 = sub(corners[1], corners[0]);
            let edge2 = sub(corners[2], corners[0]);
            let area = length(cross(edge1, edge2)) * 0.5;
            assert!(area > 1e-9, "triangle {triangle} has area {area}");
        }
    }

    #[test]
    fn normals_are_unit_length() {
        let mesh = tessellate(&scene(Representation::BallAndStick), Quality::Fine);
        for vertex in 0..mesh.vertex_count() {
            let normal = [
                mesh.normals[vertex * 3],
                mesh.normals[vertex * 3 + 1],
                mesh.normals[vertex * 3 + 2],
            ];
            assert!((length(normal) - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn every_index_addresses_a_real_vertex() {
        let mesh = tessellate(&scene(Representation::BallAndStick), Quality::Fine);
        for index in &mesh.indices {
            assert!((*index as usize) < mesh.vertex_count());
        }
    }

    #[test]
    fn draft_quality_is_cheaper_than_fine() {
        let draft = tessellate(&scene(Representation::Spacefill), Quality::Draft);
        let fine = tessellate(&scene(Representation::Spacefill), Quality::Fine);
        assert!(fine.triangle_count() > draft.triangle_count() * 2);
    }

    #[test]
    fn the_mesh_file_is_self_describing() {
        let mesh = tessellate(&scene(Representation::Sticks), Quality::Draft);
        let bytes = encode_mesh_file(&mesh);
        assert_eq!(&bytes[0..4], MESH_FILE_MAGIC);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            mesh.vertex_count()
        );
        assert_eq!(
            u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize,
            mesh.triangle_count()
        );
        // Header plus three float arrays and one index array, exactly.
        let expected = 16 + mesh.vertex_count() * 36 + mesh.triangle_count() * 12;
        assert_eq!(bytes.len(), expected);
    }

    #[test]
    fn an_empty_scene_tessellates_to_nothing() {
        let mesh = tessellate(&super::super::mesh::Scene::empty(), Quality::Fine);
        assert!(mesh.is_empty());
        assert_eq!(encode_mesh_file(&mesh).len(), 16);
    }

    #[test]
    fn a_bond_of_no_length_is_skipped_and_others_along_any_axis_are_finite() {
        // Two atoms on one spot give a zero axis, which normalises to NaN and poisons the
        // Blender mesh. Bonds along each axis exercise the helper vector's switch.
        let mut scene = super::super::mesh::Scene::default();
        scene
            .cylinders
            .push([1.0; 3], [1.0; 3], 0.3, [1.0; 3], [1.0; 3], (0, 1));
        for axis in 0..3 {
            let mut end = [0.0; 3];
            end[axis] = 1.5;
            scene
                .cylinders
                .push([0.0; 3], end, 0.3, [1.0; 3], [0.0; 3], (0, 1));
        }
        scene
            .cylinders
            .push([0.0; 3], [0.0, -1.5, 0.0], 0.3, [1.0; 3], [0.0; 3], (0, 1));
        let mesh = tessellate(&scene, Quality::Draft);
        let per_bond = Quality::Draft.cylinder_segments() * 4;
        assert_eq!(
            mesh.triangle_count(),
            4 * per_bond,
            "the empty bond adds nothing"
        );
        assert!(mesh
            .positions
            .iter()
            .chain(&mesh.normals)
            .all(|value| value.is_finite()));
        for normal in mesh.normals.chunks(3) {
            assert!((length([normal[0], normal[1], normal[2]]) - 1.0).abs() < 1e-4);
        }
    }

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn length(a: [f32; 3]) -> f32 {
        (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
    }
}
