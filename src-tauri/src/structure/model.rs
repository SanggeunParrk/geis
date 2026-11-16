//! The in-memory structure, and the wire format that gets it to the renderer.
//!
//! Per-atom data is kept in parallel arrays rather than a Vec of structs: that is the
//! layout the GPU wants, so the wire format is a memcpy of what is already here rather
//! than a transformation. A 200k-atom structure is ~5 MB this way, which crosses the IPC
//! boundary in a few milliseconds.

use super::chem::{self, Element};
use serde::{Deserialize, Serialize};

/// Bumped whenever the binary layout changes, so a stale frontend fails loudly instead of
/// misreading buffers.
pub const WIRE_VERSION: u32 = 1;
pub const WIRE_MAGIC: &[u8; 4] = b"MSTR";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecondaryStructure {
    Coil = 0,
    Helix = 1,
    Sheet = 2,
    Turn = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Residue {
    /// Three-letter code (or nucleotide / ligand code).
    pub name: String,
    /// Author-assigned residue number, which is what everyone actually cites.
    pub number: i32,
    pub insertion_code: Option<char>,
    pub chain_index: u32,
    /// Index range into the atom arrays: [first, first + count).
    pub first_atom: u32,
    pub atom_count: u32,
    pub secondary: SecondaryStructure,
    pub kind: ResidueKind,
    /// Atom the cartoon spline passes through: CA for protein, P for nucleic acids.
    /// `-1` when the residue has none, which happens in incomplete models.
    pub trace_atom: i32,
    /// Atom that orients the ribbon's flat face: the carbonyl O for protein, C1' for
    /// nucleic acids. Without it a ribbon has no defined twist and would spin freely.
    pub orient_atom: i32,
}

/// Per-atom flags, so the renderer can filter by role without carrying atom-name strings
/// across the boundary. Hiding waters and hydrogens is the first thing anyone does to a
/// structure, and it has to be a bitmask test rather than a string compare per atom per
/// frame.
pub mod flags {
    pub const BACKBONE: u8 = 1 << 0;
    pub const SIDECHAIN: u8 = 1 << 1;
    pub const WATER: u8 = 1 << 2;
    pub const LIGAND: u8 = 1 << 3;
    /// Part of a protein or nucleic chain, as opposed to solvent or a ligand.
    pub const POLYMER: u8 = 1 << 4;
    pub const HYDROGEN: u8 = 1 << 5;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidueKind {
    AminoAcid,
    Nucleotide,
    Water,
    Ligand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chain {
    /// The chain's id in the form BioMol uses: `asym_id_oper_id`, e.g. `A_1`.
    ///
    /// The asym id is the mmCIF `label_asym_id`, which is the archive's own identity for a
    /// chain and separates a haem or a run of waters from the polymer they were deposited
    /// beside - author chain A of 4HHB is asym A (protein), E (haem) and K (water). The
    /// operator id is the assembly operator that produced this copy; only the asymmetric
    /// unit is built here, so it is always `1`, and this is the field that changes when
    /// assemblies arrive.
    pub id: String,
    /// The mmCIF `label_asym_id` on its own.
    pub label_asym_id: String,
    /// Author chain id - the letter in the paper, and what a PDB file has.
    pub auth_id: String,
    /// The mmCIF entity this chain is an instance of; empty for a PDB file.
    pub entity_id: String,
    /// `polymer`, `non-polymer`, `water` or `branched` - the mmCIF `_entity.type`, or the
    /// same words inferred from the residues when a file has no entity table.
    pub entity_type: String,
    /// Index of the first residue seen for this chain.
    ///
    /// A chain's residues are *not* guaranteed to be contiguous: depositions interleave
    /// a chain's polymer residues with its ligands and with other chains, so this is a
    /// starting point, not a range. Use `Residue::chain_index` to test membership.
    pub first_residue: u32,
    pub residue_count: u32,
}

/// One atom, used only while parsing. The parsers append these and `Structure::build`
/// turns them into the parallel arrays.
#[derive(Debug, Clone)]
pub struct RawAtom {
    pub name: String,
    pub element: Element,
    pub position: [f32; 3],
    pub b_factor: f32,
    pub occupancy: f32,
    pub residue_name: String,
    pub residue_number: i32,
    pub insertion_code: Option<char>,
    /// Author chain id.
    pub chain_id: String,
    pub is_hetatm: bool,
    /// The mmCIF `label_asym_id`. Empty means "the same as the author id", which is what a
    /// PDB file and every test fixture have.
    pub label_asym_id: String,
    /// The mmCIF `label_entity_id`, or empty.
    pub entity_id: String,
}

#[derive(Debug, Default, Clone)]
pub struct Structure {
    pub title: String,
    pub source: String,
    /// The entry id - the PDB id for an archive file, the data block name otherwise.
    pub entry_id: String,
    /// How the model was determined, as the file states it: `X-RAY DIFFRACTION`,
    /// `ELECTRON MICROSCOPY`. Absent from a prediction, which is the point of keeping it -
    /// a file that names an experiment has B-factors in its B-factor column.
    pub experimental_method: Option<String>,

    // Parallel per-atom arrays.
    pub positions: Vec<f32>,
    pub elements: Vec<Element>,
    pub b_factors: Vec<f32>,
    pub residue_of_atom: Vec<u32>,
    pub atom_names: Vec<String>,
    pub flags: Vec<u8>,

    pub residues: Vec<Residue>,
    pub chains: Vec<Chain>,
    /// The atom-level bond graph.
    ///
    /// An edge feature of the atom view, in BioMol's terms: every bond carries its order
    /// and where it came from, because "these two atoms are 1.5 A apart" and "the
    /// dictionary says this is a double bond" are different facts and only one of them is
    /// chemistry.
    pub bonds: Vec<Bond>,
    /// The residue-level bond graph, derived from `bonds`.
    ///
    /// Which residues are joined, and how: the peptide chain, a disulfide, a glycan link,
    /// a metal coordinated by two histidines. The same edges a residue view carries.
    pub residue_links: Vec<ResidueLink>,
    /// What the file offers to be read as, and what this structure was read as.
    pub versions: Versions,
}

/// The ways one entry can be read, and the way this structure was.
///
/// Three different things, kept apart because they mean different things: an assembly is
/// which molecules, and how many copies - the biological unit, or the crystal's; a model
/// is one whole state of the molecule - an NMR ensemble's members, the frames of a
/// trajectory; an alternate location is a local doubt - a side chain seen in two places,
/// each with part of the occupancy - and is chosen residue by residue, not for the whole
/// entry. StructCooker files each (assembly, model, altloc) as a whole structure of its
/// own, which suits a training set and not a molecule being looked at.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Versions {
    pub assemblies: Vec<AssemblyInfo>,
    /// Model numbers, in file order. One, or none, for most crystal structures.
    pub models: Vec<String>,
    /// The alternate-location labels anywhere in the read model, in order.
    pub alternates: Vec<String>,
    /// How many residues have more than one conformer.
    pub alternate_residues: usize,
    /// Which assembly this is, or None for the deposited coordinates.
    pub assembly: Option<String>,
    /// Which model this is.
    pub model: Option<String>,
    /// The label asked for, or None for each residue's most occupied conformer.
    pub alternate: Option<String>,
}

/// One of an entry's assemblies, as `_pdbx_struct_assembly` describes it.
#[derive(Debug, Clone, Serialize)]
pub struct AssemblyInfo {
    pub id: String,
    pub details: String,
    /// How many chains it holds, as the file counts them.
    pub count: Option<u32>,
}

/// One bond between two atoms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bond {
    pub atoms: [u32; 2],
    pub order: BondOrder,
    pub kind: BondKind,
}

impl Bond {
    pub fn new(a: u32, b: u32, order: BondOrder, kind: BondKind) -> Self {
        Self {
            atoms: [a.min(b), a.max(b)],
            order,
            kind,
        }
    }

    /// Whether this is a bond a stick model should draw.
    ///
    /// Metal coordination is not: PyMOL does not draw His NE2 to a haem iron as a stick
    /// either, because a stick means a covalent bond and that is not one. It stays in the
    /// graph, where an interaction view can draw it as the dashed line it is.
    pub fn is_covalent(self) -> bool {
        self.kind != BondKind::Metal
    }
}

/// Bond order, as `_chem_comp_bond.value_order` gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BondOrder {
    Single,
    Double,
    Triple,
    /// Flagged aromatic by the dictionary, which is a statement about the ring rather than
    /// about one bond's order.
    Aromatic,
}

/// Where a bond came from. Provenance, not decoration: a bond read from a dictionary and a
/// bond guessed from a distance deserve different trust, and the UI should be able to say
/// which it is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BondKind {
    /// From a chemical component template - the file's `_chem_comp_bond` or the built-in
    /// table. The normal case, and the only one that knows bond orders.
    Template,
    /// The polymer backbone link between consecutive residues, which no component
    /// dictionary can carry because it is between two of them.
    Link,
    /// `_struct_conn` disulf.
    Disulfide,
    /// `_struct_conn` covale: a covalent modification, a glycan, a ligand bonded to the
    /// protein.
    Covalent,
    /// `_struct_conn` metalc: coordination, not a covalent bond.
    Metal,
    /// Inferred from distance, because nothing else said anything. A component with no
    /// template in a file with no dictionary.
    Inferred,
}

/// A bond between two residues, in the residue view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResidueLink {
    pub residues: [u32; 2],
    pub kind: BondKind,
}

/// A residue as alternate locations are grouped: its chain, number and insertion code.
type ResidueKey = (String, i32, Option<char>);

/// Which alternate location each residue keeps.
///
/// An alternate location is local: a side chain, a loop, a water seen in two places, each
/// with part of the occupancy, and residues elsewhere have labels of their own or none.
/// So it is chosen residue by residue - the label asked for where the residue has it, and
/// otherwise the conformer with the most occupancy (the first written, if they tie), as
/// ChimeraX does. Atoms with no label belong to every conformer.
#[derive(Debug, Default)]
pub struct Conformers {
    /// Per residue, each label's summed occupancy, in the order the labels appear.
    seen: std::collections::HashMap<ResidueKey, Vec<(char, f32)>>,
}

impl Conformers {
    fn key(atom: &RawAtom) -> ResidueKey {
        (
            format!("{}/{}", atom.chain_id, atom.label_asym_id),
            atom.residue_number,
            atom.insertion_code,
        )
    }

    /// Notes an atom: its residue, its label and its occupancy.
    pub fn note(&mut self, atom: &RawAtom, alt_loc: Option<char>) {
        let Some(label) =
            alt_loc.filter(|label| !label.is_whitespace() && *label != '.' && *label != '?')
        else {
            return;
        };
        let labels = self.seen.entry(Self::key(atom)).or_default();
        match labels.iter_mut().find(|(seen, _)| *seen == label) {
            Some((_, occupancy)) => *occupancy += atom.occupancy,
            None => labels.push((label, atom.occupancy)),
        }
    }

    /// Residues with more than one conformer.
    pub fn doubled(&self) -> usize {
        self.seen.values().filter(|labels| labels.len() > 1).count()
    }

    /// Every label, in the order first seen across the residues.
    pub fn labels(&self) -> Vec<char> {
        let mut all: Vec<char> = self
            .seen
            .values()
            .flatten()
            .map(|(label, _)| *label)
            .collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    /// Whether an atom belongs to the conformer kept for its residue.
    pub fn keep(&self, atom: &RawAtom, alt_loc: Option<char>, wanted: Option<char>) -> bool {
        let Some(label) =
            alt_loc.filter(|label| !label.is_whitespace() && *label != '.' && *label != '?')
        else {
            return true;
        };
        let Some(labels) = self.seen.get(&Self::key(atom)) else {
            return true;
        };
        if let Some(wanted) = wanted {
            if labels.iter().any(|(seen, _)| *seen == wanted) {
                return label == wanted;
            }
        }
        let mut best = labels[0];
        for &candidate in &labels[1..] {
            if candidate.1 > best.1 + 1e-4 {
                best = candidate;
            }
        }
        label == best.0
    }
}

/// How a component links into a polymer, from `_chem_comp.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Linking {
    /// Peptide: C of one residue to N of the next.
    Peptide,
    /// RNA: O3' to P.
    Rna,
    /// DNA: the same link as RNA.
    Dna,
    /// Links to nothing: a ligand, an ion, a water.
    NonPolymer,
}

impl Linking {
    /// The two atom names a link between consecutive residues uses, and how far apart they
    /// may be before it is a chain break rather than a bond.
    ///
    /// Distance appears here and nowhere else in bonding: not to *find* the bond, which the
    /// polymer sequence already states, but to notice that the sequence is broken. A file
    /// with a missing loop numbers its residues consecutively across the gap, and bonding
    /// across it draws a stick through empty space.
    pub fn link_atoms(self) -> Option<(&'static str, &'static str, f32)> {
        match self {
            Linking::Peptide => Some(("C", "N", 2.0)),
            Linking::Rna | Linking::Dna => Some(("O3\'", "P", 2.2)),
            Linking::NonPolymer => None,
        }
    }

    /// From the `_chem_comp.type` string, which is free-form enough to need matching
    /// rather than parsing: "L-peptide linking", "D-PEPTIDE LINKING", "RNA linking".
    pub fn from_chem_comp_type(kind: &str) -> Self {
        let upper = kind.to_ascii_uppercase();
        if upper.contains("PEPTIDE") && upper.contains("LINKING") {
            Linking::Peptide
        } else if upper.contains("RNA") && upper.contains("LINKING") {
            Linking::Rna
        } else if upper.contains("DNA") && upper.contains("LINKING") {
            Linking::Dna
        } else {
            Linking::NonPolymer
        }
    }
}

/// Metadata the frontend reads before it touches a single byte of geometry.
#[derive(Debug, Clone, Serialize)]
pub struct StructureHeader {
    pub version: u32,
    pub title: String,
    pub source: String,
    pub atom_count: u32,
    pub bond_count: u32,
    pub residues: Vec<ResidueSummary>,
    pub chains: Vec<Chain>,
    /// Axis-aligned bounds, so the camera can frame the structure without scanning it
    /// again on the other side of the boundary.
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub center: [f32; 3],
    /// Radius of the bounding sphere about `center`.
    pub radius: f32,
    /// Observed B-factor range. For an AlphaFold model this is the pLDDT range, which is
    /// why the viewer can offer a pLDDT colour scheme without being told the provenance.
    pub b_factor_min: f32,
    pub b_factor_max: f32,
    /// Byte offsets of each block in the payload that follows the header.
    pub blocks: Blocks,
}

/// A residue as the frontend needs it - the full `Residue` carries parsing detail the
/// renderer never reads.
#[derive(Debug, Clone, Serialize)]
pub struct ResidueSummary {
    pub name: String,
    /// One-letter code, for a sequence strip. `X` for anything not a standard residue.
    pub code: String,
    pub number: i32,
    pub chain_index: u32,
    pub first_atom: u32,
    pub atom_count: u32,
    pub secondary: u8,
    pub kind: u8,
    pub trace_atom: i32,
    pub orient_atom: i32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Blocks {
    /// f32 x 3 per atom.
    pub positions: Block,
    /// u8 per atom (atomic number).
    pub elements: Block,
    /// f32 per atom.
    pub b_factors: Block,
    /// u32 per atom.
    pub residue_of_atom: Block,
    /// u32 x 2 per bond.
    pub bonds: Block,
    /// u8 per atom - see `model::flags`.
    pub flags: Block,
    /// 256 x 3 bytes: CPK colour indexed by atomic number.
    ///
    /// Shipped with the geometry so the element table has one definition. A copy of it in
    /// the frontend would drift, and the day it did, oxygen would be the wrong colour in
    /// one representation and right in another.
    pub palette: Block,
    /// 256 f32: van der Waals radius indexed by atomic number, for the same reason.
    pub vdw_radii: Block,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Block {
    pub offset: u32,
    pub length: u32,
}

/// What separates the asym id from the operator in the label of an atom that an
/// assembly operator made: `A`, this, `2`. A control character, so it cannot be part of
/// either. See `cif::build_assembly`.
pub const OPERATOR_MARK: char = '\u{1f}';

/// A chain label taken apart: the asym id, the operator that made this copy, and the
/// asym id as the file's other records will name it.
///
/// Three forms arrive here. A copy this reader made itself from the assembly records is
/// `A`, the mark, `2`: asym `A`, operator `2`, and the file's records call it `A`. A chain
/// of one of the archive's ready-made assembly files is `A-2`, which says the same thing
/// and which that file's own records also use. Anything else is a chain as deposited,
/// operator `1` - the form StructCooker and BioMol use, `asym_id_oper_id`.
fn assembly_copy(label: &str) -> (&str, &str, &str) {
    if let Some((asym, operator)) = label.split_once(OPERATOR_MARK) {
        return (asym, operator, asym);
    }
    match label.rsplit_once('-') {
        Some((asym, operator))
            if !asym.is_empty()
                && !operator.is_empty()
                && operator.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (asym, operator, label)
        }
        _ => (label, "1", label),
    }
}

impl Structure {
    pub fn atom_count(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn position(&self, atom: usize) -> [f32; 3] {
        [
            self.positions[atom * 3],
            self.positions[atom * 3 + 1],
            self.positions[atom * 3 + 2],
        ]
    }

    /// Groups parsed atoms into residues and chains.
    ///
    /// Residue identity is (chain, number, insertion code, name). Grouping on a change of
    /// any of those - rather than on number alone - is what keeps insertion codes (100A,
    /// 100B) and microheterogeneity from collapsing into one residue.
    pub fn build(title: String, source: String, atoms: Vec<RawAtom>) -> Self {
        let mut structure = Structure {
            title,
            source,
            ..Default::default()
        };

        // Chains are keyed by id rather than grouped by adjacency. A chain id reappears
        // later in the file more often than not - 6VXX has 18 chains across 21 runs of
        // consecutive atoms - and adjacency grouping turns each run into its own chain,
        // so chain colouring and per-chain counts both come out wrong.
        //
        // Keyed by the *label* asym id when the file has one: that is the archive's own
        // notion of a chain, and it is what tells a haem apart from the protein it sits in
        // and both apart from the waters around them. A PDB file has only author ids, so
        // the label falls back to the author id and the two agree.
        let mut chain_indices: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        // The residue being filled, by index into `atoms`: compared field by field rather
        // than as a key made per atom, which was two strings allocated for each of a
        // capsid's three and a half million atoms.
        let mut current_residue: Option<usize> = None;
        let same_residue = |a: &RawAtom, b: &RawAtom| {
            a.residue_number == b.residue_number
                && a.insertion_code == b.insertion_code
                && a.residue_name == b.residue_name
                && if a.label_asym_id.is_empty() || b.label_asym_id.is_empty() {
                    a.chain_id == b.chain_id && a.label_asym_id == b.label_asym_id
                } else {
                    a.label_asym_id == b.label_asym_id
                }
        };
        structure.positions.reserve(atoms.len() * 3);
        structure.elements.reserve(atoms.len());
        structure.b_factors.reserve(atoms.len());
        structure.residue_of_atom.reserve(atoms.len());
        structure.atom_names.reserve(atoms.len());
        structure.flags.reserve(atoms.len());

        for (at, atom) in atoms.iter().enumerate() {
            let atom_index = structure.elements.len() as u32;
            let label = if atom.label_asym_id.is_empty() {
                atom.chain_id.as_str()
            } else {
                atom.label_asym_id.as_str()
            };

            let chain_index = match chain_indices.get(label) {
                Some(&index) => index,
                None => {
                    let index = structure.chains.len() as u32;
                    // `asym_id_oper_id`: which chain of the deposition, and which
                    // operator of the assembly made this copy of it.
                    let (asym, operator, recorded) = assembly_copy(label);
                    structure.chains.push(Chain {
                        id: format!("{asym}_{operator}"),
                        label_asym_id: recorded.to_string(),
                        auth_id: assembly_copy(&atom.chain_id).0.to_string(),
                        entity_id: atom.entity_id.clone(),
                        entity_type: String::new(),
                        first_residue: structure.residues.len() as u32,
                        residue_count: 0,
                    });
                    chain_indices.insert(label.to_string(), index);
                    index
                }
            };

            if !current_residue.is_some_and(|previous| same_residue(&atoms[previous], atom)) {
                structure.residues.push(Residue {
                    name: atom.residue_name.clone(),
                    number: atom.residue_number,
                    insertion_code: atom.insertion_code,
                    chain_index,
                    first_atom: atom_index,
                    atom_count: 0,
                    secondary: SecondaryStructure::Coil,
                    kind: classify_residue(&atom.residue_name, atom.is_hetatm),
                    trace_atom: -1,
                    orient_atom: -1,
                });
                structure.chains[chain_index as usize].residue_count += 1;
                current_residue = Some(at);
            }

            let residue_index = structure.residues.len() as u32 - 1;
            if let Some(residue) = structure.residues.last_mut() {
                residue.atom_count += 1;
            }

            structure.positions.extend_from_slice(&atom.position);
            structure.elements.push(atom.element);
            structure.b_factors.push(atom.b_factor);
            structure.residue_of_atom.push(residue_index);
            structure.atom_names.push(atom.name.clone());
            let kind = structure.residues[residue_index as usize].kind;
            structure
                .flags
                .push(atom_flags(&atom.name, kind, atom.element));

            // Note the atoms the cartoon needs while their names are still in hand: the
            // renderer receives flags and indices, not strings.
            let name = atom.name.trim();
            let residue = &mut structure.residues[residue_index as usize];
            match kind {
                ResidueKind::AminoAcid => {
                    if name == "CA" && residue.trace_atom < 0 {
                        residue.trace_atom = atom_index as i32;
                    } else if name == "O" && residue.orient_atom < 0 {
                        residue.orient_atom = atom_index as i32;
                    }
                }
                ResidueKind::Nucleotide => {
                    if name == "P" && residue.trace_atom < 0 {
                        residue.trace_atom = atom_index as i32;
                    } else if name == "C1'" && residue.orient_atom < 0 {
                        residue.orient_atom = atom_index as i32;
                    }
                }
                _ => {}
            }
        }

        // Millions of small strings to give back: done beside the rest of the reading
        // rather than in its way.
        if atoms.len() > 100_000 {
            std::thread::spawn(move || drop(atoms));
        }
        structure.adopt_modified_nucleotides();
        structure.trace_unphosphorylated_ends();
        structure
    }

    /// Takes modified nucleotides into the strand they are part of.
    ///
    /// A tRNA is a fifth modified bases - pseudouridine, dihydrouridine, the methylated
    /// guanines - each under a component id of its own that no list of names will ever
    /// finish. Read as ligands they cut the backbone at every one and stood beside it as
    /// loose atoms. What makes one a nucleotide is not its name: it has a sugar, and its
    /// phosphate is bonded to the sugar of the nucleotide before it, or its sugar to the
    /// phosphate of the one after. A nucleotide that is merely bound - ATP in a pocket -
    /// has the sugar and is bonded to neither.
    fn adopt_modified_nucleotides(&mut self) {
        const BOND: f32 = 2.0;
        let atom_named = |structure: &Structure, residue: &Residue, name: &str| {
            let first = residue.first_atom as usize;
            (first..first + residue.atom_count as usize)
                .find(|&atom| structure.atom_names[atom].trim() == name)
        };
        let near = |structure: &Structure, a: usize, b: usize| {
            let (p, q) = (structure.position(a), structure.position(b));
            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt() < BOND
        };
        // Repeated, because two modified nucleotides in a row adopt each other in turn.
        loop {
            let mut adopted = Vec::new();
            for index in 0..self.residues.len() {
                let residue = &self.residues[index];
                if residue.kind != ResidueKind::Ligand {
                    continue;
                }
                if !["C1'", "C3'", "C4'", "O4'"]
                    .iter()
                    .all(|name| atom_named(self, residue, name).is_some())
                {
                    continue;
                }
                let phosphorus = atom_named(self, residue, "P");
                let o3 = atom_named(self, residue, "O3'");
                let joined = self
                    .residues
                    .iter()
                    .enumerate()
                    .any(|(other_index, other)| {
                        other_index != index
                            && other.chain_index == residue.chain_index
                            && other.kind == ResidueKind::Nucleotide
                            && (phosphorus
                                .zip(atom_named(self, other, "O3'"))
                                .is_some_and(|(p, o)| near(self, p, o))
                                || o3
                                    .zip(atom_named(self, other, "P"))
                                    .is_some_and(|(o, p)| near(self, o, p)))
                    });
                if joined {
                    adopted.push(index);
                }
            }
            if adopted.is_empty() {
                break;
            }
            for index in adopted {
                self.residues[index].kind = ResidueKind::Nucleotide;
                let first = self.residues[index].first_atom as usize;
                for atom in first..first + self.residues[index].atom_count as usize {
                    // Flagged as a ligand's when it was read; it is the strand's now.
                    self.flags[atom] = atom_flags(
                        &self.atom_names[atom],
                        ResidueKind::Nucleotide,
                        self.elements[atom],
                    );
                    match self.atom_names[atom].trim() {
                        "P" if self.residues[index].trace_atom < 0 => {
                            self.residues[index].trace_atom = atom as i32
                        }
                        "C1'" if self.residues[index].orient_atom < 0 => {
                            self.residues[index].orient_atom = atom as i32
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Gives a nucleotide with no phosphate somewhere for its backbone to run.
    ///
    /// The first nucleotide of a strand usually has none - the 5' end is a hydroxyl - and
    /// with the trace keyed to phosphorus alone it was left out of the ribbon and drawn as
    /// loose atoms at the end of every strand. Its O5' is where the phosphate would be.
    fn trace_unphosphorylated_ends(&mut self) {
        for index in 0..self.residues.len() {
            if self.residues[index].kind != ResidueKind::Nucleotide
                || self.residues[index].trace_atom >= 0
            {
                continue;
            }
            let first = self.residues[index].first_atom as usize;
            let end = first + self.residues[index].atom_count as usize;
            for name in ["O5'", "C5'", "C4'"] {
                if let Some(atom) = (first..end).find(|&atom| self.atom_names[atom].trim() == name)
                {
                    self.residues[index].trace_atom = atom as i32;
                    break;
                }
            }
        }
    }

    /// Finds a residue by chain id and author number, for applying secondary structure
    /// records after the atoms are in place.
    pub fn residue_indices_in_range(
        &self,
        chain_id: &str,
        from: i32,
        to: i32,
    ) -> impl Iterator<Item = usize> + '_ {
        let chain_id = chain_id.trim().to_string();
        // The archive's own records name chains by author id or by label asym id
        // depending on the category, and a caller may hold either - or the composite
        // `A_1`. But the three are different namespaces that share letters: author chain
        // A of an antibody complex is not label asym A. So a name that is some chain's
        // author id means that, and only a name that is nobody's author id is tried as a
        // label - matching all three at once paints one chain's helix onto another.
        let is_author_id = self
            .chains
            .iter()
            .any(|chain| chain.auth_id.trim() == chain_id);
        // The name is matched once per chain, and each residue is then a lookup: matched
        // per residue, every helix of a capsid's file compared strings for all three
        // hundred thousand residues, and reading the assembly took five seconds on one
        // core. A chain's residues are not always one run - a ligand of another asym can
        // sit between two pieces of it - so the residues are still walked in order.
        let named: Vec<bool> = self
            .chains
            .iter()
            .map(|chain| {
                if is_author_id {
                    chain.auth_id.trim() == chain_id
                } else {
                    chain.label_asym_id.trim() == chain_id || chain.id.trim() == chain_id
                }
            })
            .collect();
        self.residues
            .iter()
            .enumerate()
            .filter(move |(_, residue)| {
                // Secondary structure is a polymer's. An author chain also holds its
                // waters and ligands, numbered wherever the depositor liked.
                let polymer = matches!(
                    residue.kind,
                    ResidueKind::AminoAcid | ResidueKind::Nucleotide
                );
                polymer
                    && residue.number >= from
                    && residue.number <= to
                    && named.get(residue.chain_index as usize) == Some(&true)
            })
            .map(|(index, _)| index)
    }

    /// Names each chain's entity type.
    ///
    /// `known` maps a label asym id to `(entity_id, entity_type)` from the file's
    /// `_struct_asym` and `_entity` loops. A chain the file does not describe - every chain
    /// of a PDB file - is classified from what it contains: all water is `water`, no
    /// polymer residue is `non-polymer`, anything else is `polymer`. The same four words the
    /// archive uses, so a PDB file and its mmCIF twin describe their chains alike.
    pub fn assign_entities(&mut self, known: &std::collections::HashMap<String, (String, String)>) {
        for index in 0..self.chains.len() {
            let label = self.chains[index].label_asym_id.clone();
            if let Some((entity, kind)) = known.get(&label) {
                let chain = &mut self.chains[index];
                if chain.entity_id.is_empty() {
                    chain.entity_id = entity.clone();
                }
                chain.entity_type = kind.clone();
                continue;
            }
            let residues = self
                .residues
                .iter()
                .filter(|residue| residue.chain_index == index as u32);
            let mut polymer = false;
            let mut other = false;
            let mut water = false;
            for residue in residues {
                match residue.kind {
                    ResidueKind::AminoAcid | ResidueKind::Nucleotide => polymer = true,
                    ResidueKind::Water => water = true,
                    ResidueKind::Ligand => other = true,
                }
            }
            self.chains[index].entity_type = if polymer {
                "polymer"
            } else if other {
                "non-polymer"
            } else if water {
                "water"
            } else {
                "non-polymer"
            }
            .to_string();
        }
    }

    /// Serialises to the binary payload the frontend decodes.
    ///
    /// Layout: magic, u32 header length, header JSON, then each block back to back with
    /// its own alignment respected (4-byte types are placed at 4-byte offsets so the
    /// frontend can create typed-array views without copying).
    pub fn encode(&self) -> Vec<u8> {
        let atom_count = self.atom_count();
        let bond_count = self.bonds.len();

        // Build blocks first so the header can carry their offsets.
        let mut body: Vec<u8> = Vec::with_capacity(atom_count * 24 + bond_count * 8);
        let mut blocks = Blocks::default();

        let mut palette = Vec::with_capacity(768);
        let mut radii = Vec::with_capacity(256);
        for element in 0u16..256 {
            let element = element as Element;
            palette.extend_from_slice(&chem::color(element));
            radii.push(chem::vdw_radius(element));
        }

        blocks.positions = push_f32(&mut body, &self.positions);
        blocks.vdw_radii = push_f32(&mut body, &radii);
        blocks.b_factors = push_f32(&mut body, &self.b_factors);
        blocks.residue_of_atom = push_u32(&mut body, &self.residue_of_atom);
        // The payload carries the bonds a stick model draws, which is every bond except
        // metal coordination - see `Bond::is_covalent`.
        let bonds_flat: Vec<u32> = self
            .bonds
            .iter()
            .filter(|bond| bond.is_covalent())
            .flat_map(|bond| bond.atoms)
            .collect();
        blocks.bonds = push_u32(&mut body, &bonds_flat);
        // Bytes last: they need no alignment, so putting them at the end keeps every
        // 4-byte block naturally aligned without padding.
        blocks.elements = push_u8(&mut body, &self.elements);
        blocks.flags = push_u8(&mut body, &self.flags);
        blocks.palette = push_u8(&mut body, &palette);

        let (bounds_min, bounds_max) = self.bounds();
        let center = [
            (bounds_min[0] + bounds_max[0]) * 0.5,
            (bounds_min[1] + bounds_max[1]) * 0.5,
            (bounds_min[2] + bounds_max[2]) * 0.5,
        ];

        let header = StructureHeader {
            version: WIRE_VERSION,
            title: self.title.clone(),
            source: self.source.clone(),
            atom_count: atom_count as u32,
            // What the bonds block holds, which leaves out metal coordination.
            bond_count: self.bonds.iter().filter(|bond| bond.is_covalent()).count() as u32,
            residues: self
                .residues
                .iter()
                .map(|residue| ResidueSummary {
                    name: residue.name.clone(),
                    code: crate::structure::chem::residue_code(&residue.name).to_string(),
                    number: residue.number,
                    chain_index: residue.chain_index,
                    first_atom: residue.first_atom,
                    atom_count: residue.atom_count,
                    secondary: residue.secondary as u8,
                    kind: residue.kind as u8,
                    trace_atom: residue.trace_atom,
                    orient_atom: residue.orient_atom,
                })
                .collect(),
            chains: self.chains.clone(),
            bounds_min,
            bounds_max,
            center,
            radius: self.bounding_radius(center),
            b_factor_min: self.b_factors.iter().copied().fold(f32::INFINITY, f32::min),
            b_factor_max: self
                .b_factors
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max),
            blocks,
        };

        let header_json = serde_json::to_vec(&header).unwrap_or_default();
        let mut payload = Vec::with_capacity(8 + header_json.len() + body.len() + 4);
        payload.extend_from_slice(WIRE_MAGIC);
        payload.extend_from_slice(&(header_json.len() as u32).to_le_bytes());
        payload.extend_from_slice(&header_json);
        // Pad so the body - and therefore every 4-byte block inside it - lands on a
        // 4-byte boundary. Without this, Float32Array views over the buffer throw.
        while payload.len() % 4 != 0 {
            payload.push(0);
        }
        payload.extend_from_slice(&body);
        payload
    }

    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        if self.is_empty() {
            return ([0.0; 3], [0.0; 3]);
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for atom in 0..self.atom_count() {
            for axis in 0..3 {
                let value = self.positions[atom * 3 + axis];
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
        (min, max)
    }

    fn bounding_radius(&self, center: [f32; 3]) -> f32 {
        let mut radius_squared = 0.0f32;
        for atom in 0..self.atom_count() {
            let mut distance = 0.0;
            for (axis, origin) in center.iter().enumerate() {
                let delta = self.positions[atom * 3 + axis] - origin;
                distance += delta * delta;
            }
            radius_squared = radius_squared.max(distance);
        }
        // The largest van der Waals radius in play, so a spacefill view is never clipped
        // by a camera framed on atom centres alone.
        radius_squared.sqrt() + 2.0
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

fn push_u8(body: &mut Vec<u8>, values: &[u8]) -> Block {
    let offset = body.len() as u32;
    body.extend_from_slice(values);
    Block {
        offset,
        length: values.len() as u32,
    }
}

const AMINO_ACIDS: &[&str] = &[
    "ALA", "ARG", "ASN", "ASP", "CYS", "GLN", "GLU", "GLY", "HIS", "ILE", "LEU", "LYS", "MET",
    "PHE", "PRO", "SER", "THR", "TRP", "TYR", "VAL", "SEC", "PYL", "MSE", "ASX", "GLX", "UNK",
];

const NUCLEOTIDES: &[&str] = &[
    "A", "C", "G", "T", "U", "DA", "DC", "DG", "DT", "DU", "RA", "RC", "RG", "RU",
];

const WATERS: &[&str] = &["HOH", "WAT", "DOD", "H2O"];

fn classify_residue(name: &str, _is_hetatm: bool) -> ResidueKind {
    let name = name.trim().to_ascii_uppercase();
    if WATERS.contains(&name.as_str()) {
        return ResidueKind::Water;
    }
    if AMINO_ACIDS.contains(&name.as_str()) {
        // MSE (selenomethionine) is a HETATM but is part of the chain; classifying by
        // name rather than record type is what keeps it in the cartoon.
        return ResidueKind::AminoAcid;
    }
    if NUCLEOTIDES.contains(&name.as_str()) {
        return ResidueKind::Nucleotide;
    }
    // Anything else is a ligand, whether or not it was written as a HETATM record. The
    // record type is unreliable in both directions - MSE is a HETATM that belongs to the
    // chain, and modified residues are sometimes written as ATOM - so the residue name is
    // what decides. A non-standard polymer residue therefore appears as a ligand rather
    // than joining the cartoon, which is visible and correctable; guessing from the record
    // type would put ions inside the ribbon instead.
    ResidueKind::Ligand
}

/// Backbone atom names for proteins and nucleic acids.
const BACKBONE_NAMES: &[&str] = &[
    "N", "CA", "C", "O", "OXT", // protein
    "P", "OP1", "OP2", "O5'", "C5'", "C4'", "O4'", "C3'", "O3'", "C2'", "C1'", // nucleic
];

fn atom_flags(name: &str, kind: ResidueKind, element: Element) -> u8 {
    let mut result = 0;
    if element == chem::HYDROGEN {
        result |= flags::HYDROGEN;
    }
    match kind {
        ResidueKind::Water => result |= flags::WATER,
        ResidueKind::Ligand => result |= flags::LIGAND,
        ResidueKind::AminoAcid | ResidueKind::Nucleotide => {
            result |= flags::POLYMER;
            let upper = name.trim().to_ascii_uppercase();
            if BACKBONE_NAMES.contains(&upper.as_str()) {
                result |= flags::BACKBONE;
            } else if element != chem::HYDROGEN {
                result |= flags::SIDECHAIN;
            }
        }
    }
    result
}

pub fn element_color(element: Element) -> [u8; 3] {
    chem::color(element)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(chain: &str, number: i32, residue: &str, name: &str, position: [f32; 3]) -> RawAtom {
        RawAtom {
            name: name.into(),
            element: chem::from_atom_name(name, false),
            position,
            b_factor: 50.0,
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
    fn atoms_group_into_residues_and_chains() {
        let atoms = vec![
            atom("A", 1, "MET", " N  ", [0.0, 0.0, 0.0]),
            atom("A", 1, "MET", " CA ", [1.5, 0.0, 0.0]),
            atom("A", 2, "ALA", " N  ", [3.0, 0.0, 0.0]),
            atom("B", 1, "GLY", " N  ", [10.0, 0.0, 0.0]),
        ];
        let structure = Structure::build("t".into(), "test".into(), atoms);

        assert_eq!(structure.atom_count(), 4);
        assert_eq!(structure.residues.len(), 3);
        assert_eq!(structure.chains.len(), 2);
        assert_eq!(structure.residues[0].atom_count, 2);
        assert_eq!(structure.chains[0].residue_count, 2);
        assert_eq!(structure.chains[1].id, "B_1");
        assert_eq!(structure.chains[1].auth_id, "B");
        assert_eq!(structure.residue_of_atom, vec![0, 0, 1, 2]);
        // Backbone N and CA are flagged as such; both are polymer atoms.
        assert_eq!(structure.flags[0] & flags::BACKBONE, flags::BACKBONE);
        assert_eq!(structure.flags[1] & flags::BACKBONE, flags::BACKBONE);
        assert_eq!(structure.flags[0] & flags::POLYMER, flags::POLYMER);
        // The cartoon builder needs the CA of each residue; MET's is atom 1.
        assert_eq!(structure.residues[0].trace_atom, 1);
        assert_eq!(
            structure.residues[1].trace_atom, -1,
            "ALA here has no CA atom"
        );
    }

    #[test]
    fn insertion_codes_stay_separate_residues() {
        // 100 and 100A are different residues; grouping on number alone merges them and
        // silently corrupts every downstream index.
        let mut second = atom("A", 100, "SER", " N  ", [1.0, 0.0, 0.0]);
        second.insertion_code = Some('A');
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![atom("A", 100, "SER", " N  ", [0.0, 0.0, 0.0]), second],
        );
        assert_eq!(structure.residues.len(), 2);
    }

    #[test]
    fn waters_and_ligands_are_classified_apart_from_the_chain() {
        assert_eq!(classify_residue("HOH", true), ResidueKind::Water);
        assert_eq!(classify_residue("ALA", false), ResidueKind::AminoAcid);
        assert_eq!(classify_residue("MSE", true), ResidueKind::AminoAcid);
        assert_eq!(classify_residue("ATP", true), ResidueKind::Ligand);
        assert_eq!(classify_residue("DG", false), ResidueKind::Nucleotide);
    }

    #[test]
    fn the_wire_payload_is_self_describing_and_aligned() {
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![
                atom("A", 1, "MET", " N  ", [0.0, 1.0, 2.0]),
                atom("A", 1, "MET", " CA ", [3.0, 4.0, 5.0]),
            ],
        );
        let payload = structure.encode();
        assert_eq!(&payload[0..4], WIRE_MAGIC);

        let header_length = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
        let header: serde_json::Value =
            serde_json::from_slice(&payload[8..8 + header_length]).unwrap();
        assert_eq!(header["atom_count"], 2);
        assert_eq!(header["version"], WIRE_VERSION);

        let mut body_start = 8 + header_length;
        body_start += (4 - body_start % 4) % 4;
        assert_eq!(body_start % 4, 0, "the body must be 4-byte aligned");

        // The palette travels with the geometry so the frontend has no colour table of
        // its own to fall out of step.
        assert_eq!(header["blocks"]["palette"]["length"], 768);
        assert_eq!(header["blocks"]["vdw_radii"]["length"], 1024);

        let positions_offset = header["blocks"]["positions"]["offset"].as_u64().unwrap() as usize;
        let start = body_start + positions_offset;
        let first = f32::from_le_bytes(payload[start..start + 4].try_into().unwrap());
        assert_eq!(first, 0.0);
        let second = f32::from_le_bytes(payload[start + 4..start + 8].try_into().unwrap());
        assert_eq!(second, 1.0);
    }

    #[test]
    fn an_empty_structure_encodes_without_panicking() {
        let structure = Structure::build("t".into(), "test".into(), Vec::new());
        assert!(structure.is_empty());
        assert_eq!(structure.bounds(), ([0.0; 3], [0.0; 3]));
        let payload = structure.encode();
        assert_eq!(&payload[0..4], WIRE_MAGIC);
        assert_eq!(payload.len() % 4, 0);
    }

    #[test]
    fn a_chain_id_that_comes_back_after_another_chain_is_the_same_chain() {
        let atoms = vec![
            atom("A", 1, "ALA", "CA", [0.0, 0.0, 0.0]),
            atom("B", 1, "GLY", "CA", [5.0, 0.0, 0.0]),
            atom("A", 2, "ALA", "CA", [3.8, 0.0, 0.0]),
        ];
        let structure = Structure::build("t".into(), "test".into(), atoms);
        assert_eq!(structure.chains.len(), 2);
        assert_eq!(structure.chains[0].residue_count, 2);
        assert_eq!(structure.residues[2].chain_index, 0);
    }

    #[test]
    fn the_label_asym_id_splits_an_author_chain_into_its_molecules() {
        // Author chain A of 4HHB is protein (asym A), haem (E) and water (K).
        let mut haem = atom("A", 142, "HEM", "FE", [0.0, 0.0, 0.0]);
        haem.label_asym_id = "E".into();
        let mut water = atom("A", 201, "HOH", "O", [5.0, 0.0, 0.0]);
        water.label_asym_id = "K".into();
        let mut protein = atom("A", 1, "VAL", "CA", [9.0, 0.0, 0.0]);
        protein.label_asym_id = "A".into();
        let mut structure = Structure::build("t".into(), "test".into(), vec![protein, haem, water]);
        structure.assign_entities(&std::collections::HashMap::new());
        let chains: Vec<(&str, &str, &str)> = structure
            .chains
            .iter()
            .map(|chain| {
                (
                    chain.id.as_str(),
                    chain.auth_id.as_str(),
                    chain.entity_type.as_str(),
                )
            })
            .collect();
        assert_eq!(
            chains,
            vec![
                ("A_1", "A", "polymer"),
                ("E_1", "A", "non-polymer"),
                ("K_1", "A", "water")
            ]
        );
    }

    #[test]
    fn secondary_structure_ranges_name_author_chains_first_and_skip_ligands() {
        let mut ligand = atom("A", 2, "HEM", "FE", [0.0, 0.0, 0.0]);
        ligand.label_asym_id = "B".into();
        let mut protein_a = atom("A", 1, "ALA", "CA", [0.0, 0.0, 0.0]);
        protein_a.label_asym_id = "A".into();
        let mut protein_a2 = atom("A", 3, "ALA", "CA", [3.8, 0.0, 0.0]);
        protein_a2.label_asym_id = "A".into();
        // Author chain B, whose label asym is C: "B" is an author id, so it means this one
        // and never the haem's asym B.
        let mut protein_b = atom("B", 2, "ALA", "CA", [9.0, 0.0, 0.0]);
        protein_b.label_asym_id = "C".into();
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![protein_a, ligand, protein_a2, protein_b],
        );

        let in_a: Vec<usize> = structure.residue_indices_in_range("A", 1, 3).collect();
        assert_eq!(
            in_a,
            vec![0, 2],
            "the ligand numbered 2 is not in the helix"
        );
        let in_b: Vec<usize> = structure.residue_indices_in_range("B", 1, 9).collect();
        assert_eq!(in_b, vec![3]);
        // A name that is nobody's author id is tried as a label.
        let in_c: Vec<usize> = structure.residue_indices_in_range("C", 1, 9).collect();
        assert_eq!(in_c, vec![3]);
        // A reversed range is empty rather than everything.
        assert_eq!(structure.residue_indices_in_range("A", 3, 1).count(), 0);
    }

    #[test]
    fn a_modified_nucleotide_inside_a_strand_joins_it() {
        // U - PSU - U, with PSU's phosphate 1.6 A from the previous O3' and its own O3'
        // 1.6 A from the next phosphate: pseudouridine is in the strand, not a ligand.
        let sugar = |number: i32, name: &str, x: f32| -> Vec<RawAtom> {
            vec![
                atom("A", number, name, "P", [x, 0.0, 0.0]),
                atom("A", number, name, "C4'", [x + 1.0, 1.0, 0.0]),
                atom("A", number, name, "O4'", [x + 1.5, 1.5, 0.0]),
                atom("A", number, name, "C3'", [x + 2.0, 0.5, 0.0]),
                atom("A", number, name, "C1'", [x + 2.5, 2.0, 0.0]),
                atom("A", number, name, "O3'", [x + 4.4, 0.0, 0.0]),
            ]
        };
        let atoms = [
            sugar(1, "U", 0.0),
            sugar(2, "PSU", 6.0),
            sugar(3, "U", 12.0),
        ]
        .concat();
        let structure = Structure::build("t".into(), "test".into(), atoms);
        assert_eq!(structure.residues[1].kind, ResidueKind::Nucleotide);
        assert_eq!(structure.residues[1].trace_atom, 6, "its phosphorus");
        assert!(structure.flags[6] & flags::POLYMER != 0);
        assert_eq!(structure.flags[6] & flags::LIGAND, 0);

        // The same PSU far from any strand is a bound nucleotide: a ligand.
        let alone = Structure::build("t".into(), "test".into(), sugar(2, "PSU", 60.0));
        assert_eq!(alone.residues[0].kind, ResidueKind::Ligand);
    }

    #[test]
    fn a_strands_first_nucleotide_without_phosphate_still_has_a_trace_atom() {
        let atoms = vec![
            atom("A", 1, "DG", "O5'", [0.0, 0.0, 0.0]),
            atom("A", 1, "DG", "C5'", [1.4, 0.0, 0.0]),
        ];
        let structure = Structure::build("t".into(), "test".into(), atoms);
        assert_eq!(structure.residues[0].trace_atom, 0);
    }

    #[test]
    fn the_payload_leaves_out_metal_coordination_and_says_so() {
        let mut structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![
                atom("A", 1, "MET", "N", [0.0, 0.0, 0.0]),
                atom("A", 1, "MET", "CA", [1.5, 0.0, 0.0]),
                atom("A", 2, "ZN", "ZN", [3.0, 0.0, 0.0]),
            ],
        );
        structure.bonds = vec![
            Bond::new(1, 0, BondOrder::Single, BondKind::Template),
            Bond::new(0, 2, BondOrder::Single, BondKind::Metal),
        ];
        let payload = structure.encode();
        let header_length = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
        let header: serde_json::Value =
            serde_json::from_slice(&payload[8..8 + header_length]).unwrap();
        assert_eq!(header["bond_count"], 1);
        assert_eq!(header["blocks"]["bonds"]["length"], 8, "one bond, two u32s");
        // Every block lies inside the payload.
        let body_start = (8 + header_length).div_ceil(4) * 4;
        for (name, block) in header["blocks"].as_object().unwrap() {
            let end = block["offset"].as_u64().unwrap() + block["length"].as_u64().unwrap();
            assert!(body_start as u64 + end <= payload.len() as u64, "{name}");
        }
    }

    #[test]
    fn bounds_include_room_for_van_der_waals_radii() {
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![
                atom("A", 1, "MET", " N  ", [-5.0, 0.0, 0.0]),
                atom("A", 2, "ALA", " N  ", [5.0, 0.0, 0.0]),
            ],
        );
        let (min, max) = structure.bounds();
        assert_eq!(min[0], -5.0);
        assert_eq!(max[0], 5.0);
        // A spacefill view of an atom at the extreme must not be clipped.
        assert!(structure.bounding_radius([0.0; 3]) > 5.0);
    }
}

#[cfg(test)]
mod flag_tests {
    use super::*;

    fn atom(name: &str, residue: &str, hetatm: bool) -> RawAtom {
        RawAtom {
            name: name.into(),
            element: chem::from_atom_name(name, !name.starts_with(' ')),
            position: [0.0, 0.0, 0.0],
            b_factor: 0.0,
            occupancy: 1.0,
            residue_name: residue.into(),
            residue_number: 1,
            insertion_code: None,
            chain_id: "A".into(),
            is_hetatm: hetatm,
            label_asym_id: String::new(),
            entity_id: String::new(),
        }
    }

    #[test]
    fn flags_separate_backbone_sidechain_water_and_ligand() {
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![
                atom(" CA ", "ALA", false),
                atom(" CB ", "ALA", false),
                atom(" O  ", "HOH", true),
                atom("MG  ", "MG", true),
                atom(" HB1", "ALA", false),
            ],
        );
        assert!(structure.flags[0] & flags::BACKBONE != 0);
        assert!(structure.flags[1] & flags::SIDECHAIN != 0);
        assert!(structure.flags[2] & flags::WATER != 0);
        assert!(structure.flags[3] & flags::LIGAND != 0);
        assert!(structure.flags[4] & flags::HYDROGEN != 0);

        // A water oxygen is not a polymer atom, so "show polymer only" excludes it.
        assert_eq!(structure.flags[2] & flags::POLYMER, 0);
        assert!(structure.flags[0] & flags::POLYMER != 0);
    }

    #[test]
    fn hydrogens_on_a_side_chain_are_not_also_side_chain_geometry() {
        // Otherwise "side chains only" would draw a cloud of hydrogens with no carbons
        // when a model includes them.
        let structure =
            Structure::build("t".into(), "test".into(), vec![atom(" HB1", "ALA", false)]);
        assert_eq!(structure.flags[0] & flags::SIDECHAIN, 0);
    }

    #[test]
    fn a_chain_label_is_taken_apart_into_asym_id_and_operator() {
        assert_eq!(assembly_copy("A"), ("A", "1", "A"));
        // One of the archive's assembly files.
        assert_eq!(assembly_copy("A-2"), ("A", "2", "A-2"));
        assert_eq!(assembly_copy("AA-60"), ("AA", "60", "AA-60"));
        // A copy made here from the assembly records, by an operator with any name.
        assert_eq!(assembly_copy("A\u{1f}2"), ("A", "2", "A"));
        assert_eq!(assembly_copy("B\u{1f}P"), ("B", "P", "B"));
        // Not a copy's number: left as the chain's own name.
        assert_eq!(assembly_copy("A-x"), ("A-x", "1", "A-x"));
        assert_eq!(assembly_copy("-2"), ("-2", "1", "-2"));
    }
}
