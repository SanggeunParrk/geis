//! Cartoon ribbons.
//!
//! The representation that makes a protein readable: a spline through the alpha carbons,
//! flat and wide through helices and strands, thin through loops.
//!
//! Three things make or break a cartoon, in the order they show on screen:
//!
//!   1. Frame continuity. The ribbon's flat face is oriented by the carbonyl direction,
//!      which rotates about 100 degrees per residue in a helix. Used as given, the ribbon
//!      shreds itself; each frame is aligned to the previous one instead.
//!   2. Chain breaks. Missing residues leave alpha carbons tens of angstroms apart, and a
//!      spline through them draws a bar straight across the structure.
//!   3. Smooth width transitions. A helix that ends in one step leaves a visible cliff.

use super::color;
use super::mesh::Mesh;
use crate::structure::model::{ResidueKind, SecondaryStructure};
use crate::structure::Structure;

/// Consecutive alpha carbons are 3.8 A apart. Past this the residues are not bonded.
const MAX_TRACE_GAP: f32 = 5.5;

/// The same limit for a nucleic acid, whose trace runs through the phosphorus atoms.
///
/// Consecutive phosphates are about 5.9 A apart in A-form and 6.5 to 7 A in B-form - every
/// one of them past the protein figure. Measured against that, each step of a strand is a
/// break, each nucleotide a segment of one point, and a segment of one point draws
/// nothing: the whole strand vanished from a cartoon, atoms hidden and no ribbon built.
const MAX_NUCLEIC_TRACE_GAP: f32 = 8.0;

/// How finely the ribbon is built.
///
/// Scaled to the structure rather than fixed: eight subdivisions and twenty profile points
/// are smooth at any zoom, and on a 3,000-residue assembly they are also a million
/// triangles. A capsid does not need the resolution a single domain does, because it is
/// never on screen at the same magnification.
#[derive(Debug, Clone, Copy)]
struct Detail {
    subdivisions: usize,
    /// Inside a helix. A helix turns once in 3.6 residues, so at the two or three samples
    /// a residue that carry a strand or a loop well enough it is drawn with seven to ten
    /// points a turn - a polygon, not a coil. It is given its own count, and the
    /// structures large enough to need the thinning keep round helices for the price of
    /// their helical stretches alone.
    helix_subdivisions: usize,
    profile_points: usize,
}

impl Detail {
    fn for_residue_count(count: usize) -> Self {
        match count {
            0..=1500 => Detail {
                subdivisions: 8,
                helix_subdivisions: 8,
                profile_points: 20,
            },
            1501..=6000 => Detail {
                subdivisions: 6,
                helix_subdivisions: 6,
                profile_points: 16,
            },
            6001..=30_000 => Detail {
                subdivisions: 4,
                helix_subdivisions: 6,
                profile_points: 12,
            },
            // Past here the structure is an assembly of many copies - a capsid, a
            // filament - seen from far enough away that a residue is a few pixels. The
            // ribbon is thinned to keep the whole of it within what a graphics card
            // holds: at the full detail a capsid's half a million residues are over a
            // hundred million triangles.
            30_001..=120_000 => Detail {
                subdivisions: 3,
                helix_subdivisions: 6,
                profile_points: 8,
            },
            120_001..=600_000 => Detail {
                subdivisions: 2,
                helix_subdivisions: 5,
                profile_points: 8,
            },
            // Two capsids, or a larger one: the helices stay helical, at four points a
            // residue, and the rest is as thin as a ribbon can be and still read.
            _ => Detail {
                subdivisions: 2,
                helix_subdivisions: 4,
                profile_points: 6,
            },
        }
    }

    /// The ribbon for a preview: cheap, but still round.
    ///
    /// For the shelf of worlds, where a structure is an inch across and turning. It went
    /// coarser than this at first - three subdivisions and an eight-point profile - and
    /// that was cheaper still, but a ribbon that coarse is visibly flat-sided, and a
    /// flat-sided ribbon turning slowly changes face in steps you can see. The judder was
    /// in the geometry, not in the clock. Six and twelve is half the cost of the full
    /// ribbon and keeps the curve.
    fn coarse() -> Self {
        Detail {
            subdivisions: 6,
            helix_subdivisions: 6,
            profile_points: 12,
        }
    }

    /// The detail a structure of this size is drawn at, for study or for a preview.
    ///
    /// A preview is never finer than the full view. It was, for anything large: the
    /// preview's fixed six-and-twelve stood in for the size-based choice rather than
    /// under it, so a capsid's thumbnail on the shelf of worlds was 64 million triangles
    /// and two gigabytes - six times the capsid in the viewport - and the window that
    /// received it was killed for its memory. Past 30,000 residues a preview is a square
    /// tube, one segment to a residue: a thumbnail of a capsid shows its shape, not its
    /// folds.
    fn for_scene(residues: usize, coarse: bool) -> Self {
        let full = Self::for_residue_count(residues);
        if !coarse {
            return full;
        }
        let preview = if residues > 30_000 {
            Detail {
                subdivisions: 1,
                helix_subdivisions: 1,
                profile_points: 4,
            }
        } else {
            Self::coarse()
        };
        Detail {
            subdivisions: full.subdivisions.min(preview.subdivisions),
            helix_subdivisions: full.helix_subdivisions.min(preview.helix_subdivisions),
            profile_points: full.profile_points.min(preview.profile_points),
        }
    }
}

/// Cross-section half-extents in angstroms, by secondary structure.
fn profile(secondary: SecondaryStructure) -> (f32, f32) {
    match secondary {
        SecondaryStructure::Helix => (1.15, 0.24),
        SecondaryStructure::Sheet => (1.0, 0.24),
        SecondaryStructure::Turn => (0.35, 0.35),
        SecondaryStructure::Coil => (0.3, 0.3),
    }
}

/// A nucleic acid's backbone: a round tube, a good deal stouter than a protein's coil.
///
/// It is the only thing a strand's ribbon has to say - there is no helix or sheet to
/// widen for - and at a coil's 0.3 A a double helix read as two threads with the bases,
/// which are most of a nucleotide, nowhere at all.
const NUCLEIC_BACKBONE: (f32, f32) = (0.5, 0.5);

fn point_profile(point: &TracePoint) -> (f32, f32) {
    if point.nucleic {
        NUCLEIC_BACKBONE
    } else {
        profile(point.secondary)
    }
}

/// One point on the cross-section outline: where it is and which way it faces, both in the
/// (side, up) basis.
#[derive(Debug, Clone, Copy)]
struct ProfilePoint {
    side: f32,
    up: f32,
    normal_side: f32,
    normal_up: f32,
}

/// Samples a stadium - a rectangle with semicircular ends - by arc length.
///
/// Not an ellipse, which is the obvious choice and the wrong one. Sampling an ellipse at
/// equal angles puts most of its points along the flat sides, where they are not needed,
/// and only a couple around the tightly curved ends, where the silhouette is. On a ribbon
/// flattened 5:1 that leaves visible notches along the edges. A stadium sampled by arc
/// length gives the flat faces a crisp constant normal and spends its resolution on the
/// rounded edges - which is also what a cartoon ribbon actually looks like.
fn profile_outline(width: f32, height: f32, count: usize) -> Vec<ProfilePoint> {
    let radius = width.min(height).max(1e-4);
    // The straight part's half-length. Zero for a circular cross-section, which is what a
    // coil is, and the formula degenerates to a circle cleanly.
    let flat = (width - radius).max(0.0);
    let cap = std::f32::consts::PI * radius;
    let perimeter = 4.0 * flat + 2.0 * cap;

    let mut points = Vec::with_capacity(count);
    for index in 0..count {
        // Walk counter-clockwise from the rightmost point: right cap, top, left cap,
        // bottom.
        let mut distance = index as f32 / count as f32 * perimeter;

        if distance < cap {
            let angle = distance / radius - std::f32::consts::FRAC_PI_2;
            let (sin, cos) = angle.sin_cos();
            points.push(ProfilePoint {
                side: flat + cos * radius,
                up: sin * radius,
                normal_side: cos,
                normal_up: sin,
            });
            continue;
        }
        distance -= cap;

        if distance < 2.0 * flat {
            points.push(ProfilePoint {
                side: flat - distance,
                up: radius,
                normal_side: 0.0,
                normal_up: 1.0,
            });
            continue;
        }
        distance -= 2.0 * flat;

        if distance < cap {
            let angle = distance / radius + std::f32::consts::FRAC_PI_2;
            let (sin, cos) = angle.sin_cos();
            points.push(ProfilePoint {
                side: -flat + cos * radius,
                up: sin * radius,
                normal_side: cos,
                normal_up: sin,
            });
            continue;
        }
        distance -= cap;

        points.push(ProfilePoint {
            side: -flat + distance,
            up: -radius,
            normal_side: 0.0,
            normal_up: -1.0,
        });
    }
    points
}

type Vec3 = [f32; 3];

struct TracePoint {
    position: Vec3,
    /// Direction the ribbon's flat face points, before alignment.
    side: Option<Vec3>,
    secondary: SecondaryStructure,
    residue: u32,
    /// A nucleotide's backbone is drawn as its own shape, whatever its "secondary structure".
    nucleic: bool,
}

struct Frame {
    tangent: Vec3,
    side: Vec3,
}

/// One cross-section's placement: where it sits and how it is oriented and shaped.
struct Ring {
    center: Vec3,
    side: Vec3,
    up: Vec3,
    /// Half-extent along `side` - the ribbon's width.
    width: f32,
    /// Half-extent along `up` - the ribbon's thickness.
    height: f32,
}

/// Builds the cartoon mesh for every polymer chain into `mesh`.
/// Builds ribbons.
///
/// `chains` restricts it to those chain indices, so a structure whose chains are drawn
/// differently gets a ribbon only where one was asked for. `None` means every chain.
pub fn build(
    structure: &Structure,
    colors: &[[f32; 3]],
    mesh: &mut Mesh,
    chains: Option<&[u32]>,
    coarse: bool,
) {
    build_among(
        structure,
        colors,
        mesh,
        chains,
        coarse,
        structure.residues.len(),
    );
}

/// `build`, at the detail a scene of `scene_residues` residues in all can afford.
pub fn build_among(
    structure: &Structure,
    colors: &[[f32; 3]],
    mesh: &mut Mesh,
    chains: Option<&[u32]>,
    coarse: bool,
    scene_residues: usize,
) {
    let detail = Detail::for_scene(scene_residues.max(structure.residues.len()), coarse);
    // Segments never span chains - `trace_segments` breaks on a chain change - so the
    // first point decides which chain the whole segment belongs to.
    let allowed: Option<Vec<bool>> = chains.map(|chains| {
        let mut allowed = vec![false; structure.chains.len()];
        for &chain in chains {
            if let Some(slot) = allowed.get_mut(chain as usize) {
                *slot = true;
            }
        }
        allowed
    });
    let segments: Vec<Vec<TracePoint>> = trace_segments(structure)
        .into_iter()
        .filter(|segment| segment.len() >= 2)
        .filter(|segment| match &allowed {
            None => true,
            Some(allowed) => structure
                .residues
                .get(segment[0].residue as usize)
                .is_some_and(|residue| allowed.get(residue.chain_index as usize) == Some(&true)),
        })
        .collect();
    // Each segment is its own tube, so the segments are shared out between the processor's
    // cores, each building a mesh of its own, and the pieces joined in order: the same
    // triangles as one core makes, in the same order. A capsid's sixty-odd chains were
    // built one after another.
    let residues: usize = segments.iter().map(Vec::len).sum();
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(residues.div_ceil(2_000).max(1));
    let build = |part: &[Vec<TracePoint>]| {
        // Room for about what the part will make, taken once: a mesh grown by doubling on
        // every core at once spent more time asking the system for memory than building.
        let points: usize = part.iter().map(Vec::len).sum();
        let vertices = points
            * detail.subdivisions.max(detail.helix_subdivisions)
            * (detail.profile_points + 1);
        let mut piece = Mesh::default();
        piece.positions.reserve(vertices * 3);
        piece.normals.reserve(vertices * 3);
        piece.colors.reserve(vertices * 3);
        piece.vertex_residue.reserve(vertices);
        piece.indices.reserve(vertices * 6);
        for segment in part {
            add_segment(segment, structure, colors, &mut piece, detail);
            for point in segment.iter().filter(|point| point.nucleic) {
                add_base(structure, point, colors, &mut piece);
            }
        }
        // The ribbon's rings, quads and caps each have their own natural winding; rather
        // than deriving all three by hand, they are reconciled against the normals here.
        piece.orient_faces();
        piece
    };
    if workers <= 1 {
        mesh.append(build(&segments));
        return;
    }
    // Split by residues rather than by segments, so one long chain is not one core's work
    // while the rest wait.
    let share = residues.div_ceil(workers).max(1);
    let mut parts: Vec<&[Vec<TracePoint>]> = Vec::new();
    let (mut start, mut held) = (0, 0);
    for (index, segment) in segments.iter().enumerate() {
        held += segment.len();
        if held >= share {
            parts.push(&segments[start..=index]);
            start = index + 1;
            held = 0;
        }
    }
    if start < segments.len() {
        parts.push(&segments[start..]);
    }
    let build = &build;
    let pieces: Vec<Mesh> = std::thread::scope(|scope| {
        let handles: Vec<_> = parts
            .into_iter()
            .map(|part| scope.spawn(move || build(part)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    // Room made once for all of it: growing a capsid's ribbon by doubling was most of the
    // time the joining took.
    let vertices: usize = pieces.iter().map(Mesh::vertex_count).sum();
    let indices: usize = pieces.iter().map(|piece| piece.indices.len()).sum();
    mesh.positions.reserve(vertices * 3);
    mesh.normals.reserve(vertices * 3);
    mesh.colors.reserve(vertices * 3);
    mesh.vertex_residue.reserve(vertices);
    mesh.indices.reserve(indices);
    for piece in pieces {
        mesh.append(piece);
    }
}

/// Half the thickness of a base's slab, and the radius of the rod that carries it.
const BASE_HALF_THICKNESS: f32 = 0.28;
const BASE_ROD_RADIUS: f32 = 0.26;

/// Draws one nucleotide's base: a rod from the backbone to where the base is attached,
/// and the base itself as a flat slab the shape of its rings.
///
/// This is the picture of a nucleic acid everyone carries - a ladder, rungs meeting in
/// the middle - and it is made of what a nucleotide mostly is. From the atoms that are
/// there: a purine is a hexagon fused to a pentagon, a pyrimidine a hexagon, and a model
/// with only its backbone gets the backbone alone.
fn add_base(structure: &Structure, point: &TracePoint, colors: &[[f32; 3]], mesh: &mut Mesh) {
    let Some(residue) = structure.residues.get(point.residue as usize) else {
        return;
    };
    let first = residue.first_atom as usize;
    let end = (first + residue.atom_count as usize).min(structure.atom_names.len());
    let find = |name: &str| (first..end).find(|&atom| structure.atom_names[atom].trim() == name);
    let ring = |names: &[&str]| -> Option<Vec<Vec3>> {
        names
            .iter()
            .map(|name| find(name).map(|atom| structure.position(atom)))
            .collect()
    };

    let Some(six) = ring(&["N1", "C2", "N3", "C4", "C5", "C6"]) else {
        return;
    };
    let five = ring(&["C4", "C5", "N7", "C8", "N9"]);
    // A purine hangs from N9, at the far end of its pentagon; a pyrimidine from N1.
    let attach = match (&five, find("N9"), find("N1")) {
        (Some(_), Some(n9), _) => n9,
        (_, _, Some(n1)) => n1,
        _ => return,
    };
    let color = colors.get(attach).copied().unwrap_or([0.8, 0.8, 0.8]);

    add_rod(
        mesh,
        point.position,
        structure.position(attach),
        BASE_ROD_RADIUS,
        color,
        point.residue,
    );
    add_slab(mesh, &six, BASE_HALF_THICKNESS, color, point.residue);
    if let Some(five) = five {
        add_slab(mesh, &five, BASE_HALF_THICKNESS, color, point.residue);
    }
}

/// A closed cylinder between two points.
fn add_rod(mesh: &mut Mesh, from: Vec3, to: Vec3, radius: f32, color: [f32; 3], residue: u32) {
    const SIDES: usize = 10;
    let axis = sub(to, from);
    if length(axis) < 1e-4 {
        return;
    }
    let axis = normalize(axis);
    let across = perpendicular(axis);
    let up = cross(axis, across);
    let start = mesh.vertex_count() as u32;
    for end in [from, to] {
        for side in 0..SIDES {
            let angle = side as f32 / SIDES as f32 * std::f32::consts::TAU;
            let normal = [
                across[0] * angle.cos() + up[0] * angle.sin(),
                across[1] * angle.cos() + up[1] * angle.sin(),
                across[2] * angle.cos() + up[2] * angle.sin(),
            ];
            let position = [
                end[0] + normal[0] * radius,
                end[1] + normal[1] * radius,
                end[2] + normal[2] * radius,
            ];
            mesh.push_vertex(position, normal, color, residue);
        }
    }
    for side in 0..SIDES as u32 {
        let next = (side + 1) % SIDES as u32;
        let (a, b) = (start + side, start + next);
        let (c, d) = (a + SIDES as u32, b + SIDES as u32);
        mesh.push_triangle(a, b, c);
        mesh.push_triangle(b, d, c);
    }
}

/// A flat prism over a convex ring of points: two faces and a wall round the edge.
fn add_slab(mesh: &mut Mesh, ring: &[Vec3], half: f32, color: [f32; 3], residue: u32) {
    let count = ring.len();
    if count < 3 {
        return;
    }
    let mut centre = [0.0f32; 3];
    for point in ring {
        centre = [
            centre[0] + point[0],
            centre[1] + point[1],
            centre[2] + point[2],
        ];
    }
    let centre = scaled(centre, 1.0 / count as f32);
    // The plane's normal, summed over the fan so one atom out of plane does not tilt it.
    let mut normal = [0.0f32; 3];
    for index in 0..count {
        let n = cross(
            sub(ring[index], centre),
            sub(ring[(index + 1) % count], centre),
        );
        normal = [normal[0] + n[0], normal[1] + n[1], normal[2] + n[2]];
    }
    if length(normal) < 1e-6 {
        return;
    }
    let normal = normalize(normal);
    let lift = scaled(normal, half);

    for (face, direction) in [(lift, normal), (scaled(lift, -1.0), scaled(normal, -1.0))] {
        let start = mesh.vertex_count() as u32;
        for point in ring {
            mesh.push_vertex(
                [point[0] + face[0], point[1] + face[1], point[2] + face[2]],
                direction,
                color,
                residue,
            );
        }
        for index in 1..count as u32 - 1 {
            mesh.push_triangle(start, start + index, start + index + 1);
        }
    }
    // The wall: each edge its own four vertices, so its face is flat-shaded and the slab
    // has an edge to catch the light rather than a rounded-off rim.
    for index in 0..count {
        let (a, b) = (ring[index], ring[(index + 1) % count]);
        let middle = scaled([a[0] + b[0], a[1] + b[1], a[2] + b[2]], 0.5);
        let out = sub(middle, centre);
        let along = dot(out, normal);
        let outward = normalize([
            out[0] - normal[0] * along,
            out[1] - normal[1] * along,
            out[2] - normal[2] * along,
        ]);
        let start = mesh.vertex_count() as u32;
        for corner in [a, b] {
            mesh.push_vertex(
                [
                    corner[0] + lift[0],
                    corner[1] + lift[1],
                    corner[2] + lift[2],
                ],
                outward,
                color,
                residue,
            );
            mesh.push_vertex(
                [
                    corner[0] - lift[0],
                    corner[1] - lift[1],
                    corner[2] - lift[2],
                ],
                outward,
                color,
                residue,
            );
        }
        mesh.push_triangle(start, start + 1, start + 2);
        mesh.push_triangle(start + 1, start + 3, start + 2);
    }
}

/// Splits the polymer into runs of consecutive, connected residues.
///
/// Split on distance rather than on residue numbering: numbering has gaps that are not
/// breaks (insertion codes) and continuities that are not real (renumbered constructs),
/// and the geometry is what the spline actually follows.
fn trace_segments(structure: &Structure) -> Vec<Vec<TracePoint>> {
    let mut segments: Vec<Vec<TracePoint>> = Vec::new();
    let mut current: Vec<TracePoint> = Vec::new();
    let mut last_chain = u32::MAX;

    for (index, residue) in structure.residues.iter().enumerate() {
        let is_polymer = matches!(
            residue.kind,
            ResidueKind::AminoAcid | ResidueKind::Nucleotide
        );
        if !is_polymer || residue.trace_atom < 0 {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            continue;
        }

        let position = structure.position(residue.trace_atom as usize);
        if residue.chain_index != last_chain {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            last_chain = residue.chain_index;
        } else if let Some(previous) = current.last() {
            let limit = if residue.kind == ResidueKind::Nucleotide {
                MAX_NUCLEIC_TRACE_GAP
            } else {
                MAX_TRACE_GAP
            };
            if distance(position, previous.position) > limit {
                segments.push(std::mem::take(&mut current));
            }
        }

        current.push(TracePoint {
            position,
            side: (residue.orient_atom >= 0)
                .then(|| sub(structure.position(residue.orient_atom as usize), position)),
            secondary: residue.secondary,
            residue: index as u32,
            nucleic: residue.kind == ResidueKind::Nucleotide,
        });
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// Which residues a ribbon is actually drawn over.
///
/// Not every polymer residue gets one: a residue with no trace atom, one alone between two
/// breaks, a chain a single residue long. A cartoon hides the atoms a ribbon replaces, and
/// hiding these as well leaves nothing where they are - so whoever hides atoms asks here
/// first.
pub fn covered(structure: &Structure) -> Vec<bool> {
    let mut covered = vec![false; structure.residues.len()];
    for segment in trace_segments(structure) {
        if segment.len() < 2 {
            continue;
        }
        for point in &segment {
            if let Some(slot) = covered.get_mut(point.residue as usize) {
                *slot = true;
            }
        }
    }
    covered
}

/// A helix's local axis at one residue: a point on it, and its direction.
#[derive(Debug, Clone, Copy)]
struct HelixAxis {
    centre: [f32; 3],
    direction: [f32; 3],
}

/// Ideal alpha-helix twist per residue, in degrees.
///
/// 3.6 residues per turn, which is the figure from Pauling and Corey's description of the
/// helix and from every structural biology text since. A fact about protein geometry, not
/// a parameter anyone chose.
const HELIX_TWIST_DEGREES: f32 = 100.0;

/// The four-point weights whose average lands on a helix's axis.
///
/// Derived rather than tabulated. Four consecutive alpha carbons of an ideal helix sit at
/// the same radius from the axis, at angles `0, t, 2t, 3t` with `t` the twist per residue.
/// A weighted average of the four - the outer pair sharing one weight and the inner pair
/// another, by symmetry - lands on the axis exactly when the radial parts cancel:
///
/// ```text
/// w_far (cos 0 + cos 3t) + w_near (cos t + cos 2t) = 0
/// ```
///
/// and it is an average rather than a scaling when `2 (w_far + w_near) = 1`. Two equations
/// and two unknowns, so the weights are whatever the twist makes them: at `t = 100 deg`,
/// (0.213, 0.287). Solving it here keeps the numbers attached to their reason, and means a
/// different twist - a nucleic acid's, say - would give its own weights rather than
/// needing new constants.
fn axis_weights() -> (f32, f32) {
    let twist = HELIX_TWIST_DEGREES.to_radians();
    let outer = 1.0 + (3.0 * twist).cos();
    let inner = twist.cos() + (2.0 * twist).cos();
    // w_far = -(inner / outer) w_near, and the pair sums to a half.
    let ratio = -inner / outer;
    let near = 0.5 / (1.0 + ratio);
    (near, 0.5 - near)
}

/// Fits the local helix axis at each residue of a helix run.
///
/// The four-point average of consecutive alpha carbons lands on the axis - see
/// `axis_weights` for why - and here it feeds both the ribbon's face *and* the path,
/// because a spline through alpha carbons is only 3.6 points per turn and comes out as a
/// rounded triangle. Averaging onto the axis and then rebuilding the helix around it is
/// what makes a round coil. (Reading a viewer that gets this right will tell you the same
/// thing; the arithmetic above is where it comes from.)
fn fit_helix_axes(trace: &[TracePoint]) -> Vec<Option<HelixAxis>> {
    let (near, far) = axis_weights();

    let mut points: Vec<Option<[f32; 3]>> = vec![None; trace.len()];
    for index in 3..trace.len() {
        if (index - 3..=index).any(|at| trace[at].secondary != SecondaryStructure::Helix) {
            continue;
        }
        let (a, b, c, d) = (
            trace[index - 3].position,
            trace[index - 2].position,
            trace[index - 1].position,
            trace[index].position,
        );
        // The window covers residues index-3..=index, so the point belongs between the
        // middle two - index-2 and index-1. Attributed to index-2, which keeps the run of
        // axis points in step with the residues.
        points[index - 2] = Some([
            far * (a[0] + d[0]) + near * (b[0] + c[0]),
            far * (a[1] + d[1]) + near * (b[1] + c[1]),
            far * (a[2] + d[2]) + near * (b[2] + c[2]),
        ]);
    }

    // Fill the ends of each run by extending the nearest fitted point, so the first and
    // last residues of a helix are still on the axis.
    let fitted: Vec<Option<[f32; 3]>> = points.clone();
    let mut axes: Vec<Option<HelixAxis>> = vec![None; trace.len()];
    for index in 0..trace.len() {
        if trace[index].secondary != SecondaryStructure::Helix {
            continue;
        }
        // The direction from this point to the next fitted one, or the previous pair.
        let Some(here) = fitted[index].or_else(|| nearest_fitted(&fitted, trace, index)) else {
            continue;
        };
        // The nearest fitted point either way that is not `here` itself. At the ends of
        // a run `here` is borrowed from a neighbour, so the first point found is the same
        // one - and taking it left the first residue and the last two of every helix with
        // no direction, and so with no axis at all.
        let apart = |point: &[f32; 3]| distance(*point, here) > 1e-4;
        let ahead = (index + 1..trace.len())
            .take_while(|at| trace[*at].secondary == SecondaryStructure::Helix)
            .filter_map(|at| fitted[at])
            .find(apart);
        let behind = (0..index)
            .rev()
            .take_while(|at| trace[*at].secondary == SecondaryStructure::Helix)
            .filter_map(|at| fitted[at])
            .find(apart);
        let direction = match (behind, ahead) {
            (_, Some(next)) => normalize(sub(next, here)),
            (Some(previous), _) => normalize(sub(here, previous)),
            _ => continue,
        };
        // The fitted point sits on the axis but not level with this residue: a window's
        // average lands midway between its two middle residues, and an end residue
        // borrows a neighbour's. Slid along the axis to this residue's own height, the
        // helical path starts and ends exactly on the alpha carbons - where the spline on
        // either side of the helix also passes - instead of three quarters of an angstrom
        // along from them, with a jog at both ends.
        let along = dot(sub(trace[index].position, here), direction);
        axes[index] = Some(HelixAxis {
            centre: [
                here[0] + direction[0] * along,
                here[1] + direction[1] * along,
                here[2] + direction[2] * along,
            ],
            direction,
        });
    }
    axes
}

/// The nearest fitted axis point within the same helix run.
fn nearest_fitted(
    fitted: &[Option<[f32; 3]>],
    trace: &[TracePoint],
    index: usize,
) -> Option<[f32; 3]> {
    let ahead = (index + 1..trace.len())
        .take_while(|at| trace[*at].secondary == SecondaryStructure::Helix)
        .find_map(|at| fitted[at]);
    if ahead.is_some() {
        return ahead;
    }
    (0..index)
        .rev()
        .take_while(|at| trace[*at].secondary == SecondaryStructure::Helix)
        .find_map(|at| fitted[at])
}

/// Rotates `v` about `axis` by `angle`, with Rodrigues' formula.
fn rotate_about(v: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    let cross_term = cross(axis, v);
    let dot_term = dot(axis, v);
    [
        v[0] * cos + cross_term[0] * sin + axis[0] * dot_term * (1.0 - cos),
        v[1] * cos + cross_term[1] * sin + axis[1] * dot_term * (1.0 - cos),
        v[2] * cos + cross_term[2] * sin + axis[2] * dot_term * (1.0 - cos),
    ]
}

/// A point on the helix between two residues, at fraction `t`.
///
/// The centre slides along the axis and the radius vector *rotates* about it, which is
/// what an actual helix does - so the path is round rather than a 3.6-sided polygon. Falls
/// back to `None` when the two residues do not share a plausible axis, and the caller uses
/// the spline instead.
fn helix_point(
    from: HelixAxis,
    to: HelixAxis,
    start: [f32; 3],
    end: [f32; 3],
    t: f32,
) -> Option<[f32; 3]> {
    // The two axes have to agree: at a kink they do not, and the spline is the better
    // answer there.
    if dot(from.direction, to.direction) < 0.9 {
        return None;
    }
    let axis = normalize(mix(from.direction, to.direction, t));
    let radial_from = perpendicular_to(sub(start, from.centre), from.direction);
    let radial_to = perpendicular_to(sub(end, to.centre), to.direction);
    let (from_len, to_len) = (length(radial_from), length(radial_to));
    if from_len < 0.2 || to_len < 0.2 {
        return None;
    }

    // The turn between them, signed about the axis.
    let unit_from = scaled(radial_from, 1.0 / from_len);
    let unit_to = scaled(radial_to, 1.0 / to_len);
    let mut angle = dot(unit_from, unit_to).clamp(-1.0, 1.0).acos();
    if dot(cross(unit_from, unit_to), axis) < 0.0 {
        angle = -angle;
    }

    let centre = mix(from.centre, to.centre, t);
    let radius = from_len + (to_len - from_len) * t;
    let spun = rotate_about(unit_from, axis, angle * t);
    Some([
        centre[0] + spun[0] * radius,
        centre[1] + spun[1] * radius,
        centre[2] + spun[2] * radius,
    ])
}

fn perpendicular_to(v: Vec3, axis: Vec3) -> Vec3 {
    sub(v, scaled(axis, dot(v, axis)))
}

fn add_segment(
    trace: &[TracePoint],
    structure: &Structure,
    colors: &[[f32; 3]],
    mesh: &mut Mesh,
    detail: Detail,
) {
    // Control points, as Mol*'s `setControlPoint` does it: a strand's pleat is smoothed
    // with a mild (1, 2, 1)/4 average, a helix's position is left exactly where it is. The
    // helix is handled by its orientation instead - see `round_helices` - because moving a
    // helix onto its axis throws its radius away.
    let controls: Vec<[f32; 3]> = (0..trace.len())
        .map(|index| {
            let here = trace[index].position;
            if trace[index].secondary != SecondaryStructure::Sheet
                || index == 0
                || index + 1 == trace.len()
            {
                return here;
            }
            let before = trace[index - 1].position;
            let after = trace[index + 1].position;
            [
                (before[0] + 2.0 * here[0] + after[0]) / 4.0,
                (before[1] + 2.0 * here[1] + after[1]) / 4.0,
                (before[2] + 2.0 * here[2] + after[2]) / 4.0,
            ]
        })
        .collect();

    // Tangents first: the helix orientation is built from them.
    let tangents: Vec<[f32; 3]> = (0..trace.len())
        .map(|index| {
            let previous = controls[index.saturating_sub(1)];
            let next = controls[(index + 1).min(trace.len() - 1)];
            normalize(sub(next, previous))
        })
        .collect();
    let axes = fit_helix_axes(trace);
    // The face comes from the axis: the ribbon is broad *along* the helix, so its flat
    // face points outward and consecutive turns line up.
    let helix_sides: Vec<Option<[f32; 3]>> = (0..trace.len())
        .map(|index| {
            let axis = axes[index]?;
            // Asked of the length, not of the result: `normalize` answers a zero vector
            // with a fixed direction rather than with NaN, so there is nothing to catch
            // afterwards.
            let across = perpendicular_to(axis.direction, tangents[index]);
            (length(across) > 1e-4).then(|| normalize(across))
        })
        .collect();

    // Per-residue frames, each aligned to the one before it.
    let mut frames: Vec<Frame> = Vec::with_capacity(trace.len());
    let mut in_helix = false;
    let mut helix_sense = 1.0f32;
    for index in 0..trace.len() {
        let tangent = tangents[index];

        // A helix takes its face from its own axis; everything else from the carbonyl.
        let from_axis = helix_sides[index];
        let raw = from_axis
            .or(trace[index].side)
            .or_else(|| frames.last().map(|frame| frame.side))
            .unwrap_or([1.0, 0.0, 0.0]);
        // Only the component across the chain defines the ribbon's face.
        let across = sub(raw, scaled(tangent, dot(raw, tangent)));
        let mut side = if length(across) > 1e-4 {
            normalize(across)
        } else {
            perpendicular(tangent)
        };
        // A carbonyl turns about 100 degrees per residue, and flipping each frame to agree
        // with the previous one keeps a loop's face continuous rather than letting it
        // rotate a third of a turn between residues.
        //
        // Not for a helix. `cross(axis, tangent)` turns *with* the tangent, so consecutive
        // faces along a helix are a hundred degrees apart with a negative dot product -
        // and this rule then inverts every other frame. That is what shredded the helices:
        // the ribbon was flipping face every residue while the geometry underneath was
        // perfectly smooth.
        if from_axis.is_none() {
            if let Some(previous_frame) = frames.last() {
                if dot(side, previous_frame.side) < 0.0 {
                    side = scaled(side, -1.0);
                }
            }
        } else {
            // A helix is matched to what came before it once, where it starts, and then
            // keeps that sense to its end. Left unmatched, a coil that arrives facing
            // the other way meets the helix across a half turn of ribbon in the space of
            // one residue.
            if !in_helix {
                helix_sense = match frames.last() {
                    Some(previous_frame) if dot(side, previous_frame.side) < 0.0 => -1.0,
                    _ => 1.0,
                };
            }
            side = scaled(side, helix_sense);
        }
        in_helix = from_axis.is_some();
        frames.push(Frame { tangent, side });
    }

    // Sampled first, then emitted, because the faces are smoothed along the finished
    // curve - Mol* does the same, a three-point average over the sampled normals after
    // the interpolation rather than over the residues before it.
    struct Sample {
        point: [f32; 3],
        tangent: [f32; 3],
        side: [f32; 3],
        width: f32,
        height: f32,
        residue: u32,
    }
    let mut samples: Vec<Sample> = Vec::with_capacity(trace.len() * detail.subdivisions);

    for index in 0..trace.len() - 1 {
        let per = if axes[index].is_some() && axes[index + 1].is_some() {
            detail.helix_subdivisions
        } else {
            detail.subdivisions
        };
        let steps = if index == trace.len() - 2 {
            per + 1
        } else {
            per
        };
        for step in 0..steps {
            let t = step as f32 / per as f32;
            let spline = |at: f32| {
                catmull_rom(
                    controls[index.saturating_sub(1)],
                    controls[index],
                    controls[index + 1],
                    controls[(index + 2).min(trace.len() - 1)],
                    at,
                )
            };
            // Inside a helix the path is generated as a helix - centre sliding along the
            // axis, radius rotating about it - because a spline through alpha carbons at
            // 100 degrees a residue is 3.6 points per turn and reads as a rounded
            // triangle. Everywhere else the spline is right.
            let helical = match (axes[index], axes[index + 1]) {
                (Some(from), Some(to)) => helix_point(
                    from,
                    to,
                    trace[index].position,
                    trace[index + 1].position,
                    t,
                ),
                _ => None,
            };
            let point = helical.unwrap_or_else(|| spline(t));

            // From the spline, not from the residues. Mol*'s `interpolatePointsAndTangents`
            // evaluates the curve at t plus and minus a hundredth and differences them,
            // and that is the difference between a cross-section perpendicular to the
            // path and one merely near it: blending the per-residue tangents linearly
            // leaves the ribbon tilted against its own curve, which reads as a kink at
            // every residue.
            const EPS: f32 = 0.01;
            let sample = |at: f32| match (axes[index], axes[index + 1]) {
                (Some(from), Some(to)) => helix_point(
                    from,
                    to,
                    trace[index].position,
                    trace[index + 1].position,
                    at,
                )
                .unwrap_or_else(|| spline(at)),
                _ => spline(at),
            };
            let step = sub(sample(t + EPS), sample(t - EPS));
            let tangent = if length(step) > 1e-6 {
                normalize(step)
            } else {
                // Coincident residues: the curve does not move here, so the frames say.
                normalize(mix(frames[index].tangent, frames[index + 1].tangent, t))
            };
            // Rotated rather than mixed. A straight blend of two vectors a hundred degrees
            // apart cuts the chord: the face turns fastest at the ends of each residue and
            // slowest in the middle, and the ribbon shows a crease at every boundary.
            let side = slerp(frames[index].side, frames[index + 1].side, t);

            // Blending the profile across the join is what removes the step at the end of
            // a helix; a hard switch leaves a visible cliff in the ribbon.
            let (width_a, height_a) = point_profile(&trace[index]);
            let (width_b, height_b) = point_profile(&trace[index + 1]);
            let blend = smoothstep(t);
            let residue = if t < 0.5 {
                trace[index].residue
            } else {
                trace[index + 1].residue
            };
            samples.push(Sample {
                point,
                tangent,
                side,
                width: width_a + (width_b - width_a) * blend,
                height: height_a + (height_b - height_a) * blend,
                residue,
            });
        }
    }

    // One three-point average over the sampled faces, then re-orthogonalise. This is the
    // pass that takes the last of the faceting out of a tight turn.
    if samples.len() > 2 {
        let sides: Vec<[f32; 3]> = samples.iter().map(|sample| sample.side).collect();
        for index in 1..samples.len() - 1 {
            let (before, here, after) = (sides[index - 1], sides[index], sides[index + 1]);
            samples[index].side = [
                (before[0] + here[0] + after[0]) / 3.0,
                (before[1] + here[1] + after[1]) / 3.0,
                (before[2] + here[2] + after[2]) / 3.0,
            ];
        }
    }

    let first_vertex = mesh.vertex_count() as u32;
    let mut rings = 0usize;

    {
        for sample in &samples {
            let tangent = sample.tangent;
            // Asked of the length before normalising, because `normalize` answers a zero
            // vector with a fixed axis rather than NaN - and that axis can be the tangent
            // itself, which flattens the ring into a line. Two faces that cancel in the
            // smoothing pass above, or a face that lies along the path, both land here.
            let across = sub(sample.side, scaled(tangent, dot(sample.side, tangent)));
            let side = if length(across) > 1e-4 {
                normalize(across)
            } else {
                perpendicular(tangent)
            };
            let up = normalize(cross(tangent, side));
            let residue = sample.residue;
            let color = color::residue_color(colors, structure, residue as usize);

            emit_ring(
                mesh,
                &Ring {
                    center: sample.point,
                    side,
                    up,
                    width: sample.width,
                    height: sample.height,
                },
                color,
                residue,
                detail.profile_points,
            );
            rings += 1;
        }
    }

    // Stitch consecutive rings into quads.
    let points = detail.profile_points;
    for ring in 0..rings.saturating_sub(1) {
        for point in 0..points {
            let next_point = (point + 1) % points;
            let base = first_vertex + (ring * points) as u32;
            let next_base = first_vertex + ((ring + 1) * points) as u32;
            let a = base + point as u32;
            let b = base + next_point as u32;
            let c = next_base + point as u32;
            let d = next_base + next_point as u32;
            mesh.push_triangle(a, c, b);
            mesh.push_triangle(b, c, d);
        }
    }

    // Caps, so the ends of a chain are not open tubes.
    if rings > 0 {
        cap_ring(mesh, first_vertex, true, points);
        cap_ring(
            mesh,
            first_vertex + ((rings - 1) * points) as u32,
            false,
            points,
        );
    }
}

/// One cross-section: `count` vertices with outward normals.
fn emit_ring(mesh: &mut Mesh, ring: &Ring, color: [f32; 3], residue: u32, count: usize) {
    let Ring {
        center,
        side,
        up,
        width,
        height,
    } = *ring;

    for point in profile_outline(width, height, count) {
        let position = [
            center[0] + side[0] * point.side + up[0] * point.up,
            center[1] + side[1] * point.side + up[1] * point.up,
            center[2] + side[2] * point.side + up[2] * point.up,
        ];
        // The outline carries its own normal, exact for both the flat faces and the
        // rounded edges - no reconstruction from the position needed.
        let normal = normalize([
            side[0] * point.normal_side + up[0] * point.normal_up,
            side[1] * point.normal_side + up[1] * point.normal_up,
            side[2] * point.normal_side + up[2] * point.normal_up,
        ]);
        mesh.push_vertex(position, normal, color, residue);
    }
}

/// Triangle fan closing a ring, wound so it faces outward.
fn cap_ring(mesh: &mut Mesh, base: u32, front: bool, count: usize) {
    for point in 1..count as u32 - 1 {
        if front {
            mesh.push_triangle(base, base + point, base + point + 1);
        } else {
            mesh.push_triangle(base, base + point + 1, base + point);
        }
    }
}

fn catmull_rom(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    let mut out = [0.0; 3];
    for axis in 0..3 {
        out[axis] = 0.5
            * (2.0 * p1[axis]
                + (-p0[axis] + p2[axis]) * t
                + (2.0 * p0[axis] - 5.0 * p1[axis] + 4.0 * p2[axis] - p3[axis]) * t2
                + (-p0[axis] + 3.0 * p1[axis] - 3.0 * p2[axis] + p3[axis]) * t3);
    }
    out
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scaled(a: Vec3, factor: f32) -> Vec3 {
    [a[0] * factor, a[1] * factor, a[2] * factor]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Rotates from `a` to `b` at a constant angular rate.
///
/// The ribbon's face turns a hundred degrees a residue through a helix, and a linear blend
/// of the endpoints traces the chord instead of the arc - uneven, with a visible crease at
/// each residue. Falls back to a blend when the two are nearly parallel, where the arc and
/// the chord agree and the sine below would divide by nearly zero.
fn slerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    let cosine = dot(a, b).clamp(-1.0, 1.0);
    let angle = cosine.acos();
    // NaN included: an unusable angle falls back to the blend.
    if angle.is_nan() || angle <= 1e-3 {
        return normalize(mix(a, b, t));
    }
    let sine = angle.sin();
    let wa = ((1.0 - t) * angle).sin() / sine;
    let wb = (t * angle).sin() / sine;
    normalize([
        a[0] * wa + b[0] * wb,
        a[1] * wa + b[1] * wb,
        a[2] * wa + b[2] * wb,
    ])
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

fn distance(a: Vec3, b: Vec3) -> f32 {
    length(sub(a, b))
}

/// A degenerate axis happens with duplicated coordinates; returning a valid direction
/// keeps the geometry finite instead of filling it with NaNs that blank the whole mesh.
fn normalize(a: Vec3) -> Vec3 {
    let len = length(a);
    if len < 1e-8 {
        [0.0, 0.0, 1.0]
    } else {
        [a[0] / len, a[1] / len, a[2] / len]
    }
}

fn mix(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Any unit vector perpendicular to `v`, for residues with no carbonyl to orient by.
fn perpendicular(v: Vec3) -> Vec3 {
    let helper = if v[1].abs() < 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    normalize(cross(helper, v))
}

#[cfg(test)]
mod tests {

    /// An ideal alpha helix: alpha carbons 2.3 A off the axis, 100 degrees and 1.5 A of
    /// rise per residue, about the z axis.
    fn ideal_helix(count: usize) -> Vec<TracePoint> {
        (0..count)
            .map(|index| {
                let angle = (index as f32) * 100.0_f32.to_radians();
                TracePoint {
                    position: [2.3 * angle.cos(), 2.3 * angle.sin(), 1.5 * index as f32],
                    side: None,
                    secondary: SecondaryStructure::Helix,
                    residue: index as u32,
                    nucleic: false,
                }
            })
            .collect()
    }

    /// The same ideal helix as a `Structure`, so the whole builder can be run on it.
    /// The axis is y, and every residue is marked as helix.
    fn ideal_helix_structure(count: usize) -> Structure {
        use crate::structure::model::{RawAtom, SecondaryStructure};
        let mut atoms = Vec::new();
        for index in 0..count {
            let base = 100.0_f32.to_radians() * index as f32;
            for (name, radius, offset, rise, element) in [
                (" N  ", 1.55, -28.0_f32, -0.9, 7u8),
                (" CA ", 2.30, 0.0, 0.0, 6),
                (" C  ", 1.68, 26.0, 0.6, 6),
                (" O  ", 2.15, 42.0, 0.85, 8),
            ] {
                let angle = base + offset.to_radians();
                atoms.push(RawAtom {
                    name: name.into(),
                    element,
                    position: [
                        radius * angle.cos(),
                        1.5 * index as f32 + rise,
                        radius * angle.sin(),
                    ],
                    b_factor: 20.0,
                    occupancy: 1.0,
                    residue_name: "ALA".into(),
                    residue_number: index as i32 + 1,
                    insertion_code: None,
                    chain_id: "A".into(),
                    is_hetatm: false,
                    label_asym_id: String::new(),
                    entity_id: String::new(),
                });
            }
        }
        let mut structure = Structure::build("helix".into(), "test".into(), atoms);
        for residue in structure.residues.iter_mut() {
            residue.secondary = SecondaryStructure::Helix;
        }
        structure
    }

    /// The ribbon faces a helix run gets, the same way `add_segment` derives them.
    fn round_helices(trace: &[TracePoint], tangents: &[[f32; 3]]) -> Vec<Option<[f32; 3]>> {
        let axes = fit_helix_axes(trace);
        (0..trace.len())
            .map(|index| {
                let axis = axes[index]?;
                let side = normalize(perpendicular_to(axis.direction, tangents[index]));
                side[0].is_finite().then_some(side)
            })
            .collect()
    }

    fn tangents_of(trace: &[TracePoint]) -> Vec<[f32; 3]> {
        (0..trace.len())
            .map(|index| {
                let previous = trace[index.saturating_sub(1)].position;
                let next = trace[(index + 1).min(trace.len() - 1)].position;
                normalize(sub(next, previous))
            })
            .collect()
    }

    #[test]
    fn the_helix_mesh_is_thin_across_the_radius_not_wide() {
        // The check that would have caught the screw thread. A cartoon helix is a band
        // 2.3 A wide *along the axis* and half an angstrom thick, so the mesh's spread in
        // radius from the axis should be about the thickness. Point the width radially
        // instead and the spread becomes the width - an auger, which is what this drew.
        let mut mesh = Mesh::default();
        let structure = ideal_helix_structure(24);
        let colors = white(&structure);
        build(&structure, &colors, &mut mesh, None, false);
        assert!(!mesh.is_empty(), "the fixture should produce a ribbon");

        // The middle of the helix only: the end caps are as wide as the ribbon by
        // definition, and the first and last residues take a one-sided tangent.
        let radii: Vec<f32> = (0..mesh.positions.len() / 3)
            .filter(|index| {
                let y = mesh.positions[index * 3 + 1];
                (6.0..28.0).contains(&y)
            })
            .map(|index| {
                let x = mesh.positions[index * 3];
                let z = mesh.positions[index * 3 + 2];
                (x * x + z * z).sqrt()
            })
            .collect();
        assert!(
            radii.len() > 100,
            "not enough of the middle: {}",
            radii.len()
        );
        let low = radii.iter().copied().fold(f32::INFINITY, f32::min);
        let high = radii.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let spread = high - low;
        // Thickness is 0.24 either side, plus a little from the rounded profile and the
        // band's tangential lean. Radially it would be the 2.3 A width instead.
        assert!(
            spread < 1.0,
            "the ribbon spreads {spread:.2} A in radius - it is lying radially"
        );
    }

    #[test]
    fn the_helix_path_is_round_not_a_polygon() {
        // A helix turns 100 degrees a residue, so a spline through its alpha carbons has
        // 3.6 points per turn and traces a rounded triangle: the radius from the axis dips
        // between the carbons. Generating the path *as* a helix keeps it constant, and
        // that is the difference between a round ribbon and an angular one.
        let trace = ideal_helix(16);
        let axes = fit_helix_axes(&trace);

        let mut radii = Vec::new();
        for index in 4..trace.len() - 4 {
            let (Some(from), Some(to)) = (axes[index], axes[index + 1]) else {
                continue;
            };
            for step in 0..8 {
                let t = step as f32 / 8.0;
                let point = helix_point(
                    from,
                    to,
                    trace[index].position,
                    trace[index + 1].position,
                    t,
                )
                .expect("a helical point");
                // The fixture's axis is z, so the radius is the distance from it.
                radii.push((point[0] * point[0] + point[1] * point[1]).sqrt());
            }
        }
        assert!(radii.len() > 40, "sampled {} points", radii.len());

        let low = radii.iter().copied().fold(f32::INFINITY, f32::min);
        let high = radii.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        // The fixture spirals at 2.3 A. A chord-cutting spline dips to about 1.9.
        assert!((low - 2.3).abs() < 0.1, "low {low}");
        assert!((high - 2.3).abs() < 0.1, "high {high}");
        assert!(high - low < 0.05, "the radius wobbles by {}", high - low);
    }

    #[test]
    fn a_kink_falls_back_to_the_spline() {
        // Two helices meeting at an angle have no shared axis, and forcing one would bend
        // the ribbon through the join. The spline is the right answer there.
        let mut trace = ideal_helix(8);
        for point in trace.iter_mut().skip(4) {
            // Fold the second half back on itself.
            point.position[2] = 6.0 - (point.position[2] - 6.0);
        }
        let axes = fit_helix_axes(&trace);
        if let (Some(from), Some(to)) = (axes[3], axes[4]) {
            if dot(from.direction, to.direction) < 0.9 {
                assert!(
                    helix_point(from, to, trace[3].position, trace[4].position, 0.5).is_none(),
                    "a kink should decline the helical path"
                );
            }
        }
    }

    #[test]
    fn a_helix_ribbon_is_broad_along_the_axis() {
        // The one that matters. The ribbon is 2.3 A wide and half an angstrom thick, so
        // the wide direction has to run *along* the helix: the band then spirals with its
        // flat face outward and the turns line up. Point it radially instead - which is
        // what happens if PyMOL's orientation vector is mistaken for the width direction -
        // and the helix comes out as a screw thread.
        let trace = ideal_helix(12);
        let sides = round_helices(&trace, &tangents_of(&trace));
        let mut seen = 0;
        // Interior residues only: the ends take a one-sided tangent, which tilts the face.
        for index in 2..trace.len() - 2 {
            let Some(side) = sides[index] else { continue };
            // The face is the axis with the path direction taken out of it, so for this
            // fixture it is about 0.83 along z and 0.55 along the circumference - the
            // band lies flat against the inclined path - and has no radial part at all.
            // Radial is what a screw thread looks like.
            assert!(
                side[2] > 0.7,
                "residue {index} is not along the axis: {side:?}"
            );
            let radial = normalize([trace[index].position[0], trace[index].position[1], 0.0]);
            assert!(
                dot(side, radial).abs() < 0.1,
                "residue {index} has a radial face: {side:?}"
            );
            seen += 1;
        }
        assert!(seen >= 6, "checked {seen} residues");
    }

    #[test]
    fn helix_faces_stay_together_between_residues() {
        // The face turns with the path, so consecutive residues are about 50 degrees
        // apart - the circumferential part of it rotates a hundred degrees a residue while
        // the axial part stays put. What matters is that the dot stays positive: the old
        // radial face was a hundred degrees apart with a *negative* dot, which is what let
        // a flip-to-agree rule invert every other frame.
        let trace = ideal_helix(12);
        let sides = round_helices(&trace, &tangents_of(&trace));
        for index in 3..trace.len() - 2 {
            if let (Some(a), Some(b)) = (sides[index - 1], sides[index]) {
                assert!(dot(a, b) > 0.3, "{index}: {:?} vs {:?}", a, b);
                let angle = dot(a, b).clamp(-1.0, 1.0).acos().to_degrees();
                assert!(angle < 60.0, "{index}: {angle} degrees apart");
            }
        }
    }

    #[test]
    fn the_helix_weights_land_on_the_axis() {
        // The derivation, checked rather than trusted. Two conditions define the weights:
        // they average (so twice their sum is one), and the radial components of four
        // consecutive alpha carbons cancel at the ideal twist.
        let (near, far) = axis_weights();
        assert!(
            (2.0 * (near + far) - 1.0).abs() < 1e-6,
            "an average, not a scaling: {near} {far}",
        );
        let twist = HELIX_TWIST_DEGREES.to_radians();
        let radial = far * (1.0 + (3.0 * twist).cos()) + near * (twist.cos() + (2.0 * twist).cos());
        assert!(radial.abs() < 1e-6, "radial parts must cancel: {radial}");
        // Which for a 100 degree twist works out at these, and the geometry below depends
        // on it: a regression guard, not a citation.
        assert!((near - 0.287).abs() < 5e-4, "{near}");
        assert!((far - 0.213).abs() < 5e-4, "{far}");

        // And a helix built from them has its faces along the axis.
        let trace = ideal_helix(8);
        let sides = round_helices(&trace, &tangents_of(&trace));
        // Every interior residue gets a face, leaning along z - the fixture's axis.
        for (index, side) in sides.iter().enumerate().take(6).skip(2) {
            let side = side.expect("a face");
            assert!(side[2] > 0.7, "residue {index}: {side:?}");
        }
    }

    #[test]
    fn fitting_the_axis_leaves_the_positions_alone() {
        // PyMOL writes only the orientation vectors. Smoothing the positions onto the axis
        // instead is what turned a broad spiralling ribbon into a tight corkscrew.
        let trace = ideal_helix(10);
        let before: Vec<[f32; 3]> = trace.iter().map(|point| point.position).collect();
        let _ = round_helices(&trace, &tangents_of(&trace));
        assert_eq!(
            trace.iter().map(|point| point.position).collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn a_face_is_perpendicular_to_the_path() {
        let trace = ideal_helix(9);
        let tangents = tangents_of(&trace);
        for (index, side) in round_helices(&trace, &tangents).iter().enumerate() {
            if let Some(side) = side {
                assert!(dot(*side, tangents[index]).abs() < 1e-3, "{index}");
                let length = dot(*side, *side).sqrt();
                assert!((length - 1.0).abs() < 1e-4, "{index}: {length}");
            }
        }
    }

    #[test]
    fn a_coil_gets_no_helix_face() {
        let mut trace = ideal_helix(8);
        for point in trace.iter_mut().skip(5) {
            point.secondary = SecondaryStructure::Coil;
        }
        let sides = round_helices(&trace, &tangents_of(&trace));
        assert!(
            sides[5..].iter().all(|side| side.is_none()),
            "a loop keeps its carbonyl face"
        );
    }

    #[test]
    fn a_short_helix_does_not_panic() {
        // Fewer than four residues cannot give an axis point at all.
        for count in 0..4 {
            let trace = ideal_helix(count);
            let sides = round_helices(&trace, &tangents_of(&trace));
            assert!(sides.iter().all(|side| side.is_none()), "{count}");
        }
    }
    use super::*;
    use crate::structure::model::RawAtom;

    /// A synthetic protein: `count` residues along a line, one CA and one O each.
    fn synthetic(
        count: usize,
        secondary: SecondaryStructure,
        break_before: Option<usize>,
    ) -> Structure {
        let mut atoms = Vec::new();
        let mut x = 0.0f32;
        for index in 0..count {
            if Some(index) == break_before {
                x += 30.0; // a gap no bond could span
            }
            for (name, offset) in [(" CA ", [0.0, 0.0, 0.0]), (" O  ", [0.0, 1.2, 0.5])] {
                atoms.push(RawAtom {
                    name: name.into(),
                    element: if name.trim() == "O" { 8 } else { 6 },
                    position: [x + offset[0], offset[1], offset[2]],
                    b_factor: 50.0,
                    occupancy: 1.0,
                    residue_name: "ALA".into(),
                    residue_number: index as i32 + 1,
                    insertion_code: None,
                    chain_id: "A".into(),
                    is_hetatm: false,
                    label_asym_id: String::new(),
                    entity_id: String::new(),
                });
            }
            x += 3.8; // the real CA-CA spacing
        }
        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        for residue in &mut structure.residues {
            residue.secondary = secondary;
        }
        structure
    }

    fn white(structure: &Structure) -> Vec<[f32; 3]> {
        vec![[1.0, 1.0, 1.0]; structure.atom_count()]
    }

    fn build_mesh(structure: &Structure) -> Mesh {
        let mut mesh = Mesh::default();
        build(structure, &white(structure), &mut mesh, None, false);
        mesh
    }

    #[test]
    fn produces_geometry_for_a_simple_chain() {
        let structure = synthetic(12, SecondaryStructure::Coil, None);
        let mesh = build_mesh(&structure);
        assert!(mesh.triangle_count() > 0);
        assert_eq!(mesh.positions.len(), mesh.normals.len());
        assert_eq!(mesh.vertex_count(), mesh.vertex_residue.len());
        // Every index must address a real vertex, or the draw call reads garbage.
        for index in &mesh.indices {
            assert!((*index as usize) < mesh.vertex_count());
        }
    }

    #[test]
    fn the_stadium_profile_puts_its_resolution_on_the_curved_edges() {
        // The artefact this fixes: an ellipse sampled at equal angles leaves notches along
        // a flattened ribbon's edges, because almost no samples land where the curvature
        // is. Measured as the largest gap between neighbouring outline points, which for
        // an arc-length walk is uniform.
        let outline = profile_outline(1.15, 0.24, 20);
        assert_eq!(outline.len(), 20);

        let mut gaps: Vec<f32> = Vec::new();
        for index in 0..outline.len() {
            let a = outline[index];
            let b = outline[(index + 1) % outline.len()];
            gaps.push(((a.side - b.side).powi(2) + (a.up - b.up).powi(2)).sqrt());
        }
        let longest = gaps.iter().copied().fold(0.0f32, f32::max);
        let shortest = gaps.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            longest / shortest < 1.6,
            "spacing varies {:.1}x, so the outline is not arc-length sampled",
            longest / shortest
        );

        // Normals are unit length and point outward.
        for point in &outline {
            let length = (point.normal_side.powi(2) + point.normal_up.powi(2)).sqrt();
            assert!((length - 1.0).abs() < 1e-4);
            assert!(
                point.side * point.normal_side + point.up * point.normal_up > -1e-4,
                "a normal points inward"
            );
        }
    }

    #[test]
    fn a_circular_cross_section_degenerates_cleanly() {
        // Coil is drawn as a tube, which is the stadium with no straight part at all.
        let outline = profile_outline(0.3, 0.3, 16);
        for point in &outline {
            let radius = (point.side.powi(2) + point.up.powi(2)).sqrt();
            assert!((radius - 0.3).abs() < 1e-4, "radius {radius} is not 0.3");
        }
    }

    #[test]
    fn detail_scales_down_for_large_assemblies() {
        let small = Detail::for_residue_count(76);
        let huge = Detail::for_residue_count(20000);
        assert!(small.subdivisions > huge.subdivisions);
        assert!(small.profile_points > huge.profile_points);
    }

    #[test]
    fn a_preview_is_never_finer_than_the_full_view() {
        for residues in [76, 2_000, 20_000, 60_000, 446_040] {
            let full = Detail::for_scene(residues, false);
            let preview = Detail::for_scene(residues, true);
            assert!(preview.subdivisions <= full.subdivisions, "{residues}");
            assert!(preview.profile_points <= full.profile_points, "{residues}");
        }
        // A capsid's thumbnail is a fraction of its full ribbon.
        let preview = Detail::for_scene(446_040, true);
        let full = Detail::for_scene(446_040, false);
        assert!(
            preview.subdivisions * preview.profile_points * 4
                <= full.subdivisions * full.profile_points * 3
        );
    }

    #[test]
    fn faces_wind_the_way_their_normals_point() {
        let mesh = build_mesh(&synthetic(10, SecondaryStructure::Helix, None));
        let mut opposed = 0;
        for triangle in 0..mesh.triangle_count() {
            let corner = |index: usize| {
                let vertex = mesh.indices[triangle * 3 + index] as usize;
                [
                    mesh.positions[vertex * 3],
                    mesh.positions[vertex * 3 + 1],
                    mesh.positions[vertex * 3 + 2],
                ]
            };
            let mut average = [0.0f32; 3];
            for index in 0..3 {
                let vertex = mesh.indices[triangle * 3 + index] as usize;
                for (axis, total) in average.iter_mut().enumerate() {
                    *total += mesh.normals[vertex * 3 + axis];
                }
            }
            let face = cross(sub(corner(1), corner(0)), sub(corner(2), corner(0)));
            if dot(face, average) < 0.0 {
                opposed += 1;
            }
        }
        assert_eq!(
            opposed,
            0,
            "{opposed} of {} faces are inside out",
            mesh.triangle_count()
        );
    }

    #[test]
    fn normals_are_unit_length() {
        // A non-unit normal shows up as a patch of ribbon lit differently from the rest.
        let structure = synthetic(10, SecondaryStructure::Helix, None);
        let mesh = build_mesh(&structure);
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
    fn does_not_span_a_chain_break() {
        // The visible failure: a straight bar across the structure where residues are
        // missing. No vertex may land inside the gap.
        let structure = synthetic(12, SecondaryStructure::Coil, Some(6));
        let mesh = build_mesh(&structure);
        let before = 5.0 * 3.8;
        let after = before + 30.0 + 3.8;
        for vertex in 0..mesh.vertex_count() {
            let x = mesh.positions[vertex * 3];
            assert!(
                !(x > before + 3.0 && x < after - 3.0),
                "a vertex at x={x} sits inside the chain break"
            );
        }
    }

    #[test]
    fn a_helix_ribbon_is_wider_than_a_coil_tube() {
        // The point of the representation: secondary structure must be visible as a change
        // in shape, not only in colour.
        let helix = build_mesh(&synthetic(10, SecondaryStructure::Helix, None));
        let coil = build_mesh(&synthetic(10, SecondaryStructure::Coil, None));
        assert!(radial_extent(&helix) > radial_extent(&coil) * 2.0);
    }

    /// Every ring of the ribbon is a ring: finite, and spread over two directions rather
    /// than collapsed onto a line, which is what a side vector equal to the tangent makes.
    fn assert_round_rings(structure: &Structure, what: &str) {
        let mesh = build_mesh(structure);
        assert!(
            mesh.positions
                .iter()
                .chain(&mesh.normals)
                .all(|value| value.is_finite()),
            "{what}: a NaN reached the ribbon"
        );
        let points = Detail::for_residue_count(structure.residues.len()).profile_points;
        let vertex = |index: usize| {
            [
                mesh.positions[index * 3],
                mesh.positions[index * 3 + 1],
                mesh.positions[index * 3 + 2],
            ]
        };
        for ring in 0..mesh.vertex_count() / points {
            let base = ring * points;
            let mut centre = [0.0f32; 3];
            for index in base..base + points {
                centre = [
                    centre[0] + vertex(index)[0],
                    centre[1] + vertex(index)[1],
                    centre[2] + vertex(index)[2],
                ];
            }
            let centre = scaled(centre, 1.0 / points as f32);
            let spoke_a = sub(vertex(base), centre);
            let spoke_b = sub(vertex(base + points / 4), centre);
            assert!(
                length(cross(spoke_a, spoke_b)) > 1e-3,
                "{what}: ring {ring} is flat - {spoke_a:?} and {spoke_b:?}"
            );
        }
    }

    #[test]
    fn short_and_degenerate_chains_still_build_round_ribbons() {
        for count in 2..=3 {
            for secondary in [
                SecondaryStructure::Coil,
                SecondaryStructure::Helix,
                SecondaryStructure::Sheet,
            ] {
                let structure = synthetic(count, secondary, None);
                assert_round_rings(&structure, &format!("{count} residues of {secondary:?}"));
            }
        }

        // Carbonyls pointing straight along the chain: the face has no part across it.
        let mut along = synthetic(6, SecondaryStructure::Coil, None);
        for residue in along.residues.clone() {
            let (trace, orient) = (residue.trace_atom as usize, residue.orient_atom as usize);
            along.positions[orient * 3] = along.positions[trace * 3] + 1.2;
            along.positions[orient * 3 + 1] = along.positions[trace * 3 + 1];
            along.positions[orient * 3 + 2] = along.positions[trace * 3 + 2];
        }
        assert_round_rings(&along, "carbonyls along the chain");

        // Carbonyls on top of their alpha carbons.
        let mut stacked = synthetic(6, SecondaryStructure::Sheet, None);
        for residue in stacked.residues.clone() {
            let (trace, orient) = (residue.trace_atom as usize, residue.orient_atom as usize);
            for axis in 0..3 {
                stacked.positions[orient * 3 + axis] = stacked.positions[trace * 3 + axis];
            }
        }
        assert_round_rings(&stacked, "carbonyls on their alpha carbons");

        // Two residues at one point, and a chain reversing on itself.
        let mut doubled = synthetic(5, SecondaryStructure::Coil, None);
        let (first, second) = (
            doubled.residues[1].trace_atom as usize,
            doubled.residues[2].trace_atom as usize,
        );
        for axis in 0..3 {
            doubled.positions[second * 3 + axis] = doubled.positions[first * 3 + axis];
        }
        let mesh = build_mesh(&doubled);
        assert!(mesh
            .positions
            .iter()
            .chain(&mesh.normals)
            .all(|value| value.is_finite()));
    }

    #[test]
    fn a_chain_of_one_residue_has_no_ribbon_and_is_not_covered() {
        let structure = synthetic(1, SecondaryStructure::Helix, None);
        assert_eq!(build_mesh(&structure).triangle_count(), 0);
        assert_eq!(covered(&structure), vec![false]);
        // And a chain broken into single residues leaves each of them uncovered.
        let broken = synthetic(3, SecondaryStructure::Coil, Some(1));
        let flags = covered(&broken);
        assert!(!flags[0], "the residue before the break stands alone");
    }

    #[test]
    fn colours_shorter_than_the_atoms_do_not_panic() {
        // The colour array comes from a different pass; a mismatch draws grey rather than
        // taking the viewport down.
        let structure = synthetic(6, SecondaryStructure::Helix, None);
        let mut mesh = Mesh::default();
        build(&structure, &[], &mut mesh, None, false);
        assert!(mesh.triangle_count() > 0);
        assert!(mesh.colors.iter().all(|value| value.is_finite()));
        // A chain filter naming no chain builds nothing.
        let mut mesh = Mesh::default();
        build(&structure, &white(&structure), &mut mesh, Some(&[7]), false);
        assert!(mesh.is_empty());
    }

    #[test]
    fn residues_without_a_trace_atom_are_skipped() {
        let mut structure = synthetic(8, SecondaryStructure::Coil, None);
        for residue in &mut structure.residues {
            residue.trace_atom = -1;
        }
        assert_eq!(build_mesh(&structure).triangle_count(), 0);
    }

    #[test]
    fn waters_and_ligands_are_skipped() {
        let mut structure = synthetic(8, SecondaryStructure::Coil, None);
        for residue in &mut structure.residues {
            residue.kind = ResidueKind::Water;
        }
        assert_eq!(build_mesh(&structure).triangle_count(), 0);
    }

    #[test]
    fn a_frame_flip_does_not_shred_the_ribbon() {
        // Alternating carbonyl directions are what a helix looks like to the builder. If
        // frames are not aligned to their predecessor, consecutive rings face opposite
        // ways and the surface self-intersects. Measured as the largest step between
        // consecutive ring centres' vertex 0, which must stay on the order of the spline
        // step rather than jumping the ribbon's full width.
        let mut structure = synthetic(12, SecondaryStructure::Helix, None);
        for (index, residue) in structure.residues.iter().enumerate() {
            let orient = residue.orient_atom as usize;
            // Flip every other carbonyl to the opposite side.
            if index % 2 == 1 {
                structure.positions[orient * 3 + 1] *= -1.0;
            }
        }
        let mesh = build_mesh(&structure);

        let mut worst = 0.0f32;
        let points = Detail::for_residue_count(structure.residues.len()).profile_points;
        for ring in 1..mesh.vertex_count() / points {
            let a = (ring - 1) * points;
            let b = ring * points;
            let step = distance(
                [
                    mesh.positions[a * 3],
                    mesh.positions[a * 3 + 1],
                    mesh.positions[a * 3 + 2],
                ],
                [
                    mesh.positions[b * 3],
                    mesh.positions[b * 3 + 1],
                    mesh.positions[b * 3 + 2],
                ],
            );
            worst = worst.max(step);
        }
        // The spline advances ~0.63 A per ring; a shredded ribbon jumps by twice the
        // helix width, over 2 A.
        assert!(worst < 1.5, "consecutive rings jump by {worst} A");
    }

    /// Widest the ribbon gets, measured from the chain axis - the ribbon's face is
    /// oriented by the carbonyl, so which axis carries its width varies.
    fn radial_extent(mesh: &Mesh) -> f32 {
        let mut widest = 0.0f32;
        for vertex in 0..mesh.vertex_count() {
            let y = mesh.positions[vertex * 3 + 1];
            let z = mesh.positions[vertex * 3 + 2];
            widest = widest.max((y * y + z * z).sqrt());
        }
        widest
    }

    #[test]
    fn every_residue_of_a_helix_has_an_axis_level_with_it() {
        let trace = ideal_helix(12);
        let axes = fit_helix_axes(&trace);
        for (index, axis) in axes.iter().enumerate() {
            let axis = axis.unwrap_or_else(|| panic!("residue {index} has no axis"));
            // The helix is built about one straight line, so every centre is on it...
            let off_axis =
                perpendicular_to(sub(axis.centre, axes[5].unwrap().centre), axis.direction);
            assert!(
                length(off_axis) < 0.05,
                "residue {index} is {off_axis:?} off the axis"
            );
            // ...and each is level with its own alpha carbon, not half a rise along.
            let along = dot(sub(trace[index].position, axis.centre), axis.direction);
            assert!(
                along.abs() < 1e-3,
                "residue {index} sits {along} A from its centre"
            );
        }
    }

    #[test]
    fn the_helical_path_passes_through_the_alpha_carbons() {
        let trace = ideal_helix(12);
        let axes = fit_helix_axes(&trace);
        for index in 0..trace.len() - 1 {
            let (from, to) = (axes[index].unwrap(), axes[index + 1].unwrap());
            let (start, end) = (trace[index].position, trace[index + 1].position);
            let at_start = helix_point(from, to, start, end, 0.0).expect("a shared axis");
            let at_end = helix_point(from, to, start, end, 1.0).expect("a shared axis");
            assert!(
                distance(at_start, start) < 0.02,
                "{index}: starts {} A off",
                distance(at_start, start)
            );
            assert!(
                distance(at_end, end) < 0.02,
                "{index}: ends {} A off",
                distance(at_end, end)
            );
        }
    }

    #[test]
    fn a_nucleic_acid_strand_is_one_segment() {
        use crate::structure::model::RawAtom;
        // Phosphates 6.8 A apart, as in B-form DNA: past the protein limit, inside this one.
        let atoms: Vec<RawAtom> = (0..6)
            .flat_map(|index| {
                [(" P  ", 15u8, 0.0f32), (" C1'", 6, 3.0)].map(|(name, element, lift)| RawAtom {
                    name: name.into(),
                    element,
                    position: [6.8 * index as f32, lift, 0.0],
                    b_factor: 20.0,
                    occupancy: 1.0,
                    residue_name: "DA".into(),
                    residue_number: index + 1,
                    insertion_code: None,
                    chain_id: "A".into(),
                    is_hetatm: false,
                    label_asym_id: String::new(),
                    entity_id: String::new(),
                })
            })
            .collect();
        let structure = Structure::build("dna".into(), "test".into(), atoms);
        let segments = trace_segments(&structure);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].len(), 6);
        assert!(covered(&structure).iter().all(|covered| *covered));

        let mut mesh = Mesh::default();
        let colors = vec![[1.0, 1.0, 1.0]; structure.atom_count()];
        build(&structure, &colors, &mut mesh, None, false);
        assert!(mesh.triangle_count() > 0, "the strand draws a ribbon");
    }

    #[test]
    fn a_nucleotide_with_its_base_gets_a_rod_and_a_slab() {
        use crate::structure::model::RawAtom;
        // Two nucleotides: a backbone only, and one with a pyrimidine's ring beside it.
        let atom = |name: &str, element: u8, position: [f32; 3], number: i32| RawAtom {
            name: name.into(),
            element,
            position,
            b_factor: 20.0,
            occupancy: 1.0,
            residue_name: "DC".into(),
            residue_number: number,
            insertion_code: None,
            chain_id: "A".into(),
            is_hetatm: false,
            label_asym_id: String::new(),
            entity_id: String::new(),
        };
        let backbone = |number: i32, x: f32| {
            vec![
                atom("P", 15, [x, 0.0, 0.0], number),
                atom("C1'", 6, [x, 3.0, 0.0], number),
            ]
        };
        let ring: Vec<RawAtom> = ["N1", "C2", "N3", "C4", "C5", "C6"]
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let angle = index as f32 / 6.0 * std::f32::consts::TAU;
                atom(
                    name,
                    if name.starts_with('N') { 7 } else { 6 },
                    [6.8 + 1.4 * angle.cos(), 6.0 + 1.4 * angle.sin(), 0.0],
                    2,
                )
            })
            .collect();

        let bare = Structure::build(
            "dna".into(),
            "test".into(),
            [backbone(1, 0.0), backbone(2, 6.8)].concat(),
        );
        let based = Structure::build(
            "dna".into(),
            "test".into(),
            [backbone(1, 0.0), backbone(2, 6.8), ring].concat(),
        );
        let triangles = |structure: &Structure| {
            let mut mesh = Mesh::default();
            build(
                structure,
                &vec![[1.0, 1.0, 1.0]; structure.atom_count()],
                &mut mesh,
                None,
                false,
            );
            (mesh.triangle_count(), mesh)
        };
        let (plain, _) = triangles(&bare);
        let (with_base, mesh) = triangles(&based);
        // A rod of ten sides, and a hexagonal slab: two fans of four and six walls of two.
        assert_eq!(with_base - plain, 20 + 8 + 12);
        // The slab reaches the ring, which the backbone alone never does.
        let highest = mesh
            .positions
            .chunks(3)
            .map(|p| p[1])
            .fold(f32::MIN, f32::max);
        assert!(
            highest > 7.0,
            "the base is drawn where the ring is, not just the tube: {highest}"
        );
    }
}
