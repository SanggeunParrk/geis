//! A molecule's envelope: the smooth outer surface the glass view draws over its ribbons.
//!
//! The surface is where a sum of soft blobs - one round each atom, as big as the atom -
//! crosses a level: the "Gaussian surface" of VMD's QuickSurf and Mol*, which reads as a
//! molecular surface at a glance without the cost of a solvent-excluded one. The blobs are
//! summed on a grid, and the surface is taken from the grid by surface nets: a vertex in
//! each cell the surface passes through, at the mean of where it crosses the cell's edges,
//! and a quad across each edge it crosses. That gives a mesh with no seams and good normals,
//! and no lookup tables.
//!
//! A large structure is summed by residue instead of by atom - one larger blob each - so a
//! capsid's envelope takes about as long as a single protein's.

use super::mesh::Mesh;
use crate::structure::Structure;

/// One envelope: vertices with their normals and which blob each is nearest, and
/// triangles.
#[derive(Debug, Default)]
pub struct Envelope {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    /// For each vertex, the blob nearest it: what it is coloured by.
    pub chain: Vec<u32>,
    pub indices: Vec<u32>,
    pub chains: u32,
}

/// A blob: where, how big, and what colour.
struct Blob {
    at: [f32; 3],
    radius: f32,
    colour: [f32; 3],
}

/// How sharp the blobs are: lower is smoother. VMD uses about 2.4 for a molecular surface;
/// the glass view wants a softer one.
const SHARPNESS: f32 = 1.6;
/// The grid has at most this many points, whatever the structure's size.
const MOST_POINTS: f64 = 6_000_000.0;

/// The blobs for these atoms: one each, or - for a large structure - one per residue, at
/// the middle of its atoms here, as big as they are together, in its first atom's colour.
fn blobs(structure: &Structure, atoms: &[usize], colour: &dyn Fn(usize) -> [f32; 3]) -> Vec<Blob> {
    let atoms: Vec<usize> = atoms
        .iter()
        .copied()
        .filter(|&atom| atom < structure.atom_count() && structure.elements[atom] != 1)
        .collect();
    if atoms.len() > 120_000 {
        let mut out: Vec<Blob> = Vec::new();
        let mut current: Option<(u32, Vec<usize>)> = None;
        let flush = |group: Option<(u32, Vec<usize>)>, out: &mut Vec<Blob>| {
            let Some((_, members)) = group else { return };
            let mut at = [0.0f32; 3];
            for &atom in &members {
                let p = structure.position(atom);
                for axis in 0..3 {
                    at[axis] += p[axis] / members.len() as f32;
                }
            }
            out.push(Blob {
                at,
                radius: 1.8 * (members.len() as f32).cbrt(),
                colour: colour(members[0]),
            });
        };
        for atom in atoms {
            let residue = structure.residue_of_atom[atom];
            match &mut current {
                Some((at, members)) if *at == residue => members.push(atom),
                _ => {
                    flush(current.take(), &mut out);
                    current = Some((residue, vec![atom]));
                }
            }
        }
        flush(current, &mut out);
        return out;
    }
    atoms
        .into_iter()
        .map(|atom| Blob {
            at: structure.position(atom),
            radius: crate::structure::chem::vdw_radius(structure.elements[atom]),
            colour: colour(atom),
        })
        .collect()
}

/// The glass over these atoms of a structure, as a mesh whose vertices take the colour of
/// the atom nearest them: a chain's colour where the atoms are coloured by chain, and any
/// colour given to some of them carried onto the glass over them.
pub fn envelope_of(
    structure: &Structure,
    atoms: &[usize],
    colour: &dyn Fn(usize) -> [f32; 3],
) -> Mesh {
    let blobs = blobs(structure, atoms, colour);
    let envelope = envelope_from(&blobs);
    let mut mesh = Mesh {
        positions: envelope.positions,
        normals: envelope.normals,
        indices: envelope.indices,
        ..Mesh::default()
    };
    mesh.colors = envelope
        .chain
        .iter()
        .flat_map(|&blob| blobs.get(blob as usize).map_or([0.8; 3], |blob| blob.colour))
        .collect();
    mesh
}

/// The surface round a set of blobs; each vertex's `chain` is the index of the blob
/// nearest it.
fn envelope_from(blobs: &[Blob]) -> Envelope {
    if blobs.is_empty() {
        return Envelope::default();
    }
    let widest = blobs.iter().map(|blob| blob.radius).fold(0.0f32, f32::max);
    // Each blob reaches this many of its radii before it is too faint to matter.
    let reach_factor = 2.2f32;
    let pad = widest * reach_factor + 2.0;
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for blob in blobs {
        for axis in 0..3 {
            low[axis] = low[axis].min(blob.at[axis] - pad);
            high[axis] = high[axis].max(blob.at[axis] + pad);
        }
    }
    let volume = (0..3)
        .map(|axis| f64::from(high[axis] - low[axis]))
        .product::<f64>();
    // Fine for a protein, coarser for an assembly: the grid is the same size either way.
    let spacing = ((volume / MOST_POINTS).cbrt() as f32).max(0.9);
    let dims =
        [0, 1, 2].map(|axis| (((high[axis] - low[axis]) / spacing).ceil() as usize).max(2) + 1);
    let point = |i: usize, j: usize, k: usize| {
        [
            low[0] + i as f32 * spacing,
            low[1] + j as f32 * spacing,
            low[2] + k as f32 * spacing,
        ]
    };

    // The blobs, bucketed by cell of the grid of their reach, so each point asks only the
    // blobs near it.
    let bucket = widest * reach_factor;
    let key = |p: [f32; 3]| [0, 1, 2].map(|axis| ((p[axis] - low[axis]) / bucket).floor() as i32);
    let mut buckets: std::collections::HashMap<[i32; 3], Vec<u32>> =
        std::collections::HashMap::new();
    for (index, blob) in blobs.iter().enumerate() {
        buckets.entry(key(blob.at)).or_default().push(index as u32);
    }

    // The field, and the nearest blob's chain, slab by slab across the processor's cores.
    let (nx, ny, nz) = (dims[0], dims[1], dims[2]);
    let slab = |k: usize| -> (Vec<f32>, Vec<u32>) {
        let mut field = vec![0.0f32; nx * ny];
        let mut nearest = vec![0u32; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                let p = point(i, j, k);
                let home = key(p);
                let mut sum = 0.0f32;
                let mut best = f32::INFINITY;
                let mut chain = 0;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let Some(found) =
                                buckets.get(&[home[0] + dx, home[1] + dy, home[2] + dz])
                            else {
                                continue;
                            };
                            for &index in found {
                                let blob = &blobs[index as usize];
                                let d2 = (p[0] - blob.at[0]).powi(2)
                                    + (p[1] - blob.at[1]).powi(2)
                                    + (p[2] - blob.at[2]).powi(2);
                                let r2 = blob.radius * blob.radius;
                                if d2 > r2 * reach_factor * reach_factor {
                                    continue;
                                }
                                // Blinn's blob: 1 at the atom's surface, more inside, less out.
                                sum += (-SHARPNESS * (d2 / r2 - 1.0)).exp();
                                let scaled = d2 / r2;
                                if scaled < best {
                                    best = scaled;
                                    chain = index;
                                }
                            }
                        }
                    }
                }
                field[j * nx + i] = sum;
                nearest[j * nx + i] = chain;
            }
        }
        (field, nearest)
    };
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let mut field = vec![0.0f32; nx * ny * nz];
    let mut nearest = vec![0u32; nx * ny * nz];
    std::thread::scope(|scope| {
        let slab = &slab;
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                scope.spawn(move || {
                    (worker..nz)
                        .step_by(workers)
                        .map(|k| (k, slab(k)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            for (k, (slab_field, slab_nearest)) in handle.join().unwrap_or_default() {
                field[k * nx * ny..(k + 1) * nx * ny].copy_from_slice(&slab_field);
                nearest[k * nx * ny..(k + 1) * nx * ny].copy_from_slice(&slab_nearest);
            }
        }
    });
    // Blurred before it is meshed: the sum of atom-sized blobs is lumpy at the scale of an
    // atom, and under glass every lump is two highlights and a bright crease where it meets
    // the next - the shell read as a bag of marbles. Blurred over about two angstroms it is
    // one smooth pebble, which is what glass round a molecule looks like in an
    // illustration. Not where the grid is coarser than the blur - an assembly's, which is
    // smooth at that scale already.
    let sigma = BLUR / spacing;
    if sigma > 0.6 {
        blur(&mut field, dims, sigma);
    }
    surface_nets(&field, &nearest, dims, low, spacing, blobs.len() as u32)
}

/// How far the field is blurred, in angstroms: the deviation of the Gaussian.
const BLUR: f32 = 1.8;

/// A Gaussian blur of the grid, one axis at a time.
fn blur(field: &mut [f32], dims: [usize; 3], sigma: f32) {
    let reach = (2.5 * sigma).ceil() as isize;
    let kernel: Vec<f32> = (-reach..=reach)
        .map(|d| (-(d * d) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.iter().map(|k| k / total).collect();
    let [nx, ny, nz] = dims;
    let strides = [1, nx, nx * ny];
    let mut line = Vec::new();
    for axis in 0..3 {
        let n = dims[axis];
        let stride = strides[axis];
        // Every line along this axis, by where it starts.
        let starts: Vec<usize> = match axis {
            0 => (0..nz)
                .flat_map(|k| (0..ny).map(move |j| (k * ny + j) * nx))
                .collect(),
            1 => (0..nz)
                .flat_map(|k| (0..nx).map(move |i| k * nx * ny + i))
                .collect(),
            _ => (0..ny)
                .flat_map(|j| (0..nx).map(move |i| j * nx + i))
                .collect(),
        };
        for start in starts {
            line.clear();
            line.extend((0..n).map(|s| field[start + s * stride]));
            for s in 0..n {
                let mut sum = 0.0;
                for (offset, weight) in kernel.iter().enumerate() {
                    let at = (s as isize + offset as isize - reach).clamp(0, n as isize - 1);
                    sum += weight * line[at as usize];
                }
                field[start + s * stride] = sum;
            }
        }
    }
}

/// The surface where `field` crosses 1, by surface nets.
fn surface_nets(
    field: &[f32],
    nearest: &[u32],
    dims: [usize; 3],
    low: [f32; 3],
    spacing: f32,
    chains: u32,
) -> Envelope {
    const LEVEL: f32 = 1.0;
    let [nx, ny, nz] = dims;
    let at = |i: usize, j: usize, k: usize| field[(k * ny + j) * nx + i];
    let mut vertex_of = vec![u32::MAX; nx * ny * nz];
    let mut out = Envelope {
        chains,
        ..Envelope::default()
    };
    // The eight corners of a cell, and its twelve edges by the corners they join.
    const CORNERS: [[usize; 3]; 8] = [
        [0, 0, 0],
        [1, 0, 0],
        [0, 1, 0],
        [1, 1, 0],
        [0, 0, 1],
        [1, 0, 1],
        [0, 1, 1],
        [1, 1, 1],
    ];
    const EDGES: [[usize; 2]; 12] = [
        [0, 1],
        [2, 3],
        [4, 5],
        [6, 7],
        [0, 2],
        [1, 3],
        [4, 6],
        [5, 7],
        [0, 4],
        [1, 5],
        [2, 6],
        [3, 7],
    ];
    let gradient = |i: usize, j: usize, k: usize| -> [f32; 3] {
        let g = |a: usize, b: usize, c: usize| at(a.min(nx - 1), b.min(ny - 1), c.min(nz - 1));
        [
            g(i + 1, j, k) - g(i.saturating_sub(1), j, k),
            g(i, j + 1, k) - g(i, j.saturating_sub(1), k),
            g(i, j, k + 1) - g(i, j, k.saturating_sub(1)),
        ]
    };
    for k in 0..nz - 1 {
        for j in 0..ny - 1 {
            for i in 0..nx - 1 {
                let values: [f32; 8] = CORNERS.map(|c| at(i + c[0], j + c[1], k + c[2]));
                let inside = values.iter().filter(|&&v| v > LEVEL).count();
                if inside == 0 || inside == 8 {
                    continue;
                }
                // The mean of where the level crosses this cell's edges.
                let mut sum = [0.0f32; 3];
                let mut crossings = 0.0f32;
                for edge in EDGES {
                    let (a, b) = (values[edge[0]], values[edge[1]]);
                    if (a > LEVEL) == (b > LEVEL) {
                        continue;
                    }
                    let t = (LEVEL - a) / (b - a);
                    let (ca, cb) = (CORNERS[edge[0]], CORNERS[edge[1]]);
                    for axis in 0..3 {
                        sum[axis] += ca[axis] as f32 + (cb[axis] as f32 - ca[axis] as f32) * t;
                    }
                    crossings += 1.0;
                }
                let local = sum.map(|s| s / crossings);
                let position = [
                    low[0] + (i as f32 + local[0]) * spacing,
                    low[1] + (j as f32 + local[1]) * spacing,
                    low[2] + (k as f32 + local[2]) * spacing,
                ];
                // Outward is down the field's slope, taken where the vertex actually is -
                // the eight corners' slopes blended - rather than at the nearest corner,
                // which gave every vertex in a patch the same normal and the glass facets.
                let mut g = [0.0f32; 3];
                for c in &CORNERS {
                    let weight = (0..3)
                        .map(|axis| {
                            if c[axis] == 1 {
                                local[axis]
                            } else {
                                1.0 - local[axis]
                            }
                        })
                        .product::<f32>();
                    let slope = gradient(i + c[0], j + c[1], k + c[2]);
                    for axis in 0..3 {
                        g[axis] += weight * slope[axis];
                    }
                }
                let length = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-6);
                vertex_of[(k * ny + j) * nx + i] = (out.positions.len() / 3) as u32;
                out.positions.extend_from_slice(&position);
                out.normals
                    .extend_from_slice(&[-g[0] / length, -g[1] / length, -g[2] / length]);
                let corner = CORNERS[values
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .map_or(0, |(index, _)| index)];
                out.chain
                    .push(nearest[((k + corner[2]) * ny + (j + corner[1])) * nx + i + corner[0]]);
            }
        }
    }
    // A quad across every grid edge the level crosses, joining the four cells round it,
    // turned so it faces out.
    let vertex = |i: usize, j: usize, k: usize| vertex_of[(k * ny + j) * nx + i];
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                let here = at(i, j, k) > LEVEL;
                for axis in 0..3 {
                    let (i2, j2, k2) = match axis {
                        0 => (i + 1, j, k),
                        1 => (i, j + 1, k),
                        _ => (i, j, k + 1),
                    };
                    if i2 >= nx || j2 >= ny || k2 >= nz {
                        continue;
                    }
                    if here == (at(i2, j2, k2) > LEVEL) {
                        continue;
                    }
                    // The four cells sharing this edge.
                    let cells: Option<[u32; 4]> = match axis {
                        0 if j > 0 && k > 0 => Some([
                            vertex(i, j - 1, k - 1),
                            vertex(i, j, k - 1),
                            vertex(i, j, k),
                            vertex(i, j - 1, k),
                        ]),
                        1 if i > 0 && k > 0 => Some([
                            vertex(i - 1, j, k - 1),
                            vertex(i - 1, j, k),
                            vertex(i, j, k),
                            vertex(i, j, k - 1),
                        ]),
                        2 if i > 0 && j > 0 => Some([
                            vertex(i - 1, j - 1, k),
                            vertex(i, j - 1, k),
                            vertex(i, j, k),
                            vertex(i - 1, j, k),
                        ]),
                        _ => None,
                    };
                    let Some(mut quad) = cells else { continue };
                    if quad.contains(&u32::MAX) {
                        continue;
                    }
                    if !here {
                        quad.reverse();
                    }
                    out.indices
                        .extend_from_slice(&[quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_round_field_gives_a_closed_sphere_facing_out() {
        // A field that is the distance from the middle, inverted: a sphere of radius 5.
        let n = 21;
        let mut field = vec![0.0f32; n * n * n];
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    let d = ((i as f32 - 10.0).powi(2)
                        + (j as f32 - 10.0).powi(2)
                        + (k as f32 - 10.0).powi(2))
                    .sqrt();
                    field[(k * n + j) * n + i] = 6.0 - d;
                }
            }
        }
        let envelope = surface_nets(&field, &vec![0; n * n * n], [n, n, n], [0.0; 3], 1.0, 1);
        assert!(!envelope.indices.is_empty());
        let vertices = envelope.positions.len() / 3;
        for v in 0..vertices {
            let p = &envelope.positions[v * 3..v * 3 + 3];
            let d = ((p[0] - 10.0).powi(2) + (p[1] - 10.0).powi(2) + (p[2] - 10.0).powi(2)).sqrt();
            assert!((d - 5.0).abs() < 0.3, "vertex {v} at {d} from the middle");
            // Normals point away from the middle.
            let normal = &envelope.normals[v * 3..v * 3 + 3];
            let out = [(p[0] - 10.0) / d, (p[1] - 10.0) / d, (p[2] - 10.0) / d];
            assert!(normal[0] * out[0] + normal[1] * out[1] + normal[2] * out[2] > 0.8);
        }
        // Triangles wind outwards: their face normal agrees with the vertex normals.
        for triangle in envelope.indices.chunks(3) {
            let corner = |index: u32| {
                [
                    envelope.positions[index as usize * 3],
                    envelope.positions[index as usize * 3 + 1],
                    envelope.positions[index as usize * 3 + 2],
                ]
            };
            let (a, b, c) = (
                corner(triangle[0]),
                corner(triangle[1]),
                corner(triangle[2]),
            );
            let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let face = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let middle = [
                (a[0] + b[0] + c[0]) / 3.0 - 10.0,
                (a[1] + b[1] + c[1]) / 3.0 - 10.0,
                (a[2] + b[2] + c[2]) / 3.0 - 10.0,
            ];
            assert!(face[0] * middle[0] + face[1] * middle[1] + face[2] * middle[2] > 0.0);
        }
    }
}
