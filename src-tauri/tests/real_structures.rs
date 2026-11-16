//! Parser checks against real archive files.
//!
//! The synthetic fixtures in the unit tests cover the format rules; these cover what the
//! archive actually contains - alternate conformers, ligands, waters, insertion codes,
//! multi-chain assemblies - and cross-check the two readers against each other.
//!
//! The files are not in the repository. Point `GEIS_TEST_STRUCTURES` at a directory
//! holding 1ubq.pdb, 1ubq.cif, 6vxx.pdb and 6vxx.cif and these run; otherwise they skip,
//! so `cargo test` needs no network.

use geis_lib::structure::{self, model::flags};
use std::path::PathBuf;

fn fixtures() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var("GEIS_TEST_STRUCTURES").ok()?);
    dir.is_dir().then_some(dir)
}

macro_rules! fixtures_or_skip {
    () => {
        match fixtures() {
            Some(dir) => dir,
            None => {
                eprintln!("skipping: GEIS_TEST_STRUCTURES is not set");
                return;
            }
        }
    };
}

#[test]
fn the_two_readers_agree_on_the_same_entry() {
    let dir = fixtures_or_skip!();

    for id in ["1ubq", "6vxx"] {
        let pdb = structure::load_file(&dir.join(format!("{id}.pdb"))).expect("pdb loads");
        let cif = structure::load_file(&dir.join(format!("{id}.cif"))).expect("cif loads");

        // The same deposition read two ways must contain the same atoms. A mismatch here
        // means one reader is dropping records the other keeps - the kind of bug that
        // shows up much later as a missing loop or a hole in a surface.
        assert_eq!(
            pdb.atom_count(),
            cif.atom_count(),
            "{id}: pdb has {} atoms, cif has {}",
            pdb.atom_count(),
            cif.atom_count()
        );
        assert_eq!(
            pdb.residues.len(),
            cif.residues.len(),
            "{id}: residue count"
        );
        // Chains are the other place the formats genuinely differ. A PDB file has author
        // chains; an mmCIF has label asyms, which give a haem, a glycan or a run of waters
        // an identity of its own beside the polymer they were deposited with - so the CIF
        // has at least as many chains, and the *polymer* chains agree by author id.
        assert!(
            cif.chains.len() >= pdb.chains.len(),
            "{id}: the cif should know at least as many chains: {} vs {}",
            cif.chains.len(),
            pdb.chains.len(),
        );
        let polymer_auth_ids = |structure: &geis_lib::structure::Structure| {
            let mut ids: Vec<String> = structure
                .chains
                .iter()
                .filter(|chain| chain.entity_type == "polymer")
                .map(|chain| chain.auth_id.clone())
                .collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        assert_eq!(
            polymer_auth_ids(&pdb),
            polymer_auth_ids(&cif),
            "{id}: the polymer chains, by author id"
        );
        for chain in &cif.chains {
            assert!(
                chain.id.ends_with("_1"),
                "{id}: asymmetric unit chains carry operator 1: {}",
                chain.id
            );
            assert!(
                !chain.entity_type.is_empty(),
                "{id}: every chain has an entity type: {}",
                chain.id
            );
        }
        // Bonds are where the two formats genuinely differ, and it is not a bug. An mmCIF
        // carries its own component dictionary and its `_struct_conn` records, so it knows
        // the disulfides, the glycan links and the ligands' own bonds; a PDB file carries
        // none of that, so those residues fall back to the built-in templates and to
        // distance. The CIF must therefore know at least as much, and every bond the PDB
        // found must be one the CIF found too.
        assert!(
            cif.bonds.len() >= pdb.bonds.len(),
            "{id}: the cif should know at least as much: {} vs {}",
            cif.bonds.len(),
            pdb.bonds.len(),
        );
        let cif_pairs: std::collections::HashSet<[u32; 2]> =
            cif.bonds.iter().map(|bond| bond.atoms).collect();
        let missing = pdb
            .bonds
            .iter()
            .filter(|bond| !cif_pairs.contains(&bond.atoms))
            .count();
        // A handful of distance-inferred bonds in an untemplated component can differ,
        // but the overwhelming majority must agree.
        assert!(
            missing * 200 < pdb.bonds.len(),
            "{id}: {missing} of {} bonds the pdb found are missing from the cif",
            pdb.bonds.len(),
        );
        // And the extra ones the cif knows come from records the pdb has no equivalent of.
        if cif.bonds.len() > pdb.bonds.len() {
            let from_records = cif
                .bonds
                .iter()
                .filter(|bond| {
                    matches!(
                        bond.kind,
                        structure::BondKind::Disulfide
                            | structure::BondKind::Covalent
                            | structure::BondKind::Metal
                    )
                })
                .count();
            assert!(
                from_records > 0,
                "{id}: the cif found more bonds but none from _struct_conn",
            );
        }

        // Coordinates must be identical to the precision the formats store.
        for atom in 0..pdb.atom_count() {
            let a = pdb.position(atom);
            let b = cif.position(atom);
            for axis in 0..3 {
                assert!(
                    (a[axis] - b[axis]).abs() < 0.002,
                    "{id}: atom {atom} axis {axis}: {} vs {}",
                    a[axis],
                    b[axis]
                );
            }
        }
    }
}

#[test]
fn ubiquitin_looks_like_ubiquitin() {
    let dir = fixtures_or_skip!();
    let structure = structure::load_file(&dir.join("1ubq.pdb")).expect("1ubq loads");

    // 1UBQ is 76 residues of protein plus waters, one chain.
    let protein_residues = structure
        .residues
        .iter()
        .filter(|residue| residue.kind == structure::model::ResidueKind::AminoAcid)
        .count();
    assert_eq!(protein_residues, 76);

    let waters = structure
        .residues
        .iter()
        .filter(|residue| residue.kind == structure::model::ResidueKind::Water)
        .count();
    assert!(waters > 50, "expected the ordered waters, found {waters}");

    // Every protein residue has a CA, and every CA is flagged as backbone.
    let backbone = structure
        .flags
        .iter()
        .filter(|flag| *flag & flags::BACKBONE != 0)
        .count();
    assert!(
        backbone >= protein_residues * 4,
        "expected at least N, CA, C, O per residue, found {backbone}"
    );

    // The chain must be bonded end to end: 76 residues joined by peptide bonds means the
    // backbone is one connected component. A parser that mangled residue order or a bond
    // cutoff that was too tight would leave it in pieces.
    let peptide_bonds = count_inter_residue_bonds(&structure);
    assert!(
        peptide_bonds >= 75,
        "expected 75 peptide bonds, found {peptide_bonds}"
    );
}

#[test]
fn ubiquitins_secondary_structure_is_found_in_both_formats() {
    let dir = fixtures_or_skip!();
    use geis_lib::structure::model::SecondaryStructure;

    for name in ["1ubq.pdb", "1ubq.cif"] {
        let structure = structure::load_file(&dir.join(name)).expect("loads");
        let helix = structure
            .residues
            .iter()
            .filter(|residue| residue.secondary == SecondaryStructure::Helix)
            .count();
        let sheet = structure
            .residues
            .iter()
            .filter(|residue| residue.secondary == SecondaryStructure::Sheet)
            .count();

        // 1UBQ is the beta-grasp fold: one long helix, a short one, and five strands
        // covering about 30 residues between them.
        assert!(helix >= 12, "{name}: only {helix} helix residues");
        assert!(sheet >= 25, "{name}: only {sheet} sheet residues");
    }
}

#[test]
fn a_large_assembly_parses_in_reasonable_time() {
    let dir = fixtures_or_skip!();
    // 6VXX is the SARS-CoV-2 spike trimer: three chains, glycans, ~23k atoms.
    let started = std::time::Instant::now();
    let structure = structure::load_file(&dir.join("6vxx.cif")).expect("6vxx loads");
    let elapsed = started.elapsed();

    assert!(
        structure.atom_count() > 20000,
        "{} atoms",
        structure.atom_count()
    );

    // 6VXX is a trimer. The PDB file gives every glycan its own author chain (18 in all),
    // and the mmCIF gives every asym its own - glycans, waters, the lot - so the count
    // depends on the format. What does not depend on it: three polymer chains with
    // thousands of atoms each, and "3" alone would mean the glycans had been folded in.
    assert!(
        structure.chains.len() >= 18,
        "at least the author chains: {}",
        structure.chains.len()
    );
    assert_eq!(
        structure
            .chains
            .iter()
            .filter(|chain| chain.entity_type == "polymer")
            .count(),
        3,
        "the trimer's polymer chains"
    );
    let mut atoms_per_chain = vec![0usize; structure.chains.len()];
    for residue in &structure.residues {
        atoms_per_chain[residue.chain_index as usize] += residue.atom_count as usize;
    }
    let mut large: Vec<usize> = atoms_per_chain
        .iter()
        .copied()
        .filter(|count| *count > 5000)
        .collect();
    large.sort_unstable();
    assert_eq!(
        large.len(),
        3,
        "expected three protein chains, got {atoms_per_chain:?}"
    );
    assert!(large.iter().all(|count| *count > 7000));

    // The three protein chains are author A, B and C, each exactly once - as asyms A_1,
    // B_1 and C_1, since the polymer chains keep their author letters as labels.
    let protein_ids: Vec<&str> = structure
        .chains
        .iter()
        .zip(&atoms_per_chain)
        .filter(|(_, count)| **count > 5000)
        .map(|(chain, _)| chain.auth_id.as_str())
        .collect();
    assert_eq!(protein_ids, vec!["A", "B", "C"]);
    let asym_ids: Vec<&str> = structure
        .chains
        .iter()
        .zip(&atoms_per_chain)
        .filter(|(_, count)| **count > 5000)
        .map(|(chain, _)| chain.id.as_str())
        .collect();
    assert_eq!(asym_ids, vec!["A_1", "B_1", "C_1"]);
    assert!(
        structure.bonds.len() > structure.atom_count(),
        "a protein has more bonds than atoms once rings are counted"
    );
    assert!(
        elapsed.as_millis() < 3000,
        "parsing took {elapsed:?}, which would be a visible stall"
    );

    // Glycans are ligands hanging off the protein, and they must not have been folded
    // into the chains.
    let ligands = structure
        .residues
        .iter()
        .filter(|residue| residue.kind == structure::model::ResidueKind::Ligand)
        .count();
    assert!(
        ligands > 10,
        "expected the glycan tree, found {ligands} ligands"
    );
}

#[test]
fn the_wire_payload_of_a_real_structure_round_trips() {
    let dir = fixtures_or_skip!();
    let structure = structure::load_file(&dir.join("1ubq.cif")).expect("1ubq loads");
    let payload = structure.encode();

    let header_length = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
    let header: serde_json::Value = serde_json::from_slice(&payload[8..8 + header_length]).unwrap();
    assert_eq!(header["atom_count"], structure.atom_count() as u64);

    let mut body = 8 + header_length;
    body += (4 - body % 4) % 4;

    // Spot-check the last atom, the position most likely to be off by one block.
    let offset = header["blocks"]["positions"]["offset"].as_u64().unwrap() as usize;
    let last = structure.atom_count() - 1;
    let start = body + offset + last * 12;
    let x = f32::from_le_bytes(payload[start..start + 4].try_into().unwrap());
    assert_eq!(x, structure.position(last)[0]);

    // B-factors of a crystal structure are real B-factors, not pLDDT, and must survive.
    let b_offset = header["blocks"]["b_factors"]["offset"].as_u64().unwrap() as usize;
    let b_start = body + b_offset + last * 4;
    let b = f32::from_le_bytes(payload[b_start..b_start + 4].try_into().unwrap());
    assert_eq!(b, structure.b_factors[last]);
}

/// 4HHB: haemoglobin, four chains, four haems, iron coordinated by histidine.
///
/// The entry the bonding rewrite was aimed at, and it exercises every source: the
/// dictionary for the standard residues, the file's own loop for HEM, the polymer links
/// along four chains, and `_struct_conn` for the four metal coordinations.
#[test]
fn haemoglobins_bonds_come_from_the_file_not_from_distance() {
    let dir = fixtures_or_skip!();
    let path = dir.join("4hhb.cif");
    if !path.exists() {
        eprintln!("skipping: no 4hhb.cif in the fixtures");
        return;
    }
    let structure = structure::load_file(&path).expect("4hhb loads");

    let count = |kind: structure::BondKind| {
        structure
            .bonds
            .iter()
            .filter(|bond| bond.kind == kind)
            .count()
    };
    let templated = count(structure::BondKind::Template);
    let links = count(structure::BondKind::Link);
    let metal = count(structure::BondKind::Metal);
    let inferred = count(structure::BondKind::Inferred);

    // Almost everything is a template bond: 574 residues of protein.
    assert!(templated > 4000, "template bonds: {templated}");
    // 141 + 146 residues per alpha/beta pair, so 574 residues in four chains means 570
    // peptide bonds if nothing is broken.
    assert!(
        (560..=574).contains(&links),
        "peptide links across four chains: {links}",
    );
    // One haem iron per chain, each held by a histidine NE2 - stated by `_struct_conn`,
    // and no distance rule would call a 2.1 A Fe-N contact a covalent bond.
    assert_eq!(metal, 4, "four haem coordinations");
    // Waters have no template and no bonds to find; nothing else in this entry should
    // need guessing, because the file carries HEM's own dictionary.
    assert_eq!(
        inferred, 0,
        "nothing had to be guessed: {inferred} inferred"
    );

    // Bond orders come from the dictionary: every carbonyl C=O is a double bond, and the
    // haem's porphyrin is aromatic.
    let doubles = structure
        .bonds
        .iter()
        .filter(|bond| bond.order == structure::BondOrder::Double)
        .count();
    assert!(doubles > 500, "carbonyls and more: {doubles} double bonds");
    let aromatic = structure
        .bonds
        .iter()
        .filter(|bond| bond.order == structure::BondOrder::Aromatic)
        .count();
    assert!(
        aromatic > 100,
        "phenylalanines, histidines, haem: {aromatic}"
    );

    // The residue view has the four histidine-to-haem edges. Not *cross-chain* edges:
    // chains here are author chains, and the haem of chain A is labelled chain A too -
    // only the mmCIF's own label_asym_id makes it a separate entity.
    let his_to_hem = structure
        .residue_links
        .iter()
        .filter(|link| {
            let names = [
                structure.residues[link.residues[0] as usize].name.trim(),
                structure.residues[link.residues[1] as usize].name.trim(),
            ];
            names.contains(&"HEM") && names.contains(&"HIS")
        })
        .count();
    assert_eq!(his_to_hem, 4, "one haem held by one histidine per chain");
}

fn count_inter_residue_bonds(structure: &geis_lib::structure::Structure) -> usize {
    structure
        .bonds
        .iter()
        .filter(|bond| {
            structure.residue_of_atom[bond.atoms[0] as usize]
                != structure.residue_of_atom[bond.atoms[1] as usize]
        })
        .count()
}

#[test]
fn ubiquitin_s_core_is_its_buried_hydrophobic_residues() {
    let dir = fixtures_or_skip!();
    let structure = structure::load_file(&dir.join("1ubq.pdb")).expect("1ubq reads");
    let core: Vec<i32> = structure::contacts::hydrophobic_core(&structure)
        .into_iter()
        .map(|index| structure.residues[index as usize].number)
        .collect();
    // The residues every description of ubiquitin's core names, Leu43 among them - the
    // one a neighbour count with a fixed threshold left out.
    for number in [3, 5, 13, 15, 17, 23, 26, 30, 43, 50, 56, 61, 67] {
        assert!(
            core.contains(&number),
            "residue {number} is buried and hydrophobic: {core:?}"
        );
    }
    // And the hydrophobic patch on the surface is not core: Leu8, Leu73.
    for number in [8, 73] {
        assert!(
            !core.contains(&number),
            "residue {number} is on the surface: {core:?}"
        );
    }
    let exposure = structure::contacts::relative_accessibility(&structure);
    let of = |number: i32| {
        let index = structure
            .residues
            .iter()
            .position(|residue| residue.number == number)
            .unwrap();
        exposure[index].unwrap()
    };
    eprintln!(
        "RSA: L43 {:.2}, I44 {:.2}, L8 {:.2}, L73 {:.2}, I3 {:.2}",
        of(43),
        of(44),
        of(8),
        of(73),
        of(3)
    );
}
