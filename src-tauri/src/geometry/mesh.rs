//! Geometry containers and their wire format.
//!
//! Same design as the structure payload: parallel arrays that are already in the layout
//! the GPU wants, so crossing the IPC boundary is a memcpy rather than a transformation.

use serde::Serialize;

/// An indexed triangle mesh with per-vertex normals and colours.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub colors: Vec<f32>,
    pub indices: Vec<u32>,
    /// Residue index per vertex, so a colour change re-uploads colours instead of
    /// rebuilding the spline.
    pub vertex_residue: Vec<u32>,
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Appends a vertex and returns its index.
    pub fn push_vertex(
        &mut self,
        position: [f32; 3],
        normal: [f32; 3],
        color: [f32; 3],
        residue: u32,
    ) -> u32 {
        let index = self.vertex_count() as u32;
        self.positions.extend_from_slice(&position);
        self.normals.extend_from_slice(&normal);
        self.colors.extend_from_slice(&color);
        self.vertex_residue.push(residue);
        index
    }

    pub fn push_triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Adds another mesh's triangles after this one's, as if they had been pushed here.
    pub fn append(&mut self, other: Mesh) {
        let offset = self.vertex_count() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.colors.extend_from_slice(&other.colors);
        self.vertex_residue.extend_from_slice(&other.vertex_residue);
        self.indices
            .extend(other.indices.iter().map(|index| index + offset));
    }

    /// Makes every triangle wind the same way its vertices' normals point, and reports how
    /// many had to be flipped.
    ///
    /// Winding decides which side of a face is the front, and getting it wrong is invisible
    /// in our own viewport: the mesh shader flips the normal it is given when it faces away
    /// from the camera, and culling is off, so a fully inside-out mesh renders correctly
    /// there. An external renderer has no such forgiveness - Blender lit an entire cartoon
    /// from inside its own surface and produced a black silhouette with specular highlights.
    ///
    /// Deriving each winding by hand at every emit site is the alternative, and it is worse:
    /// the correct order depends on which way the profile is traversed, which way the spline
    /// advances, and whether a cap is at the near or far end. Measuring against the normals,
    /// which are unambiguous, is one rule for all of them.
    pub fn orient_faces(&mut self) -> usize {
        let positions = &self.positions;
        let normals = &self.normals;
        let mut flipped = 0;

        for triangle in self.indices.chunks_exact_mut(3) {
            let corner = |index: u32| -> [f32; 3] {
                let base = index as usize * 3;
                [positions[base], positions[base + 1], positions[base + 2]]
            };
            let normal_of = |index: u32| -> [f32; 3] {
                let base = index as usize * 3;
                [normals[base], normals[base + 1], normals[base + 2]]
            };

            let a = corner(triangle[0]);
            let b = corner(triangle[1]);
            let c = corner(triangle[2]);
            let edge1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let edge2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let face = [
                edge1[1] * edge2[2] - edge1[2] * edge2[1],
                edge1[2] * edge2[0] - edge1[0] * edge2[2],
                edge1[0] * edge2[1] - edge1[1] * edge2[0],
            ];

            let mut average = [0.0f32; 3];
            for index in triangle.iter() {
                let normal = normal_of(*index);
                for axis in 0..3 {
                    average[axis] += normal[axis];
                }
            }

            let agreement = face[0] * average[0] + face[1] * average[1] + face[2] * average[2];
            if agreement < 0.0 {
                triangle.swap(1, 2);
                flipped += 1;
            }
        }
        flipped
    }
}

/// Spheres as instances: centre and radius, with colour kept separately because a colour
/// scheme change must not touch geometry.
#[derive(Debug, Default, Clone)]
pub struct SphereSet {
    /// x, y, z, radius per sphere.
    pub instances: Vec<f32>,
    pub colors: Vec<f32>,
    /// Atom index per sphere, for picking and recolouring.
    pub atoms: Vec<u32>,
}

impl SphereSet {
    pub fn len(&self) -> usize {
        self.instances.len() / 4
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn push(&mut self, center: [f32; 3], radius: f32, color: [f32; 3], atom: u32) {
        self.instances
            .extend_from_slice(&[center[0], center[1], center[2], radius]);
        self.colors.extend_from_slice(&color);
        self.atoms.push(atom);
    }
}

/// Cylinders as instances: two endpoints and the colour of each half.
#[derive(Debug, Default, Clone)]
pub struct CylinderSet {
    /// x1, y1, z1, x2, y2, z2, radius per cylinder.
    ///
    /// The radius rides with each bond rather than once for the set. One radius for the
    /// whole scene came from the *unassigned* style, so a chain drawn as sticks next to a
    /// cartoon got its joints drawn as 0.30 A spheres on 0.22 A sticks - a ball at every
    /// atom, in a representation whose whole point is not having balls.
    pub instances: Vec<f32>,
    /// r1, g1, b1, r2, g2, b2 per cylinder.
    pub colors: Vec<f32>,
    /// The atom pair each cylinder joins.
    pub atoms: Vec<u32>,
}

/// Floats per cylinder instance: two endpoints and a radius.
pub const CYLINDER_STRIDE: usize = 7;

impl CylinderSet {
    pub fn len(&self) -> usize {
        self.instances.len() / CYLINDER_STRIDE
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn push(
        &mut self,
        start: [f32; 3],
        end: [f32; 3],
        radius: f32,
        color_start: [f32; 3],
        color_end: [f32; 3],
        atoms: (u32, u32),
    ) {
        self.instances
            .extend_from_slice(&[start[0], start[1], start[2], end[0], end[1], end[2], radius]);
        self.colors.extend_from_slice(&color_start);
        self.colors.extend_from_slice(&color_end);
        self.atoms.extend_from_slice(&[atoms.0, atoms.1]);
    }

    /// The radius of one cylinder.
    pub fn radius(&self, index: usize) -> f32 {
        self.instances[index * CYLINDER_STRIDE + 6]
    }
}

/// Byte offsets of one block inside the geometry payload.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Block {
    pub offset: u32,
    pub length: u32,
}

/// Bumped when the layout changes, so a stale frontend fails loudly rather than reading
/// one array as another.
pub const GEOMETRY_WIRE_VERSION: u32 = 2;
pub const GEOMETRY_WIRE_MAGIC: &[u8; 4] = b"MGEO";

#[derive(Debug, Clone, Serialize)]
pub struct GeometryHeader {
    pub version: u32,
    pub representation: String,
    pub sphere_count: u32,
    pub cylinder_count: u32,
    pub vertex_count: u32,
    pub triangle_count: u32,
    /// Atoms that ended up drawn, which is not the same as the structure's atom count.
    pub drawn_atoms: u32,
    /// Framing information, computed here because it needs every drawn position and
    /// radius - the same scan the geometry builder is already doing.
    /// The size of the scene's id space, so a mask keyed by atom or residue can be sized.
    pub atom_count: u32,
    pub residue_count: u32,
    /// Where each merged structure's ids start.
    pub parts: Vec<ScenePartIds>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub blocks: GeometryBlocks,
}

/// Where one merged structure's ids begin in the scene's index space.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ScenePartIds {
    pub atom_offset: u32,
    pub residue_offset: u32,
    pub atom_count: u32,
    pub residue_count: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GeometryBlocks {
    /// f32 x 4 per sphere.
    pub sphere_instances: Block,
    /// f32 x 3 per sphere.
    pub sphere_colors: Block,
    /// u32 per sphere.
    pub sphere_atoms: Block,
    /// f32 x 7 per cylinder: both ends and the radius.
    pub cylinder_instances: Block,
    /// f32 x 6 per cylinder.
    pub cylinder_colors: Block,
    /// u32 x 2 per cylinder: the atoms at its ends.
    pub cylinder_atoms: Block,
    /// f32 x 3 per vertex.
    pub mesh_positions: Block,
    /// f32 x 3 per vertex.
    pub mesh_normals: Block,
    /// f32 x 3 per vertex.
    pub mesh_colors: Block,
    /// u32 per index.
    pub mesh_indices: Block,
    /// u32 per vertex.
    pub mesh_vertex_residue: Block,
}

/// Everything the renderer needs to draw a frame.
///
/// Usually one structure's geometry, but several can be merged into one - see `append` -
/// because a viewport draws a scene, not a molecule.
#[derive(Debug, Default, Clone)]
pub struct Scene {
    pub spheres: SphereSet,
    pub cylinders: CylinderSet,
    pub mesh: Mesh,
    pub representation: String,
    pub drawn_atoms: u32,
    /// How many atoms and residues the ids in this scene span.
    ///
    /// A scene can hold several structures, and its atom and residue ids have to stay
    /// unique across them or anything keyed by an id - a selection mask, a picked atom -
    /// means two different things at once. `append` offsets the incoming ids by these,
    /// which is what makes them a single index space.
    pub atom_count: u32,
    pub residue_count: u32,
    /// Where each merged structure's ids start, in merge order.
    pub parts: Vec<ScenePartIds>,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

impl Scene {
    /// A scene with nothing in it, to merge into.
    ///
    /// The bounds start inverted so that the first thing appended sets them. `default()`
    /// would start them at the origin, and every merged scene would then be framed as if
    /// it contained a point there.
    pub fn empty() -> Scene {
        Scene {
            bounds_min: [f32::INFINITY; 3],
            bounds_max: [f32::NEG_INFINITY; 3],
            ..Default::default()
        }
    }

    /// Collapses inverted bounds, for a merge that appended nothing.
    pub fn settle(&mut self) {
        if !self.bounds_min[0].is_finite() {
            self.bounds_min = [0.0; 3];
            self.bounds_max = [0.0; 3];
        }
    }

    /// Moves and sizes a scene so it sits at `at` with a radius of `size`.
    ///
    /// For the shelf of worlds, which is one viewport with every world standing in it: the
    /// camera travels along a row rather than each world being built when it is looked at.
    /// Sizing them alike is what a character select does - a small peptide and a ribosome
    /// are framed the same, because the row is for telling them apart, not for comparing
    /// how big they are.
    pub fn place(&mut self, at: [f32; 3], size: f32) {
        let (low, high) = (self.bounds_min, self.bounds_max);
        self.place_within(low, high, at, size);
    }

    /// `place`, measured against a box that is not this scene's own.
    ///
    /// For a world of several molecules: each is built on its own, but they stand where
    /// they stand relative to each other, so all of them are moved and sized by the box
    /// around the whole world. Placed one by one, each was centred on the same spot and
    /// blown up to the same size - a complex drawn as its parts piled on top of each other.
    pub fn place_within(&mut self, low: [f32; 3], high: [f32; 3], at: [f32; 3], size: f32) {
        let centre = [
            (low[0] + high[0]) * 0.5,
            (low[1] + high[1]) * 0.5,
            (low[2] + high[2]) * 0.5,
        ];
        if !centre[0].is_finite() {
            return;
        }
        let extent = (0..3)
            .map(|axis| (high[axis] - low[axis]) * 0.5)
            .fold(0.0_f32, f32::max);
        let scale = if extent > 1e-6 { size / extent } else { 1.0 };
        let move_axis = |value: f32, axis: usize| (value - centre[axis]) * scale + at[axis];

        for sphere in self.spheres.instances.chunks_mut(4) {
            for (axis, value) in sphere.iter_mut().take(3).enumerate() {
                *value = move_axis(*value, axis);
            }
            sphere[3] *= scale;
        }
        for cylinder in self.cylinders.instances.chunks_mut(CYLINDER_STRIDE) {
            for axis in 0..3 {
                cylinder[axis] = move_axis(cylinder[axis], axis);
                cylinder[axis + 3] = move_axis(cylinder[axis + 3], axis);
            }
            cylinder[6] *= scale;
        }
        for vertex in self.mesh.positions.chunks_mut(3) {
            for (axis, value) in vertex.iter_mut().enumerate() {
                *value = move_axis(*value, axis);
            }
        }
        for axis in 0..3 {
            let low = move_axis(self.bounds_min[axis], axis);
            let high = move_axis(self.bounds_max[axis], axis);
            self.bounds_min[axis] = low.min(high);
            self.bounds_max[axis] = high.max(low);
        }
    }

    /// Appends another scene's geometry.
    ///
    /// What lets one viewport draw several molecules: each is built on its own - it has its
    /// own structure, its own styles - and the results are concatenated into the single
    /// payload the renderer takes.
    ///
    /// Atom and residue indices are per structure, so after a merge they no longer
    /// identify anything uniquely. Nothing reads them across a merge today; picking will
    /// have to carry a structure id alongside.
    pub fn append(&mut self, other: Scene) {
        let vertex_offset = (self.mesh.positions.len() / 3) as u32;
        // Asked before anything is moved in: a scene with nothing drawn has no extent, and
        // its bounds are a placeholder at the origin rather than a place.
        let drew = |scene: &Scene| {
            !scene.spheres.is_empty() || !scene.cylinders.is_empty() || !scene.mesh.is_empty()
        };
        let (had_geometry, adds_geometry) = (drew(self), drew(&other));

        // Ids are offset into this scene's index space, not copied: two structures both
        // numbering their atoms from zero is how a selection of chain A of one ends up
        // highlighting part of the other.
        let atom_offset = self.atom_count;
        let residue_offset = self.residue_count;
        self.parts.push(ScenePartIds {
            atom_offset,
            residue_offset,
            atom_count: other.atom_count,
            residue_count: other.residue_count,
        });

        self.spheres.instances.extend(other.spheres.instances);
        self.spheres.colors.extend(other.spheres.colors);
        self.spheres
            .atoms
            .extend(other.spheres.atoms.iter().map(|atom| atom + atom_offset));

        self.cylinders.instances.extend(other.cylinders.instances);
        self.cylinders.colors.extend(other.cylinders.colors);
        self.cylinders
            .atoms
            .extend(other.cylinders.atoms.iter().map(|atom| atom + atom_offset));

        self.mesh.positions.extend(other.mesh.positions);
        self.mesh.normals.extend(other.mesh.normals);
        self.mesh.colors.extend(other.mesh.colors);
        self.mesh.vertex_residue.extend(
            other
                .mesh
                .vertex_residue
                .iter()
                .map(|residue| residue + residue_offset),
        );
        self.mesh
            .indices
            .extend(other.mesh.indices.iter().map(|index| index + vertex_offset));

        self.drawn_atoms += other.drawn_atoms;
        self.atom_count += other.atom_count;
        self.residue_count += other.residue_count;
        // A molecule with nothing visible - waters only, with waters hidden - must not
        // stretch the frame out to the origin, where its empty bounds sit.
        if adds_geometry && !had_geometry {
            self.bounds_min = other.bounds_min;
            self.bounds_max = other.bounds_max;
        } else if adds_geometry {
            for axis in 0..3 {
                self.bounds_min[axis] = self.bounds_min[axis].min(other.bounds_min[axis]);
                self.bounds_max[axis] = self.bounds_max[axis].max(other.bounds_max[axis]);
            }
        }
        if self.representation.is_empty() {
            self.representation = other.representation;
        }
    }

    /// Serialises to the binary payload the renderer decodes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body: Vec<u8> = Vec::with_capacity(
            self.spheres.instances.len() * 4
                + self.cylinders.instances.len() * 4
                + self.mesh.positions.len() * 4 * 3
                + self.mesh.indices.len() * 4,
        );
        // Order matters: each block records where it landed, so they are appended in a
        // fixed sequence and the header is built from the results.
        let blocks = GeometryBlocks {
            sphere_instances: push_f32(&mut body, &self.spheres.instances),
            sphere_colors: push_f32(&mut body, &self.spheres.colors),
            sphere_atoms: push_u32(&mut body, &self.spheres.atoms),
            cylinder_instances: push_f32(&mut body, &self.cylinders.instances),
            cylinder_colors: push_f32(&mut body, &self.cylinders.colors),
            cylinder_atoms: push_u32(&mut body, &self.cylinders.atoms),
            mesh_positions: push_f32(&mut body, &self.mesh.positions),
            // Four bytes a vertex each rather than twelve. A normal is a direction and a
            // colour is a value under one, and neither needs more than a byte a channel
            // on screen; a capsid's ribbon was 40 bytes a vertex and is 24, which is
            // what pays for its helices being round.
            mesh_normals: push_snorm8(&mut body, &self.mesh.normals),
            mesh_colors: push_unorm8(&mut body, &self.mesh.colors),
            mesh_indices: push_u32(&mut body, &self.mesh.indices),
            mesh_vertex_residue: push_u32(&mut body, &self.mesh.vertex_residue),
        };

        let header = GeometryHeader {
            version: GEOMETRY_WIRE_VERSION,
            representation: self.representation.clone(),
            sphere_count: self.spheres.len() as u32,
            cylinder_count: self.cylinders.len() as u32,
            vertex_count: self.mesh.vertex_count() as u32,
            triangle_count: self.mesh.triangle_count() as u32,
            drawn_atoms: self.drawn_atoms,
            atom_count: self.atom_count,
            residue_count: self.residue_count,
            parts: self.parts.clone(),
            bounds_min: self.bounds_min,
            bounds_max: self.bounds_max,
            blocks,
        };

        let header_json = serde_json::to_vec(&header).unwrap_or_default();
        let mut payload = Vec::with_capacity(8 + header_json.len() + body.len() + 4);
        payload.extend_from_slice(GEOMETRY_WIRE_MAGIC);
        payload.extend_from_slice(&(header_json.len() as u32).to_le_bytes());
        payload.extend_from_slice(&header_json);
        // Every block holds 4-byte values, so the body must start 4-byte aligned or the
        // frontend's typed-array views throw.
        while payload.len() % 4 != 0 {
            payload.push(0);
        }
        payload.extend_from_slice(&body);
        payload
    }
}

fn push_f32(body: &mut Vec<u8>, values: &[f32]) -> Block {
    let offset = body.len() as u32;
    for value in values {
        body.extend_from_slice(&value.to_le_bytes());
    }
    Block {
        offset,
        length: (body.len() as u32) - offset,
    }
}

/// Unit vectors, three to a vertex, as four signed bytes each: -127 to 127 for -1 to 1,
/// and a fourth byte of padding so each vertex starts on a four-byte boundary.
fn push_snorm8(body: &mut Vec<u8>, values: &[f32]) -> Block {
    let offset = body.len() as u32;
    for vector in values.chunks(3) {
        for axis in 0..3 {
            let value = vector.get(axis).copied().unwrap_or(0.0);
            let value = if value.is_finite() {
                value.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            body.push((value * 127.0).round() as i8 as u8);
        }
        body.push(0);
    }
    Block {
        offset,
        length: (body.len() as u32) - offset,
    }
}

/// Values from 0 to 1, three to a vertex, as four unsigned bytes each, the fourth padding.
fn push_unorm8(body: &mut Vec<u8>, values: &[f32]) -> Block {
    let offset = body.len() as u32;
    for colour in values.chunks(3) {
        for channel in 0..3 {
            let value = colour.get(channel).copied().unwrap_or(0.0);
            let value = if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            };
            body.push((value * 255.0).round() as u8);
        }
        body.push(255);
    }
    Block {
        offset,
        length: (body.len() as u32) - offset,
    }
}

fn push_u32(body: &mut Vec<u8>, values: &[u32]) -> Block {
    let offset = body.len() as u32;
    for value in values {
        body.extend_from_slice(&value.to_le_bytes());
    }
    Block {
        offset,
        length: (body.len() as u32) - offset,
    }
}

#[cfg(test)]
mod place_tests {
    use super::*;

    #[test]
    fn a_scene_is_moved_and_sized_to_where_it_was_asked_for() {
        let mut scene = Scene::empty();
        // Two spheres four apart on x, so the scene's own radius is 2.
        scene.spheres.instances = vec![-2.0, 0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 1.0];
        scene.mesh.positions = vec![-2.0, 0.0, 0.0, 2.0, 0.0, 0.0];
        scene.bounds_min = [-2.0, 0.0, 0.0];
        scene.bounds_max = [2.0, 0.0, 0.0];

        scene.place([40.0, 0.0, 0.0], 10.0);

        assert_eq!(scene.spheres.instances[0], 30.0);
        assert_eq!(scene.spheres.instances[4], 50.0);
        assert_eq!(scene.spheres.instances[3], 5.0);
        assert_eq!(scene.mesh.positions[0], 30.0);
        assert_eq!(scene.bounds_min[0], 30.0);
        assert_eq!(scene.bounds_max[0], 50.0);
    }

    #[test]
    fn a_scene_with_nothing_in_it_is_left_alone() {
        let mut scene = Scene::empty();
        scene.place([10.0, 0.0, 0.0], 5.0);
        assert!(!scene.bounds_min[0].is_finite());
    }

    fn all_finite(scene: &Scene) -> bool {
        scene
            .spheres
            .instances
            .iter()
            .chain(&scene.cylinders.instances)
            .chain(&scene.mesh.positions)
            .chain(&scene.bounds_min)
            .chain(&scene.bounds_max)
            .all(|value| value.is_finite())
    }

    #[test]
    fn a_box_of_no_size_is_moved_but_not_blown_up() {
        // A single point: the extent is zero, and sizing to it is a division by zero that
        // sends every coordinate to infinity.
        let mut scene = Scene::empty();
        scene.spheres.push([3.0, 4.0, 5.0], 0.0, [1.0; 3], 0);
        scene.bounds_min = [3.0, 4.0, 5.0];
        scene.bounds_max = [3.0, 4.0, 5.0];
        scene.place([10.0, 0.0, 0.0], 8.0);
        assert!(all_finite(&scene));
        assert_eq!(&scene.spheres.instances[0..3], &[10.0, 0.0, 0.0]);

        // Flat in two axes - a straight chain of atoms - sizes by the axis it has.
        let mut line = Scene::empty();
        line.cylinders
            .push([0.0; 3], [4.0, 0.0, 0.0], 0.2, [1.0; 3], [1.0; 3], (0, 1));
        line.bounds_min = [0.0; 3];
        line.bounds_max = [4.0, 0.0, 0.0];
        line.place([0.0; 3], 1.0);
        assert!(all_finite(&line));
        assert!((line.bounds_max[0] - line.bounds_min[0] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn a_placeholder_box_from_a_scene_that_drew_nothing_leaves_positions_finite() {
        // What `build_scene` reports for a molecule with nothing visible.
        let mut scene = Scene::default();
        scene.place_within([0.0; 3], [0.0; 3], [5.0, 5.0, 5.0], 10.0);
        assert!(all_finite(&scene));
        // And an infinite box - an unsettled merge - is refused rather than applied.
        let mut scene = Scene::default();
        scene.mesh.positions = vec![1.0, 2.0, 3.0];
        scene.place_within(
            [f32::INFINITY; 3],
            [f32::NEG_INFINITY; 3],
            [5.0, 5.0, 5.0],
            10.0,
        );
        assert_eq!(scene.mesh.positions, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn appending_an_empty_part_first_does_not_drag_the_frame_to_the_origin() {
        // Waters only, with waters hidden: a part with a placeholder box at the origin.
        let nothing = Scene {
            atom_count: 3,
            ..Default::default()
        };
        let mut far = Scene::default();
        far.spheres.push([100.0, 100.0, 100.0], 1.0, [1.0; 3], 0);
        far.bounds_min = [99.0; 3];
        far.bounds_max = [101.0; 3];

        for order in [
            [nothing.clone(), far.clone()],
            [far.clone(), nothing.clone()],
        ] {
            let mut merged = Scene::empty();
            for part in order {
                merged.append(part);
            }
            merged.settle();
            assert_eq!(merged.bounds_min, [99.0; 3]);
            assert_eq!(merged.bounds_max, [101.0; 3]);
        }
        // Nothing appended at all settles to a finite box.
        let mut merged = Scene::empty();
        merged.append(nothing);
        merged.settle();
        assert!(all_finite(&merged));
    }
}

#[cfg(test)]
mod tests {

    /// Two scenes' worth of geometry, concatenated.
    #[test]
    fn merged_scenes_keep_their_ids_apart() {
        // Two structures both numbering from zero: without offsets, a mask keyed by atom
        // id highlights part of the wrong molecule.
        let part = |atoms: u32, residues: u32| {
            let mut scene = Scene {
                atom_count: atoms,
                residue_count: residues,
                ..Default::default()
            };
            scene.spheres.push([0.0; 3], 1.0, [1.0; 3], 0);
            scene.spheres.push([1.0; 3], 1.0, [1.0; 3], atoms - 1);
            scene
                .cylinders
                .push([0.0; 3], [1.0, 0.0, 0.0], 0.2, [1.0; 3], [1.0; 3], (0, 1));
            scene.mesh.vertex_residue.extend([0, residues - 1]);
            scene
        };

        let mut merged = Scene::empty();
        merged.append(part(10, 3));
        merged.append(part(4, 2));

        assert_eq!(merged.atom_count, 14);
        assert_eq!(merged.residue_count, 5);
        // The second structure's atom 0 became atom 10, and its last residue became 4.
        assert_eq!(merged.spheres.atoms, vec![0, 9, 10, 13]);
        assert_eq!(merged.cylinders.atoms, vec![0, 1, 10, 11]);
        assert_eq!(merged.mesh.vertex_residue, vec![0, 2, 3, 4]);
        // And the header says where each part starts, so a selection can be mapped back.
        assert_eq!(merged.parts.len(), 2);
        assert_eq!(merged.parts[1].atom_offset, 10);
        assert_eq!(merged.parts[1].residue_offset, 3);
        assert_eq!(merged.parts[1].atom_count, 4);
    }

    #[test]
    fn appending_offsets_the_indices_and_keeps_each_radius() {
        let mut first = Scene::empty();

        let mut a = Scene::default();
        a.cylinders
            .push([0.0; 3], [1.0, 0.0, 0.0], 0.24, [1.0; 3], [1.0; 3], (0, 1));
        a.mesh
            .positions
            .extend([0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        a.mesh.indices.extend([0, 1, 2]);
        first.append(a);

        // A second part whose bonds are fatter keeps them fatter: the radius is per bond,
        // so two chains drawn differently no longer share one number.
        let mut b = Scene::default();
        b.cylinders
            .push([0.0; 3], [0.0, 1.0, 0.0], 0.9, [1.0; 3], [1.0; 3], (0, 1));
        b.mesh
            .positions
            .extend([2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 2.0, 1.0, 0.0]);
        b.mesh.indices.extend([0, 1, 2]);
        first.append(b);

        assert_eq!(first.cylinders.len(), 2);
        assert_eq!(first.cylinders.radius(0), 0.24);
        assert_eq!(first.cylinders.radius(1), 0.9);
        // The second triangle's indices moved past the first's vertices.
        assert_eq!(first.mesh.indices, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn an_empty_merge_does_not_leave_infinite_bounds() {
        let mut scene = Scene::empty();
        scene.settle();
        assert_eq!(scene.bounds_min, [0.0; 3]);
        assert_eq!(scene.bounds_max, [0.0; 3]);
    }
    use super::*;

    #[test]
    fn spheres_and_colors_stay_in_step() {
        let mut spheres = SphereSet::default();
        spheres.push([1.0, 2.0, 3.0], 1.7, [1.0, 0.0, 0.0], 42);
        spheres.push([4.0, 5.0, 6.0], 1.5, [0.0, 1.0, 0.0], 43);
        assert_eq!(spheres.len(), 2);
        assert_eq!(spheres.colors.len(), 6);
        assert_eq!(spheres.atoms, vec![42, 43]);
        assert_eq!(spheres.instances[3], 1.7);
    }

    #[test]
    fn the_payload_is_self_describing_and_aligned() {
        let mut scene = Scene {
            representation: "cartoon".into(),
            ..Default::default()
        };
        scene.spheres.push([0.0, 1.0, 2.0], 1.0, [1.0, 1.0, 1.0], 0);
        scene
            .mesh
            .push_vertex([3.0, 4.0, 5.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 7);
        scene.mesh.push_triangle(0, 0, 0);

        let payload = scene.encode();
        assert_eq!(&payload[0..4], GEOMETRY_WIRE_MAGIC);

        let header_length = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
        let header: serde_json::Value =
            serde_json::from_slice(&payload[8..8 + header_length]).unwrap();
        assert_eq!(header["version"], GEOMETRY_WIRE_VERSION);
        assert_eq!(header["sphere_count"], 1);
        assert_eq!(header["vertex_count"], 1);
        assert_eq!(header["representation"], "cartoon");

        let mut body = 8 + header_length;
        body += (4 - body % 4) % 4;
        assert_eq!(body % 4, 0);

        let offset = header["blocks"]["mesh_positions"]["offset"]
            .as_u64()
            .unwrap() as usize;
        let start = body + offset;
        assert_eq!(
            f32::from_le_bytes(payload[start..start + 4].try_into().unwrap()),
            3.0
        );
    }

    #[test]
    fn normals_and_colours_go_out_as_a_byte_a_channel() {
        let mut body = Vec::new();
        let block = push_snorm8(&mut body, &[1.0, -1.0, 0.0, 0.5, f32::NAN, 2.0]);
        assert_eq!(block.length, 8);
        assert_eq!(&body[..8], &[127, (-127_i8) as u8, 0, 0, 64, 0, 127, 0]);
        let mut body = Vec::new();
        push_unorm8(&mut body, &[0.0, 0.5, 1.0, -1.0, 2.0, f32::INFINITY]);
        assert_eq!(&body, &[0, 128, 255, 255, 0, 255, 0, 255]);
    }
}
