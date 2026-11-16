//! Bonding.
//!
//! Bonds are read, not guessed. A coordinate file names its chemical components, and the
//! Chemical Component Dictionary says exactly which atoms of each are bonded and with what
//! order - so the bonds inside a residue are a lookup. What a dictionary cannot say is how
//! two components are joined, and the file answers that too: `_entity_poly_seq` gives the
//! polymer, so consecutive residues link C to N or O3' to P, and `_struct_conn` gives the
//! disulfides, the covalent modifications and the metal coordination.
//!
//! In order:
//!
//! 1. the file's own `_chem_comp_bond`, which is the dictionary for exactly the components
//!    in this entry, including its ligands;
//! 2. the built-in table (`components.rs`) for the standard residues, which is what a PDB
//!    file - carrying no dictionary at all - gets;
//! 3. polymer links between consecutive residues of a chain;
//! 4. `_struct_conn`;
//! 5. distance, and only for a component nothing above knew: a novel ligand in a PDB file.
//!    Marked `BondKind::Inferred`, because a guess should be able to say that it is one.
//!
//! Distance appears in one other place - as a sanity check on a polymer link, since a file
//! with a missing loop numbers its residues consecutively across the gap and bonding across
//! it would draw a stick through empty space.

use super::chem::{self, Element};
use super::components::{self, Template};
use super::model::{Bond, BondKind, BondOrder, Linking, ResidueLink, Structure};
use std::collections::{HashMap, HashSet};

/// Slack added to the sum of covalent radii for the distance fallback. 0.45 A is the
/// tolerance viewers have settled on: wide enough for strained geometry and modest
/// coordinate error, tight enough that non-bonded contacts (which start around 3 A) are
/// not wired together.
const TOLERANCE: f32 = 0.45;

/// Nothing bonds beyond this, so it is the grid cell size for the fallback.
const MAX_BOND_LENGTH: f32 = 2.6;

/// Below this, two atoms are the same atom in two files, or a broken model.
const MIN_BOND_LENGTH_SQUARED: f32 = 0.16; // 0.4 A

/// One end of a `_struct_conn` record, as the file names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomRef {
    /// Author chain id.
    pub chain: String,
    /// The label asym id when the record has one - the precise key, since author chain A
    /// of 4HHB is three different asyms.
    pub label_chain: Option<String>,
    /// Author residue number. `None` for a component the record identifies only by name.
    pub number: Option<i32>,
    pub comp_id: String,
    pub atom_name: String,
    pub insertion_code: Option<char>,
}

/// A `_struct_conn` record: two named atoms and what joins them.
#[derive(Debug, Clone)]
pub struct Connection {
    pub kind: BondKind,
    pub a: AtomRef,
    pub b: AtomRef,
}

/// What a file said about bonding. Empty for a format that says nothing.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    /// Component templates from the file's own `_chem_comp_bond` / `_chem_comp.type`.
    pub templates: HashMap<String, Template>,
    pub connections: Vec<Connection>,
}

/// Builds the bond graph and the residue-level links from it.
pub fn build(structure: &mut Structure, evidence: &Evidence) {
    if structure.atom_count() < 2 {
        structure.bonds.clear();
        structure.residue_links.clear();
        return;
    }

    let mut bonds: Vec<Bond> = Vec::with_capacity(structure.atom_count() + 8);
    // Repeats are taken out at the end, the first stated kept - a template's word before a
    // link's, a link's before a distance's. A set asked about every bond as it was made was
    // a good part of a capsid's reading.
    let push = |bonds: &mut Vec<Bond>, a: u32, b: u32, order: BondOrder, kind: BondKind| {
        if a == b {
            return;
        }
        bonds.push(Bond::new(a, b, order, kind));
    };

    // Atom names within a residue, for template lookup. Names repeat across residues, so
    // the map is per residue rather than global.
    let atom_in = |residue: usize, name: &str| -> Option<u32> {
        let residue = structure.residues.get(residue)?;
        (residue.first_atom..residue.first_atom + residue.atom_count).find(|atom| {
            structure
                .atom_names
                .get(*atom as usize)
                .map(|candidate| candidate.trim() == name.trim())
                .unwrap_or(false)
        })
    };

    // --- 1 and 2: templates ------------------------------------------------
    // Each residue's own bonds are its own business, so the residues are shared out
    // between the processor's cores and the pieces joined in order.
    let template_part = |range: std::ops::Range<usize>| {
        let mut bonds: Vec<Bond> = Vec::new();
        let mut untemplated: Vec<usize> = Vec::new();
        let mut linking: Vec<Linking> = Vec::with_capacity(range.len());
        for index in range {
            let residue = &structure.residues[index];
            let from_file = evidence.templates.get(residue.name.trim());
            let builtin = components::builtin(&residue.name);
            // The file's word on how a component links is taken whenever it gives one. Its
            // word on the bonds is taken only if it has any: a file with a `_chem_comp`
            // table and no `_chem_comp_bond` - which is what prediction tools and several
            // archives write - names every component and bonds none of them, and letting
            // that empty entry stand in front of the built-in table leaves every residue a
            // cloud of unconnected atoms.
            let bonded = from_file
                .filter(|template| !template.bonds.is_empty())
                .or(builtin);
            let stated = from_file
                .map(|template| template.linking)
                .or(builtin.map(|t| t.linking));
            // A component nobody has a template for may still sit in a polymer - a
            // phosphoserine in a PDB file, which carries no dictionary. Its backbone atoms
            // say so, and without this it is cut out of the chain on both sides.
            let has = |name: &str| atom_in(index, name).is_some();
            let by_backbone = || {
                if has("N") && has("CA") && has("C") {
                    Linking::Peptide
                } else if has("P") && has("O3'") {
                    Linking::Rna
                } else {
                    Linking::NonPolymer
                }
            };
            linking.push(match stated {
                Some(Linking::NonPolymer) | None if bonded.is_none() => by_backbone(),
                Some(stated) => stated,
                None => by_backbone(),
            });
            let Some(template) = bonded else {
                // One atom has nothing to be bonded to within itself.
                if residue.atom_count > 1 {
                    untemplated.push(index);
                }
                continue;
            };
            for (a, b, order) in &template.bonds {
                if let (Some(first), Some(second)) = (atom_in(index, a), atom_in(index, b)) {
                    push(&mut bonds, first, second, *order, BondKind::Template);
                }
            }
        }
        (bonds, untemplated, linking)
    };
    let residue_count = structure.residues.len();
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(residue_count.div_ceil(5_000).max(1));
    let share = residue_count.div_ceil(workers).max(1);
    let template_part = &template_part;
    let parts: Vec<(Vec<Bond>, Vec<usize>, Vec<Linking>)> = if workers <= 1 {
        vec![template_part(0..residue_count)]
    } else {
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..residue_count)
                .step_by(share)
                .map(|start| {
                    scope.spawn(move || template_part(start..(start + share).min(residue_count)))
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("a template worker panicked"))
                .collect()
        })
    };
    let mut untemplated: Vec<usize> = Vec::new();
    let mut linking: Vec<Linking> = Vec::with_capacity(residue_count);
    for (part_bonds, part_untemplated, part_linking) in parts {
        bonds.extend(part_bonds);
        untemplated.extend(part_untemplated);
        linking.extend(part_linking);
    }

    // --- 3: the polymer ----------------------------------------------------
    // Consecutive residues of one chain, in file order. The link is stated by the polymer,
    // not found by searching: only the distance check is a search, and it is looking for a
    // break rather than for a bond.
    for window in 0..structure.residues.len().saturating_sub(1) {
        let (here, next) = (window, window + 1);
        if structure.residues[here].chain_index != structure.residues[next].chain_index {
            continue;
        }
        let kind = linking[here];
        // Compared by the link they make, not by name: RNA and DNA join the same way, and
        // a hybrid strand is one strand.
        if kind.link_atoms() != linking[next].link_atoms() {
            continue;
        }
        let Some((from, to, limit)) = kind.link_atoms() else {
            continue;
        };
        let (Some(a), Some(b)) = (atom_in(here, from), atom_in(next, to)) else {
            continue;
        };
        let distance = squared_distance(
            structure.position(a as usize),
            structure.position(b as usize),
        );
        if distance <= limit * limit {
            push(&mut bonds, a, b, BondOrder::Single, BondKind::Link);
        }
    }

    // --- 4: struct_conn ----------------------------------------------------
    // Once for each copy an assembly holds: a disulfide stated between chains A and B is
    // in every copy of the pair, and joins the two ends made by the same operator - which
    // is how StructCooker carries `_struct_conn` into an assembly.
    // Each chain's residues, found once: a chain's residues are not always together in
    // the file, and a capsid has too many of both to search every time.
    let mut residues_of: Vec<Vec<u32>> = vec![Vec::new(); structure.chains.len()];
    if !evidence.connections.is_empty() {
        for (index, residue) in structure.residues.iter().enumerate() {
            residues_of[residue.chain_index as usize].push(index as u32);
        }
    }
    for connection in &evidence.connections {
        let ends_b = resolve(structure, &residues_of, &connection.b, &atom_in);
        for (operator, a) in resolve(structure, &residues_of, &connection.a, &atom_in) {
            let Some((_, b)) = ends_b.iter().find(|(other, _)| *other == operator) else {
                continue;
            };
            push(&mut bonds, a, *b, BondOrder::Single, connection.kind);
        }
    }

    // --- 5: distance, for components nothing knew --------------------------
    if !untemplated.is_empty() {
        let mut candidates: Vec<u32> = Vec::new();
        for residue in &untemplated {
            let residue = &structure.residues[*residue];
            candidates.extend(residue.first_atom..residue.first_atom + residue.atom_count);
        }
        for [a, b] in infer_between(structure, &candidates) {
            push(&mut bonds, a, b, BondOrder::Single, BondKind::Inferred);
        }
    }

    // Stable, so that of a bond stated twice the first statement is the one kept.
    bonds.sort_by_key(|bond| bond.atoms);
    bonds.dedup_by_key(|bond| bond.atoms);
    structure.bonds = bonds;
    structure.residue_links = residue_links(structure);
}

/// Which residues are bonded to which, from the atom-level graph.
fn residue_links(structure: &Structure) -> Vec<ResidueLink> {
    let mut links: Vec<ResidueLink> = Vec::new();
    let mut seen: HashSet<[u32; 2]> = HashSet::new();
    for bond in &structure.bonds {
        let (Some(first), Some(second)) = (
            structure
                .residue_of_atom
                .get(bond.atoms[0] as usize)
                .copied(),
            structure
                .residue_of_atom
                .get(bond.atoms[1] as usize)
                .copied(),
        ) else {
            continue;
        };
        if first == second {
            continue;
        }
        let pair = [first.min(second), first.max(second)];
        if seen.insert(pair) {
            links.push(ResidueLink {
                residues: pair,
                kind: bond.kind,
            });
        }
    }
    links
}

/// Finds the atoms a `_struct_conn` end names: one in each copy of the chain it is in,
/// with the operator that made the copy.
fn resolve<'a>(
    structure: &'a Structure,
    residues_of: &[Vec<u32>],
    reference: &AtomRef,
    atom_in: &impl Fn(usize, &str) -> Option<u32>,
) -> Vec<(&'a str, u32)> {
    let mut found = Vec::new();
    for (chain_index, chain) in structure.chains.iter().enumerate() {
        let named = match &reference.label_chain {
            Some(label) => chain.label_asym_id == *label,
            None => chain.auth_id == reference.chain,
        };
        if !named {
            continue;
        }
        let residue = residues_of[chain_index]
            .iter()
            .map(|&index| index as usize)
            .find(|&index| {
                let residue = &structure.residues[index];
                residue
                    .name
                    .trim()
                    .eq_ignore_ascii_case(reference.comp_id.trim())
                    && reference
                        .number
                        .map(|number| residue.number == number)
                        .unwrap_or(true)
                    && (reference.insertion_code.is_none()
                        || residue.insertion_code == reference.insertion_code)
            });
        if let Some(atom) = residue.and_then(|residue| atom_in(residue, &reference.atom_name)) {
            let operator = chain
                .id
                .rsplit_once('_')
                .map_or("1", |(_, operator)| operator);
            found.push((operator, atom));
        }
    }
    found
}

/// The distance pass, over a subset of atoms.
///
/// A uniform grid whose cell is the longest bond, so each atom tests the 27 cells around
/// it: the naive version is 40 billion comparisons on a 200k-atom structure.
fn infer_between(structure: &Structure, atoms: &[u32]) -> Vec<[u32; 2]> {
    let cell_of = |position: [f32; 3]| -> (i32, i32, i32) {
        (
            (position[0] / MAX_BOND_LENGTH).floor() as i32,
            (position[1] / MAX_BOND_LENGTH).floor() as i32,
            (position[2] / MAX_BOND_LENGTH).floor() as i32,
        )
    };
    let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
    for atom in atoms {
        grid.entry(cell_of(structure.position(*atom as usize)))
            .or_default()
            .push(*atom);
    }

    let mut found = Vec::new();
    for &atom in atoms {
        let index = atom as usize;
        let element_a = structure.elements[index];
        let position_a = structure.position(index);
        let radius_a = chem::covalent_radius(element_a);
        let (cx, cy, cz) = cell_of(position_a);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(neighbors) =
                        neighbour_cell((cx, cy, cz), (dx, dy, dz)).and_then(|cell| grid.get(&cell))
                    else {
                        continue;
                    };
                    for &other in neighbors {
                        if other <= atom {
                            continue;
                        }
                        let element_b = structure.elements[other as usize];
                        if !may_bond(element_a, element_b) {
                            continue;
                        }
                        let position_b = structure.position(other as usize);
                        let distance = squared_distance(position_a, position_b);
                        if distance < MIN_BOND_LENGTH_SQUARED {
                            continue;
                        }
                        // Never past the cell size: a pair further apart than a cell is
                        // found or missed depending on where the cell boundary falls.
                        let limit = (radius_a + chem::covalent_radius(element_b) + TOLERANCE)
                            .min(MAX_BOND_LENGTH);
                        if distance <= limit * limit {
                            found.push([atom, other]);
                        }
                    }
                }
            }
        }
    }
    found
}

/// The grid cell `step` away from `cell`, if there is one.
///
/// Cell indices come from coordinates by a saturating cast, so an absurd but finite
/// coordinate sits at `i32::MAX` and the cell past it does not exist: plain addition
/// panicked there in a debug build and wrapped to the far side of the grid in a release one.
pub(super) fn neighbour_cell(
    cell: (i32, i32, i32),
    step: (i32, i32, i32),
) -> Option<(i32, i32, i32)> {
    Some((
        cell.0.checked_add(step.0)?,
        cell.1.checked_add(step.1)?,
        cell.2.checked_add(step.2)?,
    ))
}

/// Bonds whose length does not match the chemistry claimed for them.
///
/// StructCooker validates the same way (`transforms/ccd.py`): a bond is plausible when its
/// length is between 0.5 and 1.3 times the sum of the covalent radii - generous on the
/// high side, because strained and coordinate-error-laden models do have long bonds, and
/// tight on the low side, where two atoms in the same place mean a broken model rather
/// than a short bond.
///
/// This *reports*; it does not delete. A bond the file states is a fact about the
/// deposition, and a viewer that silently drops the ones it dislikes hides exactly the
/// problem someone would want to see.
pub fn implausible(structure: &Structure) -> Vec<(usize, f32, f32)> {
    const MAX_FACTOR: f32 = 1.3;
    const MIN_FACTOR: f32 = 0.5;
    structure
        .bonds
        .iter()
        .enumerate()
        .filter_map(|(index, bond)| {
            // Coordination is not a covalent bond, so covalent radii say nothing about it.
            if bond.kind == BondKind::Metal {
                return None;
            }
            let a = bond.atoms[0] as usize;
            let b = bond.atoms[1] as usize;
            let expected = chem::covalent_radius(structure.elements[a])
                + chem::covalent_radius(structure.elements[b]);
            let length = squared_distance(structure.position(a), structure.position(b)).sqrt();
            let ratio = length / expected.max(1e-3);
            (!(MIN_FACTOR..=MAX_FACTOR).contains(&ratio)).then_some((index, length, expected))
        })
        .collect()
}

/// Rejects pairs that are never bonded to each other regardless of distance.
fn may_bond(a: Element, b: Element) -> bool {
    // Two hydrogens close together are a clash or a rotamer artifact, never a bond.
    if a == chem::HYDROGEN && b == chem::HYDROGEN {
        return false;
    }
    a != chem::UNKNOWN || b != chem::UNKNOWN
}

fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::model::RawAtom;

    fn atom(name: &str, element: Element, position: [f32; 3], residue: (&str, i32)) -> RawAtom {
        RawAtom {
            name: name.to_string(),
            element,
            position,
            b_factor: 0.0,
            occupancy: 1.0,
            residue_name: residue.0.into(),
            residue_number: residue.1,
            insertion_code: None,
            chain_id: "A".into(),
            is_hetatm: !matches!(residue.0, "ALA" | "GLY" | "CYS"),
            label_asym_id: String::new(),
            entity_id: String::new(),
        }
    }

    /// Two alanines, laid out with real bond lengths along x.
    fn dipeptide() -> Structure {
        let atoms = vec![
            atom(" N  ", chem::NITROGEN, [0.0, 0.0, 0.0], ("ALA", 1)),
            atom(" CA ", chem::CARBON, [1.47, 0.0, 0.0], ("ALA", 1)),
            atom(" C  ", chem::CARBON, [2.99, 0.0, 0.0], ("ALA", 1)),
            atom(" O  ", chem::OXYGEN, [3.6, 1.05, 0.0], ("ALA", 1)),
            atom(" CB ", chem::CARBON, [1.95, -1.45, 0.0], ("ALA", 1)),
            atom(" N  ", chem::NITROGEN, [4.32, 0.0, 0.0], ("ALA", 2)),
            atom(" CA ", chem::CARBON, [5.79, 0.0, 0.0], ("ALA", 2)),
            atom(" C  ", chem::CARBON, [7.31, 0.0, 0.0], ("ALA", 2)),
            atom(" O  ", chem::OXYGEN, [7.92, 1.05, 0.0], ("ALA", 2)),
            atom(" CB ", chem::CARBON, [6.27, -1.45, 0.0], ("ALA", 2)),
        ];
        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        build(&mut structure, &Evidence::default());
        structure
    }

    fn bond_between(structure: &Structure, a: usize, b: usize) -> Option<Bond> {
        structure
            .bonds
            .iter()
            .copied()
            .find(|bond| bond.atoms == [a.min(b) as u32, a.max(b) as u32])
    }

    #[test]
    fn a_residues_bonds_come_from_its_template() {
        let structure = dipeptide();
        // N-CA, CA-C, C=O, CA-CB in each residue.
        let n_ca = bond_between(&structure, 0, 1).expect("N-CA");
        assert_eq!(n_ca.kind, BondKind::Template);
        assert_eq!(n_ca.order, BondOrder::Single);
        let carbonyl = bond_between(&structure, 2, 3).expect("C=O");
        assert_eq!(
            carbonyl.order,
            BondOrder::Double,
            "the dictionary knows this"
        );
        assert!(bond_between(&structure, 1, 4).is_some(), "CA-CB");
        // And nothing the template does not list, however close: N and CB are 1.9 A apart
        // here, which a distance rule would happily bond.
        assert!(
            bond_between(&structure, 0, 4).is_none(),
            "N-CB is not a bond"
        );
    }

    #[test]
    fn consecutive_residues_are_linked_through_the_backbone() {
        let structure = dipeptide();
        let peptide = bond_between(&structure, 2, 5).expect("C of 1 to N of 2");
        assert_eq!(peptide.kind, BondKind::Link);
        // And the residue view says the two residues are joined.
        assert_eq!(structure.residue_links.len(), 1);
        assert_eq!(structure.residue_links[0].residues, [0, 1]);
        assert_eq!(structure.residue_links[0].kind, BondKind::Link);
    }

    #[test]
    fn a_chain_break_is_not_bonded_across() {
        // The same two residues, with the second moved 12 A away: consecutive numbering
        // across a missing loop is the commonest thing in the archive, and a stick drawn
        // across it is a lie about the model.
        let mut atoms: Vec<RawAtom> = Vec::new();
        for (index, template) in [("ALA", 1), ("ALA", 2)].iter().enumerate() {
            let shift = index as f32 * 12.0;
            atoms.push(atom(" N  ", chem::NITROGEN, [shift, 0.0, 0.0], *template));
            atoms.push(atom(
                " CA ",
                chem::CARBON,
                [shift + 1.47, 0.0, 0.0],
                *template,
            ));
            atoms.push(atom(
                " C  ",
                chem::CARBON,
                [shift + 2.99, 0.0, 0.0],
                *template,
            ));
        }
        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        build(&mut structure, &Evidence::default());
        assert!(
            bond_between(&structure, 2, 3).is_none(),
            "not across the gap"
        );
        assert!(structure.residue_links.is_empty());
    }

    #[test]
    fn struct_conn_makes_the_disulfide_and_the_metal_stays_a_contact() {
        // Two cysteines 2.03 A apart at their sulfurs, which is a disulfide - and the file
        // says so, so it does not matter that no template can.
        let atoms = vec![
            atom(" N  ", chem::NITROGEN, [0.0, 0.0, 0.0], ("CYS", 5)),
            atom(" CA ", chem::CARBON, [1.47, 0.0, 0.0], ("CYS", 5)),
            atom(" CB ", chem::CARBON, [1.95, -1.45, 0.0], ("CYS", 5)),
            atom(" SG ", chem::SULFUR, [3.6, -1.9, 0.0], ("CYS", 5)),
            atom(" N  ", chem::NITROGEN, [10.0, 0.0, 0.0], ("CYS", 40)),
            atom(" CA ", chem::CARBON, [11.47, 0.0, 0.0], ("CYS", 40)),
            atom(" CB ", chem::CARBON, [11.95, -1.45, 0.0], ("CYS", 40)),
            atom(" SG ", chem::SULFUR, [5.62, -1.9, 0.0], ("CYS", 40)),
            atom("FE  ", 26, [3.6, -5.0, 0.0], ("HEM", 99)),
        ];
        let reference = |number: i32, comp: &str, name: &str| AtomRef {
            chain: "A".into(),
            label_chain: None,
            number: Some(number),
            comp_id: comp.into(),
            atom_name: name.into(),
            insertion_code: None,
        };
        let evidence = Evidence {
            templates: HashMap::new(),
            connections: vec![
                Connection {
                    kind: BondKind::Disulfide,
                    a: reference(5, "CYS", "SG"),
                    b: reference(40, "CYS", "SG"),
                },
                Connection {
                    kind: BondKind::Metal,
                    a: reference(5, "CYS", "SG"),
                    b: reference(99, "HEM", "FE"),
                },
            ],
        };
        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        build(&mut structure, &evidence);

        let disulfide = bond_between(&structure, 3, 7).expect("SG-SG");
        assert_eq!(disulfide.kind, BondKind::Disulfide);
        assert!(disulfide.is_covalent(), "a disulfide is drawn as a stick");

        let metal = bond_between(&structure, 3, 8).expect("SG-FE");
        assert_eq!(metal.kind, BondKind::Metal);
        assert!(
            !metal.is_covalent(),
            "coordination is a contact, not a stick - PyMOL does not draw it either",
        );

        // Both are residue-level links, whichever way they are drawn.
        assert_eq!(structure.residue_links.len(), 2);
    }

    #[test]
    fn a_component_nobody_has_a_template_for_falls_back_to_distance() {
        // A two-atom ligand with a made-up name: no dictionary, no built-in, so the only
        // thing left is the distance rule - and the bond says so.
        let atoms = vec![
            atom(" C1 ", chem::CARBON, [0.0, 0.0, 0.0], ("XYZ", 1)),
            atom(" O1 ", chem::OXYGEN, [1.43, 0.0, 0.0], ("XYZ", 1)),
            // And an atom too far away to bond.
            atom(" O2 ", chem::OXYGEN, [4.5, 0.0, 0.0], ("XYZ", 1)),
        ];
        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        build(&mut structure, &Evidence::default());
        let inferred = bond_between(&structure, 0, 1).expect("C1-O1");
        assert_eq!(inferred.kind, BondKind::Inferred);
        assert!(
            bond_between(&structure, 1, 2).is_none(),
            "3 A apart is not a bond"
        );
    }

    #[test]
    fn a_bond_whose_length_is_impossible_is_reported_not_deleted() {
        // Two alanines whose peptide bond is fine, and a template bond stretched to 3 A by
        // a bad model: the same check StructCooker runs, 0.5 to 1.3 times the covalent
        // radii sum.
        let structure = dipeptide();
        assert!(
            implausible(&structure).is_empty(),
            "a sane model has no implausible bonds",
        );

        let mut broken = dipeptide();
        // Move CB of the first residue 3.5 A from its CA.
        broken.positions[4 * 3] = 5.0;
        broken.positions[4 * 3 + 1] = 0.0;
        let reported = implausible(&broken);
        assert_eq!(reported.len(), 1, "{reported:?}");
        // Still in the graph: the file said it, and hiding it hides the problem.
        assert!(bond_between(&broken, 1, 4).is_some());
    }

    #[test]
    fn a_file_template_beats_the_built_in_one() {
        // A modified alanine: the file's dictionary lists a bond the built-in table does
        // not, and the file wins - which is the whole reason to read it.
        let atoms = vec![
            atom(" N  ", chem::NITROGEN, [0.0, 0.0, 0.0], ("ALA", 1)),
            atom(" CA ", chem::CARBON, [1.47, 0.0, 0.0], ("ALA", 1)),
            atom(" CB ", chem::CARBON, [1.95, -1.45, 0.0], ("ALA", 1)),
        ];
        let mut templates = HashMap::new();
        templates.insert(
            "ALA".to_string(),
            Template {
                linking: Linking::Peptide,
                bonds: vec![("N".into(), "CB".into(), BondOrder::Single)],
            },
        );
        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        build(
            &mut structure,
            &Evidence {
                templates,
                connections: Vec::new(),
            },
        );
        assert!(bond_between(&structure, 0, 2).is_some(), "the file's bond");
        assert!(
            bond_between(&structure, 0, 1).is_none(),
            "and only the file's: N-CA is not in this template",
        );
    }

    #[test]
    fn a_file_template_with_no_bonds_does_not_hide_the_built_in_one() {
        // What a file with `_chem_comp` and no `_chem_comp_bond` produces: every
        // component named, none of them bonded.
        let mut structure = dipeptide();
        let mut evidence = Evidence::default();
        evidence.templates.insert(
            "ALA".to_string(),
            Template {
                linking: Linking::Peptide,
                bonds: Vec::new(),
            },
        );
        build(&mut structure, &evidence);
        let kinds: Vec<BondKind> = structure.bonds.iter().map(|bond| bond.kind).collect();
        assert!(
            kinds.contains(&BondKind::Template),
            "the residues have their bonds"
        );
        assert!(
            kinds.contains(&BondKind::Link),
            "and the peptide link between them"
        );
        assert!(!kinds.contains(&BondKind::Inferred));
    }

    #[test]
    fn a_modified_residue_stays_in_its_chain() {
        // Phosphoserine between two alanines, as a PDB file has it: no dictionary, so
        // nothing knows SEP - but it has a backbone, and the chain runs through it.
        let atoms = vec![
            atom(" N  ", chem::NITROGEN, [0.0, 0.0, 0.0], ("ALA", 1)),
            atom(" CA ", chem::CARBON, [1.47, 0.0, 0.0], ("ALA", 1)),
            atom(" C  ", chem::CARBON, [2.99, 0.0, 0.0], ("ALA", 1)),
            atom(" N  ", chem::NITROGEN, [4.32, 0.0, 0.0], ("SEP", 2)),
            atom(" CA ", chem::CARBON, [5.79, 0.0, 0.0], ("SEP", 2)),
            atom(" C  ", chem::CARBON, [7.31, 0.0, 0.0], ("SEP", 2)),
            atom(" N  ", chem::NITROGEN, [8.64, 0.0, 0.0], ("ALA", 3)),
            atom(" CA ", chem::CARBON, [10.11, 0.0, 0.0], ("ALA", 3)),
            atom(" C  ", chem::CARBON, [11.63, 0.0, 0.0], ("ALA", 3)),
        ];
        let mut structure = Structure::build("t".into(), "t".into(), atoms);
        build(&mut structure, &Evidence::default());
        let links: Vec<[u32; 2]> = structure
            .bonds
            .iter()
            .filter(|bond| bond.kind == BondKind::Link)
            .map(|bond| bond.atoms)
            .collect();
        assert_eq!(links, vec![[2, 3], [5, 6]]);
        // Its own bonds are still the distance pass's, and say so.
        assert!(structure
            .bonds
            .iter()
            .any(|bond| bond.kind == BondKind::Inferred));
    }

    #[test]
    fn empty_and_single_atom_structures_have_no_bonds() {
        let mut empty = Structure::build("t".into(), "t".into(), Vec::new());
        build(&mut empty, &Evidence::default());
        assert!(empty.bonds.is_empty() && empty.residue_links.is_empty());

        let mut one = Structure::build(
            "t".into(),
            "t".into(),
            vec![atom(" C1 ", chem::CARBON, [0.0, 0.0, 0.0], ("XYZ", 1))],
        );
        build(&mut one, &Evidence::default());
        assert!(one.bonds.is_empty());
    }

    #[test]
    fn consecutive_nucleotides_link_o3_to_p_even_across_dna_and_rna() {
        // A DNA-RNA hybrid strand is one strand: DA then A, O3' to P 1.6 A apart.
        let atoms = vec![
            atom(" C3'", chem::CARBON, [0.0, 0.0, 0.0], ("DA", 1)),
            atom(" O3'", chem::OXYGEN, [1.43, 0.0, 0.0], ("DA", 1)),
            atom(" P  ", 15, [3.03, 0.0, 0.0], ("A", 2)),
            atom(" OP1", chem::OXYGEN, [3.5, 1.4, 0.0], ("A", 2)),
        ];
        let mut structure = Structure::build("t".into(), "t".into(), atoms);
        build(&mut structure, &Evidence::default());
        let link = bond_between(&structure, 1, 2).expect("O3'-P");
        assert_eq!(link.kind, BondKind::Link);
        assert!(
            bond_between(&structure, 0, 1).is_some(),
            "C3'-O3' from the DA template"
        );
    }

    #[test]
    fn a_protein_is_not_linked_to_a_nucleotide_or_a_water_after_it() {
        let atoms = vec![
            atom(" C  ", chem::CARBON, [0.0, 0.0, 0.0], ("ALA", 1)),
            atom(" P  ", 15, [1.3, 0.0, 0.0], ("DA", 2)),
            atom(" O  ", chem::OXYGEN, [2.6, 0.0, 0.0], ("HOH", 3)),
        ];
        let mut structure = Structure::build("t".into(), "t".into(), atoms);
        build(&mut structure, &Evidence::default());
        assert!(
            structure
                .bonds
                .iter()
                .all(|bond| bond.kind != BondKind::Link),
            "{:?}",
            structure.bonds
        );
    }

    #[test]
    fn a_connection_naming_atoms_the_model_lacks_is_dropped() {
        let mut structure = dipeptide();
        let before = structure.bonds.clone();
        let missing = AtomRef {
            chain: "Z".into(),
            label_chain: None,
            number: Some(1),
            comp_id: "ALA".into(),
            atom_name: "CB".into(),
            insertion_code: None,
        };
        let present = AtomRef {
            chain: "A".into(),
            ..missing.clone()
        };
        let evidence = Evidence {
            templates: HashMap::new(),
            connections: vec![
                Connection {
                    kind: BondKind::Covalent,
                    a: missing,
                    b: present.clone(),
                },
                Connection {
                    kind: BondKind::Covalent,
                    a: present.clone(),
                    b: AtomRef {
                        atom_name: "XX".into(),
                        ..present
                    },
                },
            ],
        };
        build(&mut structure, &evidence);
        assert_eq!(structure.bonds, before);
    }

    #[test]
    fn two_atoms_in_the_same_place_are_not_bonded_by_distance() {
        // A duplicated atom - two files merged, or a broken model - is not a bond.
        let atoms = vec![
            atom(" C1 ", chem::CARBON, [0.0, 0.0, 0.0], ("XYZ", 1)),
            atom(" C2 ", chem::CARBON, [0.1, 0.0, 0.0], ("XYZ", 1)),
        ];
        let mut structure = Structure::build("t".into(), "t".into(), atoms);
        build(&mut structure, &Evidence::default());
        assert!(structure.bonds.is_empty());
    }

    #[test]
    fn a_lone_ion_is_not_wired_to_its_neighbours_by_distance() {
        // Magnesium 2.1 A from an oxygen of an unknown ligand: coordination, not a bond.
        let mut magnesium = atom("MG  ", chem::from_symbol("Mg"), [0.0, 0.0, 0.0], ("MG", 1));
        magnesium.chain_id = "B".into();
        let atoms = vec![
            magnesium,
            atom(" O1 ", chem::OXYGEN, [2.1, 0.0, 0.0], ("LIG", 2)),
            atom(" C1 ", chem::CARBON, [3.5, 0.0, 0.0], ("LIG", 2)),
        ];
        let mut structure = Structure::build("t".into(), "t".into(), atoms);
        build(&mut structure, &Evidence::default());
        assert_eq!(structure.bonds.len(), 1, "only the ligand's own O1-C1");
        assert_eq!(structure.bonds[0].atoms, [1, 2]);
    }
}
