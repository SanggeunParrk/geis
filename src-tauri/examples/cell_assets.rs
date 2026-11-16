//! Makes the molecules the opening screens' cell is filled with.
//!
//!     cargo run --release --example cell_assets -- <folder of .cif files> <output.json>
//!
//! Each structure is read as its biological assembly and reduced to one bead per residue -
//! a residue's heavy atoms' centre, as big as they are - which keeps a protein's real
//! outline (the barrel of GFP, the Y of an antibody, the cage of GroEL) at a cost the
//! window can afford for hundreds of copies. The coordinates are PDB entries, which the
//! wwPDB releases under CC0.
//!
//! The small molecules - ATP, NAD+, haem and the rest - are one bead per heavy atom, from
//! the Chemical Component Dictionary's ideal coordinates (`<ID>_ideal.sdf` in the same
//! folder, from files.rcsb.org/ligands/download/, also CC0). Their beads are grouped by
//! element instead of by chain - carbon, oxygen, nitrogen, phosphorus and sulfur, metal -
//! so they are coloured as chemistry is.

use geis_lib::structure::{
    self,
    model::{ResidueKind, SecondaryStructure},
    Unit,
};
use std::fmt::Write as _;
use std::path::PathBuf;

/// The molecules, what they are, and how coarsely each is beaded: one bead every `step`
/// residues, so the largest are not most of the scene.
const MOLECULES: &[(&str, &str, usize)] = &[
    ("1ubq", "ubiquitin", 1),
    ("1lyz", "lysozyme", 1),
    ("1ema", "green fluorescent protein", 1),
    ("4hhb", "haemoglobin", 1),
    ("1igt", "antibody", 1),
    ("3hhr", "growth hormone and its receptor", 1),
    ("1aon", "GroEL-GroES chaperonin", 2),
    ("6vxx", "SARS-CoV-2 spike", 2),
    ("1ehz", "transfer RNA", 1),
    ("1j6z", "actin", 1),
    ("1kx5", "nucleosome", 1),
    ("4ybb", "70S ribosome", 3),
];

/// The small molecules: component id, and what it is.
const SMALL: &[(&str, &str)] = &[
    ("ATP", "ATP"),
    ("ADP", "ADP"),
    ("NAD", "NAD+"),
    ("HEM", "haem"),
    ("GLC", "glucose"),
    ("SAM", "S-adenosylmethionine"),
];

/// A small molecule's heavy atoms from a V2000 SDF: positions, radii and element groups
/// (0 carbon, 1 oxygen, 2 nitrogen, 3 phosphorus or sulfur, 4 metal).
fn small_molecule(text: &str) -> Vec<([f32; 3], f32, u32)> {
    let lines: Vec<&str> = text.lines().collect();
    let count: usize = lines[3][..3].trim().parse().expect("an atom count");
    lines[4..4 + count]
        .iter()
        .filter_map(|line| {
            let field = |at: usize| {
                line[at..at + 10]
                    .trim()
                    .parse::<f32>()
                    .expect("a coordinate")
            };
            let element = line[31..34].trim();
            let (radius, group) = match element {
                "H" => return None,
                "C" => (1.7, 0),
                "O" => (1.52, 1),
                "N" => (1.55, 2),
                "P" | "S" => (1.8, 3),
                _ => (1.4, 4),
            };
            Some(([field(0), field(10), field(20)], radius, group))
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let folder = PathBuf::from(args.get(1).expect("a folder of .cif files"));
    let output = PathBuf::from(args.get(2).expect("an output .json path"));
    let mut json =
        String::from("{\"source\":\"wwPDB entries (CC0), one bead per residue\",\"molecules\":[");
    for (index, (id, name, step)) in MOLECULES.iter().enumerate() {
        let path = folder.join(format!("{id}.cif"));
        let s = structure::load_file_as(&path, Unit::Assembly)
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        // Beads: one per residue (or per `step`), skipping water and lone ions.
        let mut beads: Vec<([f32; 3], f32, u32, bool)> = Vec::new();
        let mut chain_of_bead = Vec::new();
        let mut taken = 0usize;
        for residue in &s.residues {
            if matches!(residue.kind, ResidueKind::Water) {
                continue;
            }
            let first = residue.first_atom as usize;
            let atoms: Vec<usize> = (first..first + residue.atom_count as usize)
                .filter(|&atom| s.elements[atom] != 1)
                .collect();
            if atoms.len() < 2 {
                continue;
            }
            taken += 1;
            if taken % step != 0 {
                continue;
            }
            let mut centre = [0.0f32; 3];
            for &atom in &atoms {
                let p = s.position(atom);
                for axis in 0..3 {
                    centre[axis] += p[axis] / atoms.len() as f32;
                }
            }
            // As big as the atoms it stands for: a residue of n heavy atoms fills about the
            // volume of n spheres of 1.7 A, and beads every `step` residues are grown to
            // fill the gaps the skipped ones leave.
            let radius = 1.7 * (atoms.len() as f32 * *step as f32).cbrt() * 1.05;
            let nucleic = matches!(residue.kind, ResidueKind::Nucleotide);
            beads.push((centre, radius, residue.chain_index, nucleic));
            chain_of_bead.push(residue.chain_index);
        }
        // Centred, so a copy turns about its own middle.
        let mut middle = [0.0f32; 3];
        for (p, ..) in &beads {
            for axis in 0..3 {
                middle[axis] += p[axis] / beads.len() as f32;
            }
        }
        let extent = beads
            .iter()
            .map(|(p, r, ..)| {
                ((p[0] - middle[0]).powi(2)
                    + (p[1] - middle[1]).powi(2)
                    + (p[2] - middle[2]).powi(2))
                .sqrt()
                    + r
            })
            .fold(0.0f32, f32::max);
        // Chains renumbered from 0 in the order they appear, for colouring.
        let mut chains: Vec<u32> = Vec::new();
        let mut chain_nucleic: Vec<bool> = Vec::new();
        let mut coords = String::new();
        let mut chain_list = String::new();
        let mut nucleic_any = false;
        for (bead, (p, r, chain, nucleic)) in beads.iter().enumerate() {
            let slot = match chains.iter().position(|c| c == chain) {
                Some(slot) => slot,
                None => {
                    chains.push(*chain);
                    chain_nucleic.push(false);
                    chains.len() - 1
                }
            };
            chain_nucleic[slot] |= *nucleic;
            nucleic_any |= *nucleic;
            if bead > 0 {
                coords.push(',');
                chain_list.push(',');
            }
            let _ = write!(
                coords,
                "{:.1},{:.1},{:.1},{:.2}",
                p[0] - middle[0],
                p[1] - middle[1],
                p[2] - middle[2],
                r
            );
            let _ = write!(chain_list, "{slot}");
        }
        // The backbone, for the glass view's ribbons: every polymer residue's trace atom
        // (alpha carbon, or phosphorus), centred as the beads are, with its secondary
        // structure (H helix, E strand, C anything else, N nucleic) and its chain slot.
        let mut trace = String::new();
        let mut ss = String::new();
        let mut trace_chain = String::new();
        for residue in &s.residues {
            let nucleic = matches!(residue.kind, ResidueKind::Nucleotide);
            if !(nucleic || matches!(residue.kind, ResidueKind::AminoAcid))
                || residue.trace_atom < 0
            {
                continue;
            }
            let slot = match chains.iter().position(|c| *c == residue.chain_index) {
                Some(slot) => slot,
                None => {
                    chains.push(residue.chain_index);
                    chain_nucleic.push(nucleic);
                    chains.len() - 1
                }
            };
            let p = s.position(residue.trace_atom as usize);
            if !ss.is_empty() {
                trace.push(',');
                trace_chain.push(',');
            }
            let _ = write!(
                trace,
                "{:.1},{:.1},{:.1}",
                p[0] - middle[0],
                p[1] - middle[1],
                p[2] - middle[2]
            );
            let _ = write!(trace_chain, "{slot}");
            ss.push(if nucleic {
                'N'
            } else {
                match residue.secondary {
                    SecondaryStructure::Helix => 'H',
                    SecondaryStructure::Sheet => 'E',
                    _ => 'C',
                }
            });
        }
        if index > 0 {
            json.push(',');
        }
        let _ = write!(
            json,
            "{{\"id\":\"{id}\",\"name\":\"{name}\",\"nucleic\":{nucleic_any},\"radius\":{extent:.1},\"chains\":{},\"nucleic_chains\":[{}],\"beads\":[{coords}],\"chain\":[{chain_list}],\"trace\":[{trace}],\"ss\":\"{ss}\",\"trace_chain\":[{trace_chain}]}}",
            chains.len(),
            chain_nucleic
                .iter()
                .map(|n| if *n { "1" } else { "0" })
                .collect::<Vec<_>>()
                .join(",")
        );
        eprintln!(
            "{id}: {} beads, {} chains, radius {extent:.0} A",
            beads.len(),
            chains.len()
        );
    }
    for (id, name) in SMALL {
        let path = folder.join(format!("{id}_ideal.sdf"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{id}: {error}"));
        let atoms = small_molecule(&text);
        let mut middle = [0.0f32; 3];
        for (p, ..) in &atoms {
            for axis in 0..3 {
                middle[axis] += p[axis] / atoms.len() as f32;
            }
        }
        let extent = atoms
            .iter()
            .map(|(p, r, _)| {
                ((p[0] - middle[0]).powi(2)
                    + (p[1] - middle[1]).powi(2)
                    + (p[2] - middle[2]).powi(2))
                .sqrt()
                    + r
            })
            .fold(0.0f32, f32::max);
        let coords: Vec<String> = atoms
            .iter()
            .map(|(p, r, _)| {
                format!(
                    "{:.2},{:.2},{:.2},{:.2}",
                    p[0] - middle[0],
                    p[1] - middle[1],
                    p[2] - middle[2],
                    r
                )
            })
            .collect();
        let groups: Vec<String> = atoms.iter().map(|(.., group)| group.to_string()).collect();
        let _ = write!(
            json,
            ",{{\"id\":\"{}\",\"name\":\"{name}\",\"small\":true,\"nucleic\":false,\"radius\":{extent:.1},\"chains\":5,\"beads\":[{}],\"chain\":[{}]}}",
            id.to_lowercase(),
            coords.join(","),
            groups.join(",")
        );
        eprintln!("{id}: {} atoms, radius {extent:.0} A", atoms.len());
    }
    json.push_str("]}\n");
    std::fs::write(&output, json).expect("writes");
}
