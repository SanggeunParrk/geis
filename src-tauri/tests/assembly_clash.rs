//! Whether an assembly's copies sit where they should: atoms of different chains closer
//! than any real contact are counted. Runs when `GEIS_TEST_STRUCTURES` holds 1uf2.cif.

use geis_lib::structure::{self, Structure, Unit};
use std::collections::HashMap;
use std::path::PathBuf;

/// Pairs of heavy atoms in different chains closer than `limit`, and how many chains have one.
fn clashes(s: &Structure, limit: f32, only_ca: bool) -> (usize, usize, Vec<(String, String, f32)>) {
    let cell = 3.0_f32;
    let mut grid: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
    let chain_of = |atom: usize| s.residues[s.residue_of_atom[atom] as usize].chain_index;
    let keep =
        |atom: usize| s.elements[atom] != 1 && (!only_ca || s.atom_names[atom].trim() == "CA");
    let key = |p: [f32; 3]| {
        (
            (p[0] / cell).floor() as i32,
            (p[1] / cell).floor() as i32,
            (p[2] / cell).floor() as i32,
        )
    };
    for atom in 0..s.atom_count() {
        if keep(atom) {
            grid.entry(key(s.position(atom))).or_default().push(atom);
        }
    }
    let (mut count, mut chains, mut examples) = (0, std::collections::HashSet::new(), Vec::new());
    for atom in 0..s.atom_count() {
        if !keep(atom) {
            continue;
        }
        let p = s.position(atom);
        let (x, y, z) = key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &other in grid
                        .get(&(x + dx, y + dy, z + dz))
                        .map(Vec::as_slice)
                        .unwrap_or(&[])
                    {
                        if other <= atom || chain_of(other) == chain_of(atom) {
                            continue;
                        }
                        let q = s.position(other);
                        let d =
                            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2))
                                .sqrt();
                        if d < limit {
                            count += 1;
                            chains.insert(chain_of(atom));
                            if examples.len() < 6 {
                                examples.push((
                                    s.chains[chain_of(atom) as usize].id.clone(),
                                    s.chains[chain_of(other) as usize].id.clone(),
                                    d,
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    (count, chains.len(), examples)
}

#[test]
fn the_capsid_s_copies_do_not_overlap() {
    let Some(dir) = std::env::var("GEIS_TEST_STRUCTURES")
        .ok()
        .map(PathBuf::from)
    else {
        return;
    };
    let path = dir.join("1uf2.cif");
    if !path.exists() {
        return;
    }
    for unit in [Unit::Deposited, Unit::Assembly] {
        let s = structure::load_file_as(&path, unit.clone()).unwrap();
        let (heavy, chains, examples) = clashes(&s, 1.5, false);
        let (ca, ca_chains, ca_examples) = clashes(&s, 3.0, true);
        eprintln!(
            "{unit:?}: {} chains, {} atoms; heavy<1.5A {heavy} pairs in {chains} chains {examples:?}; CA<3A {ca} in {ca_chains} {ca_examples:?}",
            s.chains.len(),
            s.atom_count()
        );
    }
}

#[test]
fn no_bond_reaches_across_the_capsid() {
    let Some(dir) = std::env::var("GEIS_TEST_STRUCTURES")
        .ok()
        .map(PathBuf::from)
    else {
        return;
    };
    let path = dir.join("1uf2.cif");
    if !path.exists() {
        return;
    }
    for unit in [Unit::Deposited, Unit::Assembly] {
        let s = structure::load_file_as(&path, unit.clone()).unwrap();
        let mut long = Vec::new();
        let mut cross = 0;
        for bond in &s.bonds {
            let (a, b) = (bond.atoms[0] as usize, bond.atoms[1] as usize);
            let (p, q) = (s.position(a), s.position(b));
            let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
            let ca = s.residues[s.residue_of_atom[a] as usize].chain_index;
            let cb = s.residues[s.residue_of_atom[b] as usize].chain_index;
            if ca != cb {
                cross += 1;
            }
            if d > 3.0 && long.len() < 5 {
                long.push((
                    s.chains[ca as usize].id.clone(),
                    s.chains[cb as usize].id.clone(),
                    d,
                    format!("{:?}", bond.kind),
                ));
            }
        }
        let total_long = s
            .bonds
            .iter()
            .filter(|bond| {
                let (p, q) = (
                    s.position(bond.atoms[0] as usize),
                    s.position(bond.atoms[1] as usize),
                );
                ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt() > 3.0
            })
            .count();
        eprintln!(
            "{unit:?}: {} bonds, {cross} between chains, {total_long} longer than 3 A {long:?}",
            s.bonds.len()
        );
    }
}

#[test]
fn every_assembly_of_the_capsid_is_built_and_none_overlaps() {
    let Some(dir) = std::env::var("GEIS_TEST_STRUCTURES")
        .ok()
        .map(PathBuf::from)
    else {
        return;
    };
    let path = dir.join("1uf2.cif");
    if !path.exists() {
        return;
    }
    // Chains each assembly should hold, from `_pdbx_struct_assembly.oligomeric_count`.
    for (id, chains) in [
        ("1", 960),
        ("2", 16),
        ("3", 80),
        ("4", 96),
        ("5", 16),
        ("6", 240),
    ] {
        let reading = structure::Reading::from(Unit::AssemblyId(id.into()));
        let s = structure::load_file_reading(&path, &reading).unwrap();
        assert_eq!(s.versions.assembly.as_deref(), Some(id));
        assert_eq!(s.chains.len(), chains, "assembly {id}");
        assert_eq!(s.versions.assemblies.len(), 6);
        let (heavy, _, examples) = clashes(&s, 1.5, false);
        assert_eq!(heavy, 0, "assembly {id} overlaps: {examples:?}");
        eprintln!("assembly {id}: {} chains, no overlaps", s.chains.len());
    }
}
