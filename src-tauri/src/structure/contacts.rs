//! Contacts between atoms that are not bonds: what holds a fold and a complex together.
//!
//! Each kind is found by the criteria the field's own tools publish, on heavy atoms - a
//! deposited structure has no hydrogens - and the criteria are named where they are used
//! so a result can be checked against them:
//!
//! - **Hydrogen bonds** after HBPLUS (McDonald & Thornton 1994) in its heavy-atom form:
//!   a donor and an acceptor within 3.5 A, with the angles at both ends (antecedent -
//!   acceptor ... donor, and antecedent - donor ... acceptor) at least 90 degrees. Which
//!   atoms can donate and accept is by residue and atom name for the standard amino
//!   acids, and any N or O for everything else.
//! - **Salt bridges** after Barlow & Thornton (1983): a carboxylate oxygen of Asp or Glu
//!   within 4.0 A of a charged nitrogen of Lys, Arg or His.
//! - **Pi stacking** after PLIP (Salentin et al. 2015): two aromatic ring centres within
//!   5.5 A, their planes within 30 degrees of parallel with the centres offset by no more
//!   than 2.0 A, or between 60 and 90 degrees apart (T-shaped).
//! - **Hydrophobic contacts** after PLIP: carbon atoms of hydrophobic side chains, in
//!   different residues, within 4.0 A.
//!
//! These are geometric candidates, not energies: no hydrogens are placed, no protonation
//! state is worked out, and a histidine is taken to be able to do everything it ever can.

use super::bonds::neighbour_cell;
use super::model::{ResidueKind, Structure};
use serde::Serialize;

/// The kinds of contact this can look for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContactKind {
    /// Hydrogen bonds, by distance and both angles.
    Polar,
    SaltBridge,
    PiStacking,
    Hydrophobic,
}

#[derive(Debug, Clone, Serialize)]
pub struct Contact {
    pub residues: [u32; 2],
    /// Each end as it is cited: `ASP 52 OD1 (A_1)`, or `PHE 43 ring (A_1)`.
    pub names: [String; 2],
    pub distance: f32,
    /// Where the two ends are: atoms, or ring centres.
    pub from: [f32; 3],
    pub to: [f32; 3],
}

type Vec3 = [f32; 3];
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// The angle at `at` between `a` and `b`, in degrees.
fn angle(a: Vec3, at: Vec3, b: Vec3) -> f32 {
    let (u, v) = (sub(a, at), sub(b, at));
    let scale = length(u) * length(v);
    if scale < 1e-6 {
        return 180.0;
    }
    (dot(u, v) / scale).clamp(-1.0, 1.0).acos().to_degrees()
}

const HYDROPHOBIC: &[&str] = &[
    "ALA", "VAL", "LEU", "ILE", "MET", "PHE", "TRP", "PRO", "TYR", "CYS",
];

/// Whether an atom can donate a hydrogen bond, accept one, or both.
fn polar_role(structure: &Structure, atom: usize) -> (bool, bool) {
    let element = structure.elements[atom];
    if element != 7 && element != 8 {
        return (false, false);
    }
    let residue = &structure.residues[structure.residue_of_atom[atom] as usize];
    let name = structure.atom_names[atom].trim();
    if residue.kind != ResidueKind::AminoAcid {
        // A ligand, a nucleotide, a water: no table to go by, so either.
        return (true, true);
    }
    match (residue.name.trim(), name) {
        ("PRO", "N") => (false, false),
        (_, "N") => (true, false),
        (_, "O" | "OXT") => (false, true),
        ("SER", "OG") | ("THR", "OG1") | ("TYR", "OH") | ("HIS", "ND1" | "NE2") => (true, true),
        ("ASN", "OD1") | ("GLN", "OE1") | ("ASP", "OD1" | "OD2") | ("GLU", "OE1" | "OE2") => {
            (false, true)
        }
        ("ASN", "ND2")
        | ("GLN", "NE2")
        | ("LYS", "NZ")
        | ("ARG", "NE" | "NH1" | "NH2")
        | ("TRP", "NE1") => (true, false),
        // A modified residue's own atoms: unknown, so either.
        _ => (true, true),
    }
}

fn salt_role(structure: &Structure, atom: usize) -> u8 {
    let residue = &structure.residues[structure.residue_of_atom[atom] as usize];
    match (residue.name.trim(), structure.atom_names[atom].trim()) {
        ("ASP", "OD1" | "OD2") | ("GLU", "OE1" | "OE2") => 1,
        ("LYS", "NZ") | ("ARG", "NH1" | "NH2" | "NE") | ("HIS", "ND1" | "NE2") => 2,
        _ => 0,
    }
}

fn is_hydrophobic_carbon(structure: &Structure, atom: usize) -> bool {
    if structure.elements[atom] != 6 {
        return false;
    }
    let residue = &structure.residues[structure.residue_of_atom[atom] as usize];
    let name = structure.atom_names[atom].trim();
    // Side-chain carbons only: every residue has a backbone, and backbones touch.
    residue.kind == ResidueKind::AminoAcid
        && HYDROPHOBIC.contains(&residue.name.trim())
        && !matches!(name, "C" | "CA")
}

fn cite(structure: &Structure, atom: usize, what: Option<&str>) -> String {
    let residue = &structure.residues[structure.residue_of_atom[atom] as usize];
    let chain = &structure.chains[residue.chain_index as usize];
    format!(
        "{} {} {} ({})",
        residue.name.trim(),
        residue.number,
        what.unwrap_or_else(|| structure.atom_names[atom].trim()),
        chain.id
    )
}

/// Pairs of atoms that pass `candidate`, within `cutoff` of each other, in different
/// residues that are not neighbours along a chain.
fn atom_pairs(
    structure: &Structure,
    cutoff: f32,
    waters: bool,
    candidate: impl Fn(usize) -> bool,
) -> Vec<(usize, usize, f32)> {
    let count = structure.atom_count();
    let keep: Vec<bool> = (0..count)
        .map(|atom| {
            let residue = &structure.residues[structure.residue_of_atom[atom] as usize];
            (waters || residue.kind != ResidueKind::Water) && candidate(atom)
        })
        .collect();
    // On the graphics card when there is one: each atom's neighbours are its own question,
    // and the card asks thousands at once.
    let sites: Vec<crate::gpu::Site> = (0..count)
        .filter(|&atom| keep[atom])
        .map(|atom| {
            let residue = structure.residue_of_atom[atom];
            crate::gpu::Site {
                atom: atom as u32,
                position: structure.position(atom),
                residue,
                chain: structure.residues[residue as usize].chain_index,
            }
        })
        .collect();
    if sites.is_empty() {
        return Vec::new();
    }
    if let Some(pairs) = crate::gpu::pairs(&sites, cutoff) {
        return pairs;
    }
    // A grid of cells as wide as the reach: a pair within reach is in the same cell or in
    // neighbouring ones, so each atom is compared with a handful and not with all.
    let key = |position: Vec3| -> (i32, i32, i32) {
        (
            (position[0] / cutoff).floor() as i32,
            (position[1] / cutoff).floor() as i32,
            (position[2] / cutoff).floor() as i32,
        )
    };
    let mut cells: std::collections::HashMap<(i32, i32, i32), Vec<u32>> =
        std::collections::HashMap::new();
    for (atom, kept) in keep.iter().enumerate() {
        if *kept {
            cells
                .entry(key(structure.position(atom)))
                .or_default()
                .push(atom as u32);
        }
    }
    // The kept atoms are shared out between the processor's cores, each finding the pairs
    // its own atoms start: a capsid is three and a half million atoms, and on one core the
    // search was most of the seconds its analysis took.
    let kept: Vec<usize> = (0..count).filter(|&atom| keep[atom]).collect();
    let search = |a: usize, pairs: &mut Vec<(usize, usize, f32)>| {
        let here = structure.position(a);
        let (x, y, z) = key(here);
        let residue_a = structure.residue_of_atom[a];
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(others) =
                        neighbour_cell((x, y, z), (dx, dy, dz)).and_then(|cell| cells.get(&cell))
                    else {
                        continue;
                    };
                    for &other in others {
                        let b = other as usize;
                        let residue_b = structure.residue_of_atom[b];
                        if b <= a || residue_a == residue_b {
                            continue;
                        }
                        // Neighbours along a chain are within reach by construction.
                        let (ra, rb) = (
                            &structure.residues[residue_a as usize],
                            &structure.residues[residue_b as usize],
                        );
                        if ra.chain_index == rb.chain_index && residue_a.abs_diff(residue_b) == 1 {
                            continue;
                        }
                        let distance = length(sub(here, structure.position(b)));
                        if distance <= cutoff && distance >= 2.0 {
                            pairs.push((a, b, distance));
                        }
                    }
                }
            }
        }
    };
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(kept.len().div_ceil(20_000).max(1));
    let share = kept.len().div_ceil(workers).max(1);
    let search = &search;
    let mut pairs = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = kept
            .chunks(share)
            .map(|chunk| {
                scope.spawn(move || {
                    let mut found = Vec::new();
                    for &a in chunk {
                        search(a, &mut found);
                    }
                    found
                })
            })
            .collect();
        for handle in handles {
            pairs.extend(handle.join().unwrap_or_default());
        }
    });
    pairs
}

/// An aromatic ring: where its centre is and which way its plane faces.
struct Ring {
    residue: u32,
    anchor: usize,
    centre: Vec3,
    normal: Vec3,
}

fn rings(structure: &Structure) -> Vec<Ring> {
    const RINGS: &[(&str, &[&str])] = &[
        ("PHE", &["CG", "CD1", "CE1", "CZ", "CE2", "CD2"]),
        ("TYR", &["CG", "CD1", "CE1", "CZ", "CE2", "CD2"]),
        ("HIS", &["CG", "ND1", "CE1", "NE2", "CD2"]),
        ("TRP", &["CD2", "CE2", "CZ2", "CH2", "CZ3", "CE3"]),
        ("TRP", &["CG", "CD1", "NE1", "CE2", "CD2"]),
    ];
    let mut found = Vec::new();
    for (index, residue) in structure.residues.iter().enumerate() {
        let first = residue.first_atom as usize;
        let end = first + residue.atom_count as usize;
        for (name, atoms) in RINGS
            .iter()
            .filter(|(name, _)| *name == residue.name.trim())
        {
            let _ = name;
            let members: Option<Vec<usize>> = atoms
                .iter()
                .map(|wanted| {
                    (first..end).find(|&atom| structure.atom_names[atom].trim() == *wanted)
                })
                .collect();
            let Some(members) = members else { continue };
            let mut centre = [0.0f32; 3];
            for &atom in &members {
                let p = structure.position(atom);
                centre = [centre[0] + p[0], centre[1] + p[1], centre[2] + p[2]];
            }
            let n = members.len() as f32;
            let centre = [centre[0] / n, centre[1] / n, centre[2] / n];
            let normal = cross(
                sub(structure.position(members[0]), centre),
                sub(structure.position(members[2]), centre),
            );
            let size = length(normal);
            if size < 1e-6 {
                continue;
            }
            found.push(Ring {
                residue: index as u32,
                anchor: members[0],
                centre,
                normal: [normal[0] / size, normal[1] / size, normal[2] / size],
            });
        }
    }
    found
}

/// Finds contacts of a kind.
///
/// `within` limits it to contacts with at least one end in those residues - what is
/// selected - and with none it is the whole structure. Waters are left out unless asked
/// for, since every surface atom has one within reach.
pub fn find(
    structure: &Structure,
    kind: ContactKind,
    within: Option<&[u32]>,
    waters: bool,
) -> Vec<Contact> {
    find_named(structure, kind, within, waters, true)
}

/// `find`, with the two ends' names written out only when `named`.
///
/// A name is a formatted string for each end of each contact, and a capsid's hydrogen
/// bonds are half a million: the strings were a good part of the search, and the scene
/// list, which shows only counts and draws only positions, never read them.
pub fn find_named(
    structure: &Structure,
    kind: ContactKind,
    within: Option<&[u32]>,
    waters: bool,
    named: bool,
) -> Vec<Contact> {
    let chosen: Option<std::collections::HashSet<u32>> =
        within.map(|list| list.iter().copied().collect());
    let in_scope = |a: u32, b: u32| {
        chosen
            .as_ref()
            .map_or(true, |set| set.contains(&a) || set.contains(&b))
    };
    let atom_contact = |a: usize, b: usize, distance: f32| Contact {
        residues: [structure.residue_of_atom[a], structure.residue_of_atom[b]],
        names: if named {
            [cite(structure, a, None), cite(structure, b, None)]
        } else {
            [String::new(), String::new()]
        },
        distance,
        from: structure.position(a),
        to: structure.position(b),
    };

    match kind {
        ContactKind::Polar => {
            // Who each atom is bonded to, for the angles at the two ends: as two flat
            // arrays, where to start and the neighbours in a row, since a list per atom is
            // three and a half million allocations for a capsid.
            let atoms = structure.atom_count();
            let mut start = vec![0_u32; atoms + 1];
            let covalent = || structure.bonds.iter().filter(|bond| bond.is_covalent());
            for bond in covalent() {
                start[bond.atoms[0] as usize + 1] += 1;
                start[bond.atoms[1] as usize + 1] += 1;
            }
            for at in 0..atoms {
                start[at + 1] += start[at];
            }
            let mut filled = start.clone();
            let mut neighbours = vec![0_u32; start[atoms] as usize];
            for bond in covalent() {
                for (from, to) in [
                    (bond.atoms[0], bond.atoms[1]),
                    (bond.atoms[1], bond.atoms[0]),
                ] {
                    neighbours[filled[from as usize] as usize] = to;
                    filled[from as usize] += 1;
                }
            }
            let bonded = |at: usize| &neighbours[start[at] as usize..start[at + 1] as usize];
            let open = |at: usize, other: usize| {
                bonded(at).iter().all(|&antecedent| {
                    angle(
                        structure.position(antecedent as usize),
                        structure.position(at),
                        structure.position(other),
                    ) >= 90.0
                })
            };
            atom_pairs(structure, 3.5, waters, |atom| {
                let (donor, acceptor) = polar_role(structure, atom);
                donor || acceptor
            })
            .into_iter()
            .filter(|&(a, b, _)| {
                let (donor_a, acceptor_a) = polar_role(structure, a);
                let (donor_b, acceptor_b) = polar_role(structure, b);
                ((donor_a && acceptor_b) || (donor_b && acceptor_a)) && open(a, b) && open(b, a)
            })
            .filter(|&(a, b, _)| {
                in_scope(structure.residue_of_atom[a], structure.residue_of_atom[b])
            })
            .map(|(a, b, distance)| atom_contact(a, b, distance))
            .collect()
        }
        ContactKind::SaltBridge => atom_pairs(structure, 4.0, waters, |atom| {
            salt_role(structure, atom) != 0
        })
        .into_iter()
        .filter(|&(a, b, _)| salt_role(structure, a) != salt_role(structure, b))
        .filter(|&(a, b, _)| in_scope(structure.residue_of_atom[a], structure.residue_of_atom[b]))
        .map(|(a, b, distance)| atom_contact(a, b, distance))
        .collect(),
        ContactKind::Hydrophobic => atom_pairs(structure, 4.0, waters, |atom| {
            is_hydrophobic_carbon(structure, atom)
        })
        .into_iter()
        .filter(|&(a, b, _)| in_scope(structure.residue_of_atom[a], structure.residue_of_atom[b]))
        .map(|(a, b, distance)| atom_contact(a, b, distance))
        .collect(),
        ContactKind::PiStacking => {
            let rings = rings(structure);
            // Each ring is compared with the rings in its own and the neighbouring cells of
            // a grid as wide as the reach, not with every other ring: a capsid has sixty
            // thousand of them, and all pairs is two billion.
            const REACH: f32 = 5.5;
            let cell = |point: Vec3| {
                (
                    (point[0] / REACH).floor() as i32,
                    (point[1] / REACH).floor() as i32,
                    (point[2] / REACH).floor() as i32,
                )
            };
            let mut grid: std::collections::HashMap<(i32, i32, i32), Vec<usize>> =
                std::collections::HashMap::new();
            for (index, ring) in rings.iter().enumerate() {
                grid.entry(cell(ring.centre)).or_default().push(index);
            }
            let mut found = Vec::new();
            for (at, first) in rings.iter().enumerate() {
                let home = cell(first.centre);
                let mut near: Vec<usize> = Vec::new();
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            if let Some(members) = super::bonds::neighbour_cell(home, (dx, dy, dz))
                                .and_then(|key| grid.get(&key))
                            {
                                near.extend(members.iter().copied().filter(|&other| other > at));
                            }
                        }
                    }
                }
                // In ring order, as the search over all pairs found them.
                near.sort_unstable();
                for second in near.into_iter().map(|index| &rings[index]) {
                    if first.residue == second.residue || !in_scope(first.residue, second.residue) {
                        continue;
                    }
                    let between = sub(second.centre, first.centre);
                    let distance = length(between);
                    if distance > 5.5 {
                        continue;
                    }
                    // The angle between the planes, folded into 0..90.
                    let tilt = dot(first.normal, second.normal)
                        .abs()
                        .clamp(0.0, 1.0)
                        .acos()
                        .to_degrees();
                    // How far the second centre sits off the first ring's axis.
                    let along = dot(between, first.normal);
                    let offset = (distance * distance - along * along).max(0.0).sqrt();
                    let parallel = tilt <= 30.0 && offset <= 2.0;
                    let t_shaped = tilt >= 60.0;
                    if !parallel && !t_shaped {
                        continue;
                    }
                    let shape = if parallel {
                        "ring, parallel"
                    } else {
                        "ring, T-shaped"
                    };
                    found.push(Contact {
                        residues: [first.residue, second.residue],
                        names: if named {
                            [
                                cite(structure, first.anchor, Some(shape)),
                                cite(structure, second.anchor, Some(shape)),
                            ]
                        } else {
                            [String::new(), String::new()]
                        },
                        distance,
                        from: first.centre,
                        to: second.centre,
                    });
                }
            }
            found
        }
    }
}

/// The most of itself a residue can show to solvent, in square angstroms: its surface in
/// an extended Gly-X-Gly tripeptide (Tien et al. 2013, theoretical values).
fn max_accessible(name: &str) -> Option<f32> {
    Some(match name {
        "ALA" => 129.0,
        "ARG" => 274.0,
        "ASN" => 195.0,
        "ASP" => 193.0,
        "CYS" => 167.0,
        "GLN" => 225.0,
        "GLU" => 223.0,
        "GLY" => 104.0,
        "HIS" => 224.0,
        "ILE" => 197.0,
        "LEU" => 201.0,
        "LYS" => 236.0,
        "MET" => 224.0,
        "PHE" => 240.0,
        "PRO" => 159.0,
        "SER" => 155.0,
        "THR" => 172.0,
        "TRP" => 285.0,
        "TYR" => 263.0,
        "VAL" => 174.0,
        _ => return None,
    })
}

/// Solvent-accessible surface per residue, in square angstroms, by Shrake and Rupley's
/// method (1973): each atom is a sphere of its van der Waals radius plus a 1.4 A probe,
/// dotted with points, and its accessible area is the share of those points that lie
/// inside no other atom's sphere. Waters and hydrogens are not part of the molecule being
/// measured; everything else is, so a ligand or another chain buries what it covers.
pub fn accessible_surface(structure: &Structure) -> Vec<f32> {
    accessible_surface_of(structure, |_| true)
}

/// `accessible_surface`, measured only for the residues `wanted` picks; the rest read 0.
///
/// Every atom still occludes - a buried leucine is buried by whatever is around it - but
/// only the atoms of the residues asked about are measured, which for the core is the
/// hydrophobic residues: about two fifths of a protein's atoms.
pub fn accessible_surface_of(
    structure: &Structure,
    wanted: impl Fn(&crate::structure::model::Residue) -> bool + Sync,
) -> Vec<f32> {
    const PROBE: f32 = 1.4;
    const POINTS: usize = 96;
    // Points spread evenly over a unit sphere, by the golden spiral.
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    let sphere: Vec<Vec3> = (0..POINTS)
        .map(|index| {
            let y = 1.0 - 2.0 * (index as f32 + 0.5) / POINTS as f32;
            let ring = (1.0 - y * y).max(0.0).sqrt();
            let turn = golden * index as f32;
            [ring * turn.cos(), y, ring * turn.sin()]
        })
        .collect();

    let atoms: Vec<usize> = (0..structure.atom_count())
        .filter(|&atom| {
            let residue = &structure.residues[structure.residue_of_atom[atom] as usize];
            residue.kind != ResidueKind::Water && structure.elements[atom] != 1
        })
        .collect();
    let reach: Vec<f32> = atoms
        .iter()
        .map(|&atom| super::chem::vdw_radius(structure.elements[atom]) + PROBE)
        .collect();
    let widest = reach.iter().copied().fold(0.0_f32, f32::max).max(1.0);
    let cell = widest * 2.0;
    let key = |position: Vec3| -> (i32, i32, i32) {
        (
            (position[0] / cell).floor() as i32,
            (position[1] / cell).floor() as i32,
            (position[2] / cell).floor() as i32,
        )
    };
    let mut cells: std::collections::HashMap<(i32, i32, i32), Vec<u32>> =
        std::collections::HashMap::new();
    for (slot, &atom) in atoms.iter().enumerate() {
        cells
            .entry(key(structure.position(atom)))
            .or_default()
            .push(slot as u32);
    }

    let measured: Vec<bool> = atoms
        .iter()
        .map(|&atom| wanted(&structure.residues[structure.residue_of_atom[atom] as usize]))
        .collect();
    let sphere_points: Vec<[f32; 3]> = sphere.clone();
    let spheres: Vec<([f32; 3], f32)> = atoms
        .iter()
        .zip(&reach)
        .map(|(&atom, &radius)| (structure.position(atom), radius))
        .collect();
    if let Some(open) = crate::gpu::open_points(&spheres, &measured, &sphere_points) {
        let mut surface = vec![0.0_f32; structure.residues.len()];
        for (slot, &atom) in atoms.iter().enumerate() {
            if measured[slot] {
                let radius = reach[slot];
                surface[structure.residue_of_atom[atom] as usize] +=
                    4.0 * std::f32::consts::PI * radius * radius * open[slot] as f32
                        / POINTS as f32;
            }
        }
        return surface;
    }

    // The area open on one atom. Each atom's is its own sum over its own neighbours, so
    // the atoms are shared out between the processor's cores: this is nine tenths of what
    // a structure's preset analysis costs, and on a capsid it is seconds on one core.
    let area_of = |slot: usize| -> f32 {
        let atom = atoms[slot];
        let here = structure.position(atom);
        let radius = reach[slot];
        let (x, y, z) = key(here);
        // The atoms whose spheres can reach into this one.
        let mut near: Vec<(Vec3, f32)> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(others) =
                        neighbour_cell((x, y, z), (dx, dy, dz)).and_then(|cell| cells.get(&cell))
                    else {
                        continue;
                    };
                    for &other in others {
                        let other = other as usize;
                        if other == slot {
                            continue;
                        }
                        let there = structure.position(atoms[other]);
                        if length(sub(here, there)) < radius + reach[other] {
                            near.push((there, reach[other] * reach[other]));
                        }
                    }
                }
            }
        }
        let open = sphere
            .iter()
            .filter(|point| {
                let at = [
                    here[0] + point[0] * radius,
                    here[1] + point[1] * radius,
                    here[2] + point[2] * radius,
                ];
                near.iter().all(|(there, squared)| {
                    let d = sub(at, *there);
                    dot(d, d) >= *squared
                })
            })
            .count();
        4.0 * std::f32::consts::PI * radius * radius * open as f32 / POINTS as f32
    };
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(atoms.len().max(1));
    let share = atoms.len().div_ceil(workers).max(1);
    let slots: Vec<usize> = (0..atoms.len()).filter(|&slot| measured[slot]).collect();
    let areas: Vec<f32> = std::thread::scope(|scope| {
        let handles: Vec<_> = slots
            .chunks(share)
            .map(|chunk| {
                scope.spawn(|| {
                    chunk
                        .iter()
                        .map(|&slot| area_of(slot))
                        .collect::<Vec<f32>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    let mut surface = vec![0.0_f32; structure.residues.len()];
    for (area, &slot) in areas.into_iter().zip(&slots) {
        surface[structure.residue_of_atom[atoms[slot]] as usize] += area;
    }
    surface
}

/// How buried a residue has to be to count as core: a fifth of what it could show.
const CORE_EXPOSURE: f32 = 0.20;

/// The hydrophobic core: hydrophobic residues that are buried.
///
/// Buried the way the field measures it - relative solvent accessibility: a residue's
/// accessible surface in this structure over the most it could have (Tien et al. 2013),
/// with at most a fifth showing. Not by counting neighbours within a distance, which was
/// the first version here: a count that fits a large protein is never reached in a small
/// one, and ubiquitin's Leu43 - buried, by any reading of the structure - missed a
/// threshold of twenty by one.
pub fn hydrophobic_core(structure: &Structure) -> Vec<u32> {
    let candidate = |residue: &crate::structure::model::Residue| {
        residue.kind == ResidueKind::AminoAcid && HYDROPHOBIC.contains(&residue.name.trim())
    };
    let surface = accessible_surface_of(structure, candidate);
    structure
        .residues
        .iter()
        .enumerate()
        .filter(|(_, residue)| {
            residue.kind == ResidueKind::AminoAcid && HYDROPHOBIC.contains(&residue.name.trim())
        })
        .filter(|(index, residue)| {
            max_accessible(residue.name.trim())
                .is_some_and(|most| surface[*index] / most <= CORE_EXPOSURE)
        })
        .map(|(index, _)| index as u32)
        .collect()
}

/// The residues around the ligands: every polymer residue with an atom within 4 A of an
/// atom of something that is neither polymer nor water - a cofactor, a drug, an ion.
/// The cutoff is the one contact tables conventionally use for "in contact".
pub fn binding_site(structure: &Structure) -> Vec<u32> {
    near_other(structure, |residue| match residue.kind {
        ResidueKind::Ligand => Side::Ligand,
        ResidueKind::Water => Side::Neither,
        _ => Side::Polymer(u32::MAX),
    })
}

/// The residues where chains meet: every polymer residue with an atom within 4 A of an
/// atom of a different polymer chain.
pub fn interface(structure: &Structure) -> Vec<u32> {
    near_other(structure, |residue| match residue.kind {
        ResidueKind::Ligand | ResidueKind::Water => Side::Neither,
        _ => Side::Polymer(residue.chain_index),
    })
}

/// Which side of a meeting a residue is on, for `near_other`.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Neither,
    Ligand,
    /// A polymer residue, and the chain it is in where that matters.
    Polymer(u32),
}

/// The polymer residues with a heavy atom within 4 A of the other side: a ligand, or a
/// polymer residue of another chain.
fn near_other(
    structure: &Structure,
    side: impl Fn(&crate::structure::model::Residue) -> Side,
) -> Vec<u32> {
    const REACH: f32 = 4.0;
    let sides: Vec<Side> = structure.residues.iter().map(&side).collect();
    let of = |atom: usize| sides[structure.residue_of_atom[atom] as usize];
    let mut found = vec![false; structure.residues.len()];
    let pairs = atom_pairs(structure, REACH, false, |atom| {
        structure.elements[atom] != 1 && of(atom) != Side::Neither
    });
    for (a, b, _) in pairs {
        let (side_a, side_b) = (of(a), of(b));
        let meets = match (side_a, side_b) {
            (Side::Ligand, Side::Polymer(_)) | (Side::Polymer(_), Side::Ligand) => true,
            (Side::Polymer(one), Side::Polymer(other)) => one != other,
            _ => false,
        };
        if !meets {
            continue;
        }
        for (atom, here) in [(a, side_a), (b, side_b)] {
            if matches!(here, Side::Polymer(_)) {
                found[structure.residue_of_atom[atom] as usize] = true;
            }
        }
    }
    found
        .iter()
        .enumerate()
        .filter(|(_, hit)| **hit)
        .map(|(index, _)| index as u32)
        .collect()
}

/// Each residue's relative solvent accessibility, 0 buried to about 1 fully exposed;
/// `None` for anything that is not a standard amino acid.
pub fn relative_accessibility(structure: &Structure) -> Vec<Option<f32>> {
    let surface = accessible_surface(structure);
    structure
        .residues
        .iter()
        .enumerate()
        .map(|(index, residue)| {
            max_accessible(residue.name.trim()).map(|most| surface[index] / most)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::model::RawAtom;

    fn atom(
        name: &str,
        element: u8,
        residue: &str,
        number: i32,
        chain: &str,
        position: [f32; 3],
    ) -> RawAtom {
        RawAtom {
            name: name.into(),
            element,
            position,
            b_factor: 20.0,
            occupancy: 1.0,
            residue_name: residue.into(),
            residue_number: number,
            insertion_code: None,
            chain_id: chain.into(),
            is_hetatm: false,
            label_asym_id: String::new(),
            entity_id: String::new(),
        }
    }

    #[test]
    fn contacts_are_found_between_residues_and_not_within_or_beside_them() {
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![
                // An Asp and a Lys three residues apart, their charged ends 3 A apart.
                atom("OD1", 8, "ASP", 10, "A", [0.0, 0.0, 0.0]),
                atom("OD2", 8, "ASP", 10, "A", [1.2, 0.0, 0.0]),
                // The next residue's amide nitrogen, as close: two bonds away, not a contact.
                atom("N", 7, "GLY", 11, "A", [0.0, 0.0, 3.0]),
                atom("NZ", 7, "LYS", 13, "A", [0.0, 3.0, 0.0]),
                // And an oxygen out of reach of everything.
                atom("O", 8, "SER", 40, "A", [30.0, 0.0, 0.0]),
            ],
        );
        let polar = find(&structure, ContactKind::Polar, None, false);
        let pairs: Vec<[String; 2]> = polar.iter().map(|contact| contact.names.clone()).collect();
        assert!(pairs
            .iter()
            .any(|pair| pair[0].starts_with("ASP 10 OD1") && pair[1].starts_with("LYS 13 NZ")));
        assert!(
            pairs.iter().all(|pair| !pair[1].starts_with("GLY 11")),
            "{pairs:?}"
        );
        assert!(polar
            .iter()
            .all(|contact| contact.residues[0] != contact.residues[1]));
        // Carboxylate to carboxylate is two acceptors and no donor: not a hydrogen bond.
        assert!(pairs
            .iter()
            .all(|pair| !(pair[0].contains("ASP") && pair[1].contains("ASP"))));

        let bridges = find(&structure, ContactKind::SaltBridge, None, false);
        assert_eq!(
            bridges.len(),
            2,
            "both carboxylate oxygens reach the lysine: {bridges:?}"
        );
        assert!(bridges.iter().all(|contact| contact.distance <= 4.0));

        // Limited to a residue that takes part in none of it.
        let serine = structure
            .residues
            .iter()
            .position(|residue| residue.name.trim() == "SER")
            .unwrap() as u32;
        assert!(find(&structure, ContactKind::Polar, Some(&[serine]), false).is_empty());
    }

    #[test]
    fn huge_coordinates_do_not_overflow_the_grid() {
        // Cell indices saturate at i32::MAX; the neighbour step past it must not overflow.
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![
                atom("OD1", 8, "ASP", 1, "A", [3e38, 3e38, -3e38]),
                atom("NZ", 7, "LYS", 5, "A", [3e38, 3e38, -3e38]),
                atom("CB", 6, "LEU", 9, "A", [1e12, 1e12, 1e12]),
                atom("CB", 6, "VAL", 20, "A", [1e12, 1e12, 1e12]),
            ],
        );
        for kind in [
            ContactKind::Polar,
            ContactKind::SaltBridge,
            ContactKind::Hydrophobic,
            ContactKind::PiStacking,
        ] {
            let _ = find(&structure, kind, None, true);
        }
        assert_eq!(
            accessible_surface(&structure).len(),
            structure.residues.len()
        );
    }

    #[test]
    fn an_empty_structure_has_no_contacts_and_no_surface() {
        let structure = Structure::build("t".into(), "test".into(), Vec::new());
        assert!(find(&structure, ContactKind::Polar, None, false).is_empty());
        assert!(find(&structure, ContactKind::PiStacking, Some(&[0, 99]), false).is_empty());
        assert!(accessible_surface(&structure).is_empty());
        assert!(hydrophobic_core(&structure).is_empty());
    }

    #[test]
    fn a_lone_residue_is_fully_exposed_and_a_ligand_has_no_relative_value() {
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![
                atom("CA", 6, "LEU", 1, "A", [0.0, 0.0, 0.0]),
                atom("C1", 6, "LIG", 2, "B", [40.0, 0.0, 0.0]),
            ],
        );
        let relative = relative_accessibility(&structure);
        assert!(relative[0].is_some_and(|value| value > 0.2), "{relative:?}");
        assert_eq!(relative[1], None);
        assert!(hydrophobic_core(&structure).is_empty(), "nothing buries it");
    }

    #[test]
    fn waters_are_left_out_unless_asked_for() {
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![
                atom("OG", 8, "SER", 1, "A", [0.0, 0.0, 0.0]),
                atom("O", 8, "HOH", 101, "W", [2.8, 0.0, 0.0]),
            ],
        );
        assert!(find(&structure, ContactKind::Polar, None, false).is_empty());
        assert_eq!(find(&structure, ContactKind::Polar, None, true).len(), 1);
    }

    #[test]
    fn stacked_rings_are_found_and_skewed_ones_are_not() {
        // A benzene-like ring in the xy plane, as a phenylalanine's.
        let ring = |number: i32, lift: f32, tilt: bool| -> Vec<RawAtom> {
            ["CG", "CD1", "CE1", "CZ", "CE2", "CD2"]
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    let turn = index as f32 / 6.0 * std::f32::consts::TAU;
                    let (x, y) = (1.4 * turn.cos(), 1.4 * turn.sin());
                    // Tilted 45 degrees about x: neither stacked nor T-shaped.
                    let half = std::f32::consts::FRAC_1_SQRT_2;
                    let position = if tilt {
                        [x, y * half, lift + y * half]
                    } else {
                        [x, y, lift]
                    };
                    atom(name, 6, "PHE", number, "A", position)
                })
                .collect()
        };
        let stacked = Structure::build(
            "t".into(),
            "test".into(),
            [ring(10, 0.0, false), ring(20, 3.6, false)].concat(),
        );
        let found = find(&stacked, ContactKind::PiStacking, None, false);
        assert_eq!(found.len(), 1);
        assert!((found[0].distance - 3.6).abs() < 0.01);
        assert!(found[0].names[0].contains("parallel"));

        let skewed = Structure::build(
            "t".into(),
            "test".into(),
            [ring(10, 0.0, false), ring(20, 4.5, true)].concat(),
        );
        assert!(find(&skewed, ContactKind::PiStacking, None, false).is_empty());
    }

    #[test]
    fn the_surface_is_the_same_however_many_cores_share_it() {
        // Two atoms far apart are each fully open: 4 pi r^2 with r the atom's radius plus
        // the probe's. Summed per residue, in residue order, whatever thread did which.
        let text = "\
ATOM      1  CA  ALA A   1       0.000   0.000   0.000  1.00 20.00           C
ATOM      2  CA  GLY A   2      50.000   0.000   0.000  1.00 20.00           C
";
        let structure = crate::structure::parse_text(text, "two.pdb").unwrap();
        let surface = accessible_surface(&structure);
        let expected = 4.0 * std::f32::consts::PI * (1.70_f32 + 1.4).powi(2);
        assert_eq!(surface.len(), 2);
        for area in surface {
            assert!((area - expected).abs() < 0.5, "{area} vs {expected}");
        }
    }

    #[test]
    fn a_binding_site_is_the_polymer_around_a_ligand_and_an_interface_is_between_chains() {
        let text = "\
ATOM      1  CA  ALA A   1       0.000   0.000   0.000  1.00 20.00           C
ATOM      2  CA  GLY A   2       3.800   0.000   0.000  1.00 20.00           C
ATOM      3  CA  SER A   3      30.000   0.000   0.000  1.00 20.00           C
ATOM      4  CA  LEU B   1       3.800   3.500   0.000  1.00 20.00           C
ATOM      5  CA  VAL B   2      60.000   0.000   0.000  1.00 20.00           C
HETATM    6 ZN    ZN A 101       0.000   2.500   0.000  1.00 20.00          ZN
HETATM    7  O   HOH A 201      30.000   2.500   0.000  1.00 20.00           O
";
        let structure = crate::structure::parse_text(text, "site.pdb").unwrap();
        let named = |list: Vec<u32>| -> Vec<String> {
            list.iter()
                .map(|&index| structure.residues[index as usize].name.trim().to_string())
                .collect()
        };
        // The zinc touches ALA 1 and, across the chains, LEU 1; the water beside SER 3 is
        // not a ligand.
        assert_eq!(named(binding_site(&structure)), ["ALA", "LEU"]);
        // GLY 2 and LEU 1 are 3.5 A apart in different chains; nothing else meets.
        assert_eq!(named(interface(&structure)), ["GLY", "LEU"]);
    }
}
