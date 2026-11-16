//! The structure as three levels and the table between them - BioMol's model.
//!
//! BioMol (`vendor/BioMol/src/biomol/core/{index,view}.py`) keeps a structure as three
//! feature containers - atoms, residues, chains - and an `IndexTable` that maps between
//! them: a forward parent array for each level (`atom_to_res`, `res_to_chain`) and a CSR
//! reverse map for each (`res_atom_indptr/indices`, `chain_res_indptr/indices`). A `View`
//! is then nothing but a level and an array of indices, and moving between levels is a
//! table lookup. This module is that model in Rust, built from `Structure` on demand, so
//! that a selection made in the BioMol panel, the sequence strip or the viewport is one
//! kind of thing - a view - and the outline, the card and the tree all derive from it.
//!
//! The same shape on both sides is deliberate: a view serialised here is a view BioMol can
//! read, and the CIF feature names (`chain_id`, `auth_asym_id`, `auth_idx`, `chem_comp_id`,
//! `hetero`, `element`) are BioMol's, so a selection spec written for one is valid for
//! the other. BioMol is MIT and this project's author is one of its authors.

use serde::{Deserialize, Serialize};

use super::model::{ResidueKind, SecondaryStructure, Structure};

/// The three structural levels, as BioMol names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Chain,
    Residue,
    Atom,
}

/// Forward parent maps and reverse CSR maps between atoms, residues and chains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexTable {
    /// Each atom's residue.
    pub atom_to_res: Vec<u32>,
    /// Each residue's chain.
    pub res_to_chain: Vec<u32>,
    /// CSR: residue `r`'s atoms are `res_atom_indices[res_atom_indptr[r]..res_atom_indptr[r+1]]`.
    pub res_atom_indptr: Vec<u32>,
    pub res_atom_indices: Vec<u32>,
    /// CSR: chain `c`'s residues, the same way. Not a range, since depositions interleave
    /// a chain's ligands and waters with other chains - which is the reason the table exists.
    pub chain_res_indptr: Vec<u32>,
    pub chain_res_indices: Vec<u32>,
}

/// Builds a CSR reverse map from a parent array: which children each parent has, in
/// child order.
fn build_csr(parent_of_child: &[u32], n_parent: usize) -> (Vec<u32>, Vec<u32>) {
    let mut counts = vec![0u32; n_parent];
    for parent in parent_of_child {
        if let Some(count) = counts.get_mut(*parent as usize) {
            *count += 1;
        }
    }
    let mut indptr = vec![0u32; n_parent + 1];
    for (index, count) in counts.iter().enumerate() {
        indptr[index + 1] = indptr[index] + count;
    }
    let mut offsets: Vec<u32> = indptr[..n_parent].to_vec();
    let mut indices = vec![0u32; parent_of_child.len()];
    for (child, parent) in parent_of_child.iter().enumerate() {
        if let Some(offset) = offsets.get_mut(*parent as usize) {
            indices[*offset as usize] = child as u32;
            *offset += 1;
        }
    }
    (indptr, indices)
}

impl IndexTable {
    /// BioMol's `IndexTable.from_parents`.
    pub fn from_parents(atom_to_res: Vec<u32>, res_to_chain: Vec<u32>, n_chain: usize) -> Self {
        let (res_atom_indptr, res_atom_indices) = build_csr(&atom_to_res, res_to_chain.len());
        let (chain_res_indptr, chain_res_indices) = build_csr(&res_to_chain, n_chain);
        Self {
            atom_to_res,
            res_to_chain,
            res_atom_indptr,
            res_atom_indices,
            chain_res_indptr,
            chain_res_indices,
        }
    }

    pub fn from_structure(structure: &Structure) -> Self {
        Self::from_parents(
            structure.residue_of_atom.clone(),
            structure
                .residues
                .iter()
                .map(|residue| residue.chain_index)
                .collect(),
            structure.chains.len(),
        )
    }

    pub fn atom_count(&self) -> usize {
        self.atom_to_res.len()
    }
    pub fn residue_count(&self) -> usize {
        self.res_to_chain.len()
    }
    pub fn chain_count(&self) -> usize {
        self.chain_res_indptr.len().saturating_sub(1)
    }

    fn children(indptr: &[u32], indices: &[u32], parents: &[u32]) -> Vec<u32> {
        let mut out = Vec::new();
        for parent in parents {
            let p = *parent as usize;
            if p + 1 < indptr.len() {
                out.extend_from_slice(&indices[indptr[p] as usize..indptr[p + 1] as usize]);
            }
        }
        out
    }

    /// Converts indices between levels, as BioMol's `convert`. Going up maps each element
    /// to its parent (duplicates kept, as BioMol keeps them; `View::unique` drops them);
    /// going down concatenates each parent's children in order.
    pub fn convert(&self, indices: &[u32], source: Level, target: Level) -> Vec<u32> {
        match (source, target) {
            (a, b) if a == b => indices.to_vec(),
            (Level::Atom, Level::Residue) => indices
                .iter()
                .filter_map(|atom| self.atom_to_res.get(*atom as usize).copied())
                .collect(),
            (Level::Residue, Level::Chain) => indices
                .iter()
                .filter_map(|residue| self.res_to_chain.get(*residue as usize).copied())
                .collect(),
            (Level::Atom, Level::Chain) => {
                let residues = self.convert(indices, Level::Atom, Level::Residue);
                self.convert(&residues, Level::Residue, Level::Chain)
            }
            (Level::Residue, Level::Atom) => {
                Self::children(&self.res_atom_indptr, &self.res_atom_indices, indices)
            }
            (Level::Chain, Level::Residue) => {
                Self::children(&self.chain_res_indptr, &self.chain_res_indices, indices)
            }
            (Level::Chain, Level::Atom) => {
                let residues = self.convert(indices, Level::Chain, Level::Residue);
                self.convert(&residues, Level::Residue, Level::Atom)
            }
            _ => unreachable!("every level pair is covered"),
        }
    }
}

/// A selection: a level and the indices of the elements in it. BioMol's `View`, less the
/// pointer to the molecule, which the caller carries as the structure id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub level: Level,
    pub indices: Vec<u32>,
}

impl View {
    pub fn new(level: Level, indices: Vec<u32>) -> Self {
        Self { level, indices }
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Drops duplicates, keeping first-occurrence order - BioMol's `unique`.
    pub fn unique(mut self) -> Self {
        let mut seen = std::collections::HashSet::new();
        self.indices.retain(|index| seen.insert(*index));
        self
    }

    /// The same selection at another level, unique. BioMol's `.atoms/.residues/.chains`.
    pub fn at(&self, level: Level, table: &IndexTable) -> View {
        View::new(level, table.convert(&self.indices, self.level, level)).unique()
    }
    pub fn to_atoms(&self, table: &IndexTable) -> View {
        self.at(Level::Atom, table)
    }
    pub fn to_residues(&self, table: &IndexTable) -> View {
        self.at(Level::Residue, table)
    }
    pub fn to_chains(&self, table: &IndexTable) -> View {
        self.at(Level::Chain, table)
    }

    /// Drops indices the structure does not have, so a stale selection cannot index out of
    /// bounds after a structure is reloaded.
    pub fn bounded(mut self, table: &IndexTable) -> Self {
        let limit = match self.level {
            Level::Atom => table.atom_count(),
            Level::Residue => table.residue_count(),
            Level::Chain => table.chain_count(),
        } as u32;
        self.indices.retain(|index| *index < limit);
        self
    }
}

/// A closed range of author residue numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberRange {
    pub from: i32,
    pub to: i32,
}

/// The conditions a selection is built from, all ANDed - BioMol's `select(**kwargs)`,
/// written down. Every field is optional; an empty spec selects everything. Names are
/// BioMol's CIF feature names where one exists, so a spec is readable on both sides.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Where {
    /// Chain ids in the `A_1` form (`chain_id`), or author ids (`auth_asym_id`): either
    /// matches, since both are how people name a chain.
    pub chains: Vec<String>,
    /// `polymer`, `non-polymer`, `water`, `branched`.
    pub entity_type: Vec<String>,
    /// Residue indices in the structure - what the sequence strip and the tree hand back.
    pub residue_indices: Vec<u32>,
    /// Author residue numbers (`auth_idx`), as closed ranges.
    pub residue_numbers: Vec<NumberRange>,
    /// Three-letter codes (`chem_comp_id`).
    pub residue_names: Vec<String>,
    /// `hetero`: true for ligands and waters, false for the polymer.
    pub hetero: Option<bool>,
    /// `H`, `E`, `C` (helix, strand, other).
    pub secondary: Vec<String>,
    /// Atom names, `CA`, `NZ`.
    pub atom_names: Vec<String>,
    /// Element symbols.
    pub elements: Vec<String>,
    /// Atom indices in the structure.
    pub atom_indices: Vec<u32>,
}

/// A selection spec: the level the result is wanted at, and the conditions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub level: Level,
    #[serde(default)]
    pub r#where: Where,
}

fn matches_any(values: &[String], candidate: &str) -> bool {
    values.is_empty()
        || values
            .iter()
            .any(|value| value.trim().eq_ignore_ascii_case(candidate.trim()))
}

/// Evaluates a spec against a structure: every atom is tested against every condition,
/// its residue's and chain's included, and the surviving atoms are lifted to the level
/// asked for. One pass over the atoms, which is what a ribosome allows.
pub fn evaluate(structure: &Structure, table: &IndexTable, spec: &Spec) -> View {
    let w = &spec.r#where;
    let chain_ok: Vec<bool> = structure
        .chains
        .iter()
        .map(|chain| {
            (w.chains.is_empty()
                || w.chains.iter().any(|wanted| {
                    // Exactly: chain ids are case-sensitive, and a ribosome has both an
                    // `A` and an `a`.
                    let wanted = wanted.trim();
                    wanted == chain.id.trim() || wanted == chain.auth_id.trim()
                }))
                && matches_any(&w.entity_type, &chain.entity_type)
        })
        .collect();
    let residue_set: std::collections::HashSet<u32> = w.residue_indices.iter().copied().collect();
    let atom_set: std::collections::HashSet<u32> = w.atom_indices.iter().copied().collect();
    let residue_ok: Vec<bool> = structure
        .residues
        .iter()
        .enumerate()
        .map(|(index, residue)| {
            chain_ok
                .get(residue.chain_index as usize)
                .copied()
                .unwrap_or(false)
                && (residue_set.is_empty() || residue_set.contains(&(index as u32)))
                && (w.residue_numbers.is_empty()
                    || w.residue_numbers
                        .iter()
                        .any(|range| residue.number >= range.from && residue.number <= range.to))
                && matches_any(&w.residue_names, &residue.name)
                && w.hetero.map_or(true, |hetero| {
                    hetero == matches!(residue.kind, ResidueKind::Water | ResidueKind::Ligand)
                })
                && (w.secondary.is_empty()
                    || w.secondary.iter().any(|code| {
                        let code = code.trim().to_ascii_uppercase();
                        match residue.secondary {
                            SecondaryStructure::Helix => code == "H",
                            SecondaryStructure::Sheet => code == "E",
                            _ => code == "C" || code == "-",
                        }
                    }))
        })
        .collect();
    let atoms: Vec<u32> = (0..structure.atom_count())
        .filter(|atom| {
            let residue = table.atom_to_res.get(*atom).copied().unwrap_or(u32::MAX) as usize;
            residue_ok.get(residue).copied().unwrap_or(false)
                && (atom_set.is_empty() || atom_set.contains(&(*atom as u32)))
                && (w.atom_names.is_empty()
                    || structure
                        .atom_names
                        .get(*atom)
                        .is_some_and(|name| matches_any(&w.atom_names, name)))
                && (w.elements.is_empty()
                    || structure.elements.get(*atom).is_some_and(|element| {
                        matches_any(&w.elements, super::chem::symbol(*element))
                    }))
        })
        .map(|atom| atom as u32)
        .collect();
    View::new(Level::Atom, atoms).at(spec.level, table)
}

/// The two masks the viewport outlines with, from a view.
///
/// Atoms are the view's atoms. Residues are the view's residues for a chain or residue
/// view, and none for an atom view: a ribbon is a residue's, not an atom's, and outlining
/// a whole residue for one atom would say the wrong thing.
pub fn masks(view: &View, table: &IndexTable) -> (Vec<u8>, Vec<u8>) {
    let mut atoms = vec![0u8; table.atom_count()];
    for atom in &view.to_atoms(table).indices {
        if let Some(slot) = atoms.get_mut(*atom as usize) {
            *slot = 1;
        }
    }
    let mut residues = vec![0u8; table.residue_count()];
    if view.level != Level::Atom {
        for residue in &view.to_residues(table).indices {
            if let Some(slot) = residues.get_mut(*residue as usize) {
                *slot = 1;
            }
        }
    }
    (atoms, residues)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two chains; chain 0 has residues 0 and 2, chain 1 has residue 1 - interleaved, as
    /// depositions are. Residue 0 has atoms 0,1; residue 1 has atom 2; residue 2 has 3,4.
    fn table() -> IndexTable {
        IndexTable::from_parents(vec![0, 0, 1, 2, 2], vec![0, 1, 0], 2)
    }

    #[test]
    fn csr_lists_children_in_order_even_when_interleaved() {
        let t = table();
        assert_eq!(t.chain_res_indptr, vec![0, 2, 3]);
        assert_eq!(t.chain_res_indices, vec![0, 2, 1]);
        assert_eq!(t.res_atom_indptr, vec![0, 2, 3, 5]);
        assert_eq!(t.res_atom_indices, vec![0, 1, 2, 3, 4]);
        assert_eq!(t.chain_count(), 2);
    }

    #[test]
    fn converts_between_every_pair_of_levels() {
        let t = table();
        assert_eq!(
            t.convert(&[0, 2, 4], Level::Atom, Level::Residue),
            vec![0, 1, 2]
        );
        assert_eq!(t.convert(&[0, 4], Level::Atom, Level::Chain), vec![0, 0]);
        assert_eq!(t.convert(&[0], Level::Chain, Level::Residue), vec![0, 2]);
        assert_eq!(t.convert(&[0], Level::Chain, Level::Atom), vec![0, 1, 3, 4]);
        assert_eq!(
            t.convert(&[2, 0], Level::Residue, Level::Atom),
            vec![3, 4, 0, 1]
        );
        assert_eq!(
            t.convert(&[9], Level::Atom, Level::Residue),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn views_move_between_levels_and_stay_unique() {
        let t = table();
        let atoms = View::new(Level::Atom, vec![4, 0, 3]);
        assert_eq!(atoms.to_residues(&t).indices, vec![2, 0]);
        assert_eq!(atoms.to_chains(&t).indices, vec![0]);
        assert_eq!(
            View::new(Level::Chain, vec![1]).to_atoms(&t).indices,
            vec![2]
        );
        assert_eq!(
            View::new(Level::Atom, vec![7, 1]).bounded(&t).indices,
            vec![1]
        );
    }

    #[test]
    fn masks_light_atoms_and_residues_but_not_residues_for_an_atom() {
        let t = table();
        let (atoms, residues) = masks(&View::new(Level::Chain, vec![0]), &t);
        assert_eq!(atoms, vec![1, 1, 0, 1, 1]);
        assert_eq!(residues, vec![1, 0, 1]);
        let (atoms, residues) = masks(&View::new(Level::Atom, vec![2]), &t);
        assert_eq!(atoms, vec![0, 0, 1, 0, 0]);
        assert_eq!(residues, vec![0, 0, 0]);
    }

    #[test]
    fn out_of_range_indices_from_a_stale_selection_are_ignored() {
        let t = table();
        assert_eq!(t.convert(&[9, 0], Level::Residue, Level::Atom), vec![0, 1]);
        assert_eq!(
            t.convert(&[u32::MAX], Level::Chain, Level::Atom),
            Vec::<u32>::new()
        );
        assert_eq!(
            t.convert(&[7], Level::Residue, Level::Chain),
            Vec::<u32>::new()
        );
        let (atoms, residues) = masks(&View::new(Level::Residue, vec![1, 40]), &t);
        assert_eq!(atoms, vec![0, 0, 1, 0, 0]);
        assert_eq!(residues, vec![0, 1, 0]);
        assert_eq!(
            View::new(Level::Chain, vec![2, 1]).bounded(&t).indices,
            vec![1]
        );
    }

    #[test]
    fn a_residue_or_chain_with_no_atoms_has_no_children() {
        // Residue 1 and chain 1 own nothing; a parent array pointing past the end is ignored.
        let t = IndexTable::from_parents(vec![0, 2, 9], vec![0, 0, 0], 2);
        assert_eq!(
            t.convert(&[1], Level::Residue, Level::Atom),
            Vec::<u32>::new()
        );
        assert_eq!(
            t.convert(&[1], Level::Chain, Level::Residue),
            Vec::<u32>::new()
        );
        assert_eq!(t.convert(&[0], Level::Chain, Level::Atom), vec![0, 1]);
        let empty = IndexTable::from_parents(Vec::new(), Vec::new(), 0);
        assert_eq!(empty.chain_count(), 0);
        assert!(masks(&View::new(Level::Chain, vec![0]), &empty)
            .0
            .is_empty());
    }

    #[test]
    fn spec_conditions_select_from_a_real_structure() {
        let text = "\
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA A   1       1.470   0.000   0.000  1.00 20.00           C
ATOM      3  N   GLY a   5       4.000   0.000   0.000  1.00 20.00           N
HETATM    4  O   HOH A 101      10.000   0.000   0.000  1.00 20.00           O
";
        let structure = super::super::pdb::parse(text, "spec.pdb");
        let t = IndexTable::from_structure(&structure);
        let run =
            |json: &str| evaluate(&structure, &t, &serde_json::from_str(json).unwrap()).indices;
        assert_eq!(run(r#"{"level":"atom"}"#), vec![0, 1, 2, 3]);
        // Chain ids are case-sensitive: a ribosome has both A and a.
        assert_eq!(
            run(r#"{"level":"residue","where":{"chains":["a"]}}"#),
            vec![1]
        );
        assert_eq!(run(r#"{"level":"atom","where":{"hetero":true}}"#), vec![3]);
        assert_eq!(
            run(r#"{"level":"atom","where":{"elements":["n"]}}"#),
            vec![0, 2]
        );
        assert_eq!(
            run(r#"{"level":"residue","where":{"residue_numbers":[{"from":-5,"to":5}]}}"#),
            vec![0, 1]
        );
        assert_eq!(
            run(r#"{"level":"chain","where":{"atom_indices":[3, 999]}}"#),
            vec![0]
        );
        assert!(run(r#"{"level":"atom","where":{"residue_names":["TRP"]}}"#).is_empty());
    }

    #[test]
    fn spec_round_trips_with_defaults() {
        let spec: Spec =
            serde_json::from_str(r#"{"level":"residue","where":{"chains":["A_1"]}}"#).unwrap();
        assert_eq!(spec.level, Level::Residue);
        assert_eq!(spec.r#where.chains, vec!["A_1"]);
        assert!(spec.r#where.hetero.is_none());
        let empty: Spec = serde_json::from_str(r#"{"level":"atom"}"#).unwrap();
        assert_eq!(empty.r#where, Where::default());
    }
}
