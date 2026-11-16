//! Chemical component templates: which atoms are bonded inside one residue.
//!
//! Bonds are not guessed from distance. The Chemical Component Dictionary says exactly
//! which atoms of ALA are bonded and with what order, and a coordinate file names its
//! components - so the bonds of a residue are a lookup, not an inference. This is the
//! table for the components that appear in almost every file; anything else comes from the
//! `_chem_comp_bond` loop the mmCIF carries with it (see `cif.rs`).
//!
//! Generated from the CCD definitions: the twenty standard amino acids and selenocysteine
//! and selenomethionine, plus the RNA and DNA nucleotides. Bond codes are `s` single, `d`
//! double, `t` triple, `a` aromatic - the same distinction `_chem_comp_bond.value_order`
//! and `pdbx_aromatic_flag` make.

use super::model::{BondOrder, Linking};
use std::collections::HashMap;
use std::sync::OnceLock;

/// One component: how it links into a polymer, and its bonds as `ATOM-ATOM<order>`.
type Row = (&'static str, Linking, &'static str);

const TABLE: &[Row] = &[
    (
        "ALA",
        Linking::Peptide,
        "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-HB1s CB-HB2s CB-HB3s OXT-HXTs",
    ),
    (
        "ARG",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-CDs CG-HG2s ",
            "CG-HG3s CD-NEs CD-HD2s CD-HD3s NE-CZs NE-HEs CZ-NH1s CZ-NH2d NH1-HH11s NH1-HH12s ",
            "NH2-HH21s NH2-HH22s OXT-HXTs",
        ),
    ),
    (
        "ASN",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-OD1d CG-ND2s ",
            "ND2-HD21s ND2-HD22s OXT-HXTs",
        ),
    ),
    (
        "ASP",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-OD1d CG-OD2s ",
            "OD2-HD2s OXT-HXTs",
        ),
    ),
    (
        "CYS",
        Linking::Peptide,
        "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-SGs CB-HB2s CB-HB3s SG-HGs OXT-HXTs",
    ),
    (
        "GLN",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-CDs CG-HG2s ",
            "CG-HG3s CD-OE1d CD-NE2s NE2-HE21s NE2-HE22s OXT-HXTs",
        ),
    ),
    (
        "GLU",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-CDs CG-HG2s ",
            "CG-HG3s CD-OE1d CD-OE2s OE2-HE2s OXT-HXTs",
        ),
    ),
    (
        "GLY",
        Linking::Peptide,
        "N-CAs N-Hs N-H2s CA-Cs CA-HA2s CA-HA3s C-Od C-OXTs OXT-HXTs",
    ),
    (
        "HIS",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-ND1a CG-CD2a ",
            "ND1-CE1a ND1-HD1s CD2-NE2a CD2-HD2s CE1-NE2a CE1-HE1s NE2-HE2s OXT-HXTs",
        ),
    ),
    (
        "ILE",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CG1s CB-CG2s CB-HBs CG1-CD1s ",
            "CG1-HG12s CG1-HG13s CG2-HG21s CG2-HG22s CG2-HG23s CD1-HD11s CD1-HD12s CD1-HD13s OXT-HXTs",
        ),
    ),
    (
        "LEU",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-CD1s CG-CD2s ",
            "CG-HGs CD1-HD11s CD1-HD12s CD1-HD13s CD2-HD21s CD2-HD22s CD2-HD23s OXT-HXTs",
        ),
    ),
    (
        "LYS",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-CDs CG-HG2s ",
            "CG-HG3s CD-CEs CD-HD2s CD-HD3s CE-NZs CE-HE2s CE-HE3s NZ-HZ1s NZ-HZ2s NZ-HZ3s OXT-HXTs",
        ),
    ),
    (
        "MET",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-SDs CG-HG2s ",
            "CG-HG3s SD-CEs CE-HE1s CE-HE2s CE-HE3s OXT-HXTs",
        ),
    ),
    (
        "PHE",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-CD1a CG-CD2a ",
            "CD1-CE1a CD1-HD1s CD2-CE2a CD2-HD2s CE1-CZa CE1-HE1s CE2-CZa CE2-HE2s CZ-HZs OXT-HXTs",
        ),
    ),
    (
        "PRO",
        Linking::Peptide,
        concat!(
            "N-CAs N-CDs N-Hs CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-CDs CG-HG2s ",
            "CG-HG3s CD-HD2s CD-HD3s OXT-HXTs",
        ),
    ),
    (
        "SER",
        Linking::Peptide,
        "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-OGs CB-HB2s CB-HB3s OG-HGs OXT-HXTs",
    ),
    (
        "THR",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-OG1s CB-CG2s CB-HBs OG1-HG1s ",
            "CG2-HG21s CG2-HG22s CG2-HG23s OXT-HXTs",
        ),
    ),
    (
        "TRP",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-CD1a CG-CD2a ",
            "CD1-NE1a CD1-HD1s CD2-CE2a CD2-CE3a NE1-CE2a NE1-HE1s CE2-CZ2a CE3-CZ3a CE3-HE3s ",
            "CZ2-CH2a CZ2-HZ2s CZ3-CH2a CZ3-HZ3s CH2-HH2s OXT-HXTs",
        ),
    ),
    (
        "TYR",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CGs CB-HB2s CB-HB3s CG-CD1a CG-CD2a ",
            "CD1-CE1a CD1-HD1s CD2-CE2a CD2-HD2s CE1-CZa CE1-HE1s CE2-CZa CE2-HE2s CZ-OHs OH-HHs ",
            "OXT-HXTs",
        ),
    ),
    (
        "VAL",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs CB-CG1s CB-CG2s CB-HBs CG1-HG11s ",
            "CG1-HG12s CG1-HG13s CG2-HG21s CG2-HG22s CG2-HG23s OXT-HXTs",
        ),
    ),
    (
        "MSE",
        Linking::Peptide,
        concat!(
            "N-CAs N-Hs N-H2s CA-Cs CA-CBs CA-HAs C-Od C-OXTs OXT-HXTs CB-CGs CB-HB2s CB-HB3s CG-SEs ",
            "CG-HG2s CG-HG3s SE-CEs CE-HE1s CE-HE2s CE-HE3s",
        ),
    ),
    (
        "SEC",
        Linking::Peptide,
        "N-CAs N-Hs N-H2s CA-CBs CA-Cs CA-HAs CB-SEs CB-HB2s CB-HB3s SE-HEs C-Od C-OXTs OXT-HXTs",
    ),
    (
        "A",
        Linking::Rna,
        concat!(
            "OP3-Ps OP3-HOP3s P-OP1d P-OP2s P-O5's OP2-HOP2s O5'-C5's C5'-C4's C5'-H5's C5'-H5''s ",
            "C4'-O4's C4'-C3's C4'-H4's O4'-C1's C3'-O3's C3'-C2's C3'-H3's O3'-HO3's C2'-O2's ",
            "C2'-C1's C2'-H2's O2'-HO2's C1'-N9s C1'-H1's N9-C8a N9-C4a C8-N7a C8-H8s N7-C5a C5-C6a ",
            "C5-C4a C6-N6s C6-N1a N6-H61s N6-H62s N1-C2a C2-N3a C2-H2s N3-C4a",
        ),
    ),
    (
        "C",
        Linking::Rna,
        concat!(
            "OP3-Ps OP3-HOP3s P-OP1d P-OP2s P-O5's OP2-HOP2s O5'-C5's C5'-C4's C5'-H5's C5'-H5''s ",
            "C4'-O4's C4'-C3's C4'-H4's O4'-C1's C3'-O3's C3'-C2's C3'-H3's O3'-HO3's C2'-O2's ",
            "C2'-C1's C2'-H2's O2'-HO2's C1'-N1s C1'-H1's N1-C2s N1-C6s C2-O2d C2-N3s N3-C4d C4-N4s ",
            "C4-C5s N4-H41s N4-H42s C5-C6d C5-H5s C6-H6s",
        ),
    ),
    (
        "G",
        Linking::Rna,
        concat!(
            "OP3-Ps OP3-HOP3s P-OP1d P-OP2s P-O5's OP2-HOP2s O5'-C5's C5'-C4's C5'-H5's C5'-H5''s ",
            "C4'-O4's C4'-C3's C4'-H4's O4'-C1's C3'-O3's C3'-C2's C3'-H3's O3'-HO3's C2'-O2's ",
            "C2'-C1's C2'-H2's O2'-HO2's C1'-N9s C1'-H1's N9-C8a N9-C4a C8-N7a C8-H8s N7-C5a C5-C6s ",
            "C5-C4a C6-O6d C6-N1s N1-C2s N1-H1s C2-N2s C2-N3d N2-H21s N2-H22s N3-C4s",
        ),
    ),
    (
        "U",
        Linking::Rna,
        concat!(
            "OP3-Ps OP3-HOP3s P-OP1d P-OP2s P-O5's OP2-HOP2s O5'-C5's C5'-C4's C5'-H5's C5'-H5''s ",
            "C4'-O4's C4'-C3's C4'-H4's O4'-C1's C3'-O3's C3'-C2's C3'-H3's O3'-HO3's C2'-O2's ",
            "C2'-C1's C2'-H2's O2'-HO2's C1'-N1s C1'-H1's N1-C2s N1-C6s C2-O2d C2-N3s N3-C4s N3-H3s ",
            "C4-O4d C4-C5s C5-C6d C5-H5s C6-H6s",
        ),
    ),
    (
        "DA",
        Linking::Dna,
        concat!(
            "OP3-Ps OP3-HOP3s P-OP1d P-OP2s P-O5's OP2-HOP2s O5'-C5's C5'-C4's C5'-H5's C5'-H5''s ",
            "C4'-O4's C4'-C3's C4'-H4's O4'-C1's C3'-O3's C3'-C2's C3'-H3's O3'-HO3's C2'-C1's ",
            "C2'-H2's C2'-H2''s C1'-N9s C1'-H1's N9-C8a N9-C4a C8-N7a C8-H8s N7-C5a C5-C6a C5-C4a ",
            "C6-N6s C6-N1a N6-H61s N6-H62s N1-C2a C2-N3a C2-H2s N3-C4a",
        ),
    ),
    (
        "DC",
        Linking::Dna,
        concat!(
            "OP3-Ps OP3-HOP3s P-OP1d P-OP2s P-O5's OP2-HOP2s O5'-C5's C5'-C4's C5'-H5's C5'-H5''s ",
            "C4'-O4's C4'-C3's C4'-H4's O4'-C1's C3'-O3's C3'-C2's C3'-H3's O3'-HO3's C2'-C1's ",
            "C2'-H2's C2'-H2''s C1'-N1s C1'-H1's N1-C2s N1-C6s C2-O2d C2-N3s N3-C4d C4-N4s C4-C5s ",
            "N4-H41s N4-H42s C5-C6d C5-H5s C6-H6s",
        ),
    ),
    (
        "DG",
        Linking::Dna,
        concat!(
            "OP3-Ps OP3-HOP3s P-OP1d P-OP2s P-O5's OP2-HOP2s O5'-C5's C5'-C4's C5'-H5's C5'-H5''s ",
            "C4'-O4's C4'-C3's C4'-H4's O4'-C1's C3'-O3's C3'-C2's C3'-H3's O3'-HO3's C2'-C1's ",
            "C2'-H2's C2'-H2''s C1'-N9s C1'-H1's N9-C8a N9-C4a C8-N7a C8-H8s N7-C5a C5-C6s C5-C4a ",
            "C6-O6d C6-N1s N1-C2s N1-H1s C2-N2s C2-N3d N2-H21s N2-H22s N3-C4s",
        ),
    ),
    (
        "DT",
        Linking::Dna,
        concat!(
            "OP3-Ps OP3-HOP3s P-OP1d P-OP2s P-O5's OP2-HOP2s O5'-C5's C5'-C4's C5'-H5's C5'-H5''s ",
            "C4'-O4's C4'-C3's C4'-H4's O4'-C1's C3'-O3's C3'-C2's C3'-H3's O3'-HO3's C2'-C1's ",
            "C2'-H2's C2'-H2''s C1'-N1s C1'-H1's N1-C2s N1-C6s C2-O2d C2-N3s N3-C4s N3-H3s C4-O4d ",
            "C4-C5s C5-C7s C5-C6d C7-H71s C7-H72s C7-H73s C6-H6s",
        ),
    ),
];

/// A component's bonds, by atom name.
#[derive(Debug, Clone)]
pub struct Template {
    pub linking: Linking,
    /// `(atom a, atom b, order)`, in the dictionary's own order.
    pub bonds: Vec<(String, String, BondOrder)>,
}

/// The built-in templates, parsed once.
pub fn builtin(comp_id: &str) -> Option<&'static Template> {
    static PARSED: OnceLock<HashMap<&'static str, Template>> = OnceLock::new();
    let table = PARSED.get_or_init(|| {
        TABLE
            .iter()
            .map(|(comp, linking, bonds)| {
                (
                    *comp,
                    Template {
                        linking: *linking,
                        bonds: parse(bonds),
                    },
                )
            })
            .collect()
    });
    table.get(comp_id.trim().to_ascii_uppercase().as_str())
}

/// Whether a component is one this table knows, so a caller can tell "no bonds here" from
/// "no template for this".
pub fn is_builtin(comp_id: &str) -> bool {
    builtin(comp_id).is_some()
}

fn parse(encoded: &str) -> Vec<(String, String, BondOrder)> {
    encoded
        .split_whitespace()
        .filter_map(|entry| {
            let (pair, order) = entry.split_at(entry.len().checked_sub(1)?);
            let order = match order {
                "d" => BondOrder::Double,
                "t" => BondOrder::Triple,
                "a" => BondOrder::Aromatic,
                _ => BondOrder::Single,
            };
            let (a, b) = pair.split_once('-')?;
            Some((a.to_string(), b.to_string(), order))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alanine_is_the_dictionarys_alanine() {
        let ala = builtin("ALA").expect("ALA is a standard residue");
        assert_eq!(ala.linking, Linking::Peptide);
        // The bonds the CCD gives, including the ones to hydrogens a file may not have.
        let has = |a: &str, b: &str, order: BondOrder| {
            ala.bonds
                .iter()
                .any(|(x, y, o)| ((x == a && y == b) || (x == b && y == a)) && *o == order)
        };
        assert!(has("N", "CA", BondOrder::Single));
        assert!(has("CA", "CB", BondOrder::Single));
        // The carbonyl is a double bond, which is the sort of thing distance cannot tell
        // you and a dictionary can.
        assert!(has("C", "O", BondOrder::Double));
        assert!(
            !has("N", "CB", BondOrder::Single),
            "not a bond in any alanine"
        );
    }

    #[test]
    fn the_aromatic_rings_are_marked_aromatic() {
        let phe = builtin("PHE").expect("PHE");
        let aromatic = phe
            .bonds
            .iter()
            .filter(|(_, _, order)| *order == BondOrder::Aromatic)
            .count();
        // Six ring bonds in phenylalanine's benzene.
        assert_eq!(aromatic, 6);

        let trp = builtin("TRP").expect("TRP");
        assert!(
            trp.bonds
                .iter()
                .filter(|(_, _, order)| *order == BondOrder::Aromatic)
                .count()
                >= 9,
            "indole is two fused rings",
        );
    }

    #[test]
    fn nucleotides_link_as_nucleic_acids() {
        assert_eq!(builtin("DA").expect("DA").linking, Linking::Dna);
        assert_eq!(builtin("A").expect("A").linking, Linking::Rna);
        assert_eq!(builtin("GLY").expect("GLY").linking, Linking::Peptide);
        // Water is not in the table: it has no bonds worth drawing and no linking.
        assert!(builtin("HOH").is_none());
        assert!(
            builtin("HEM").is_none(),
            "a ligand comes from the file's own loop"
        );
    }

    #[test]
    fn every_row_parses_and_names_two_distinct_atoms() {
        for (comp, _, encoded) in TABLE {
            let template = builtin(comp).expect("in the table");
            assert!(!template.bonds.is_empty(), "{comp} has bonds");
            // The parser drops an entry it cannot read, so a typo in a row is silent
            // unless the count is checked: TYR once lost its OXT-HXT bond to a stray space.
            assert_eq!(
                template.bonds.len(),
                encoded.split_whitespace().count(),
                "{comp}: an entry did not parse"
            );
            for (a, b, _) in &template.bonds {
                assert!(!a.is_empty() && !b.is_empty(), "{comp}: empty atom name");
                assert_ne!(a, b, "{comp}: an atom bonded to itself");
            }
        }
    }
}
