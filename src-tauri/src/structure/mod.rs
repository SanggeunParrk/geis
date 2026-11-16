//! Reading molecular structures and getting them to the renderer.

pub mod bonds;
pub mod chem;
pub mod cif;
pub mod components;
pub mod contacts;
pub mod index;
pub mod model;
pub mod pdb;

pub use model::{Bond, BondKind, BondOrder, Structure, StructureHeader};

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum StructureError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} has no recognised extension - expected .pdb, .cif, .mmcif, or .ent")]
    UnknownFormat { path: String },
    #[error("{path} contains no atoms")]
    Empty { path: String },
}

/// Loads a structure from disk, inferring the format from the extension and the content.
///
/// Extension sniffing alone is not enough: prediction tools write `.cif` files, mmCIF
/// files arrive named `.pdb` from conversion scripts, and gzip-compressed files are
/// common. Content is the tiebreaker.
pub fn load_file(path: &Path) -> Result<Structure, StructureError> {
    load_file_as(path, Unit::Deposited)
}

pub use cif::{Reading, Unit};

/// Loads a structure as deposited, or as its first biological assembly where the file
/// says how one is made. Only mmCIF says; a PDB file is read as it is written.
pub fn load_file_as(path: &Path, unit: Unit) -> Result<Structure, StructureError> {
    load_file_reading(path, &Reading::from(unit))
}

/// Reads a structure file the way `reading` says: which assembly, model and conformers.
pub fn load_file_reading(path: &Path, reading: &Reading) -> Result<Structure, StructureError> {
    // Read as bytes and decoded leniently. Both parsers tolerate a bad byte, and one
    // Latin-1 angstrom sign in a remark is no reason to refuse the coordinates under it.
    let bytes = std::fs::read(path).map_err(|source| StructureError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let text = String::from_utf8_lossy(&bytes);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("structure");
    let structure =
        parse_text_reading(&text, name, reading).ok_or_else(|| StructureError::UnknownFormat {
            path: path.display().to_string(),
        })?;
    if structure.is_empty() {
        return Err(StructureError::Empty {
            path: path.display().to_string(),
        });
    }
    Ok(structure)
}

/// PDB records that identify the format even before any coordinates appear.
const PDB_RECORDS: &[&str] = &[
    "ATOM  ", "HETATM", "MODEL ", "HEADER", "CRYST1", "SEQRES", "HELIX ", "SHEET ", "TITLE ",
    "COMPND", "EXPDTA", "REMARK",
];

/// Parses structure text, choosing the reader by content.
///
/// mmCIF is identified by its `data_` block, which is always at the top. PDB is identified
/// by a record name anywhere in the file - and it has to be *anywhere*, not within the
/// first N lines: a large deposition's header runs to thousands of lines before the first
/// ATOM (6VXX reaches its at line 2160), and a windowed check rejects exactly the files
/// that matter most.
pub fn parse_text(text: &str, source: &str) -> Option<Structure> {
    parse_text_as(text, source, Unit::Deposited)
}

/// As `parse_text`, building the unit asked for.
pub fn parse_text_as(text: &str, source: &str, unit: Unit) -> Option<Structure> {
    parse_text_reading(text, source, &Reading::from(unit))
}

/// `parse_text_as`, read the way `reading` says.
pub fn parse_text_reading(text: &str, source: &str, reading: &Reading) -> Option<Structure> {
    // A byte-order mark, which Windows editors put at the top of a file they save. It is
    // not whitespace, so `data_` behind it was not at the start of its line and an mmCIF
    // file saved from Notepad was refused as an unknown format.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    // The first line that is not blank or a comment, wherever it falls: a tool that writes
    // a long comment banner pushes `data_` past any fixed window, and the file then fell
    // through to the PDB sniff - which matches, since `_atom_site` rows start with `ATOM`
    // - and was read by columns into nonsense.
    let first_content = text
        .lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty() && !line.starts_with('#'));
    let looks_like_cif = first_content.is_some_and(|line| line.starts_with("data_"))
        || text
            .lines()
            .take(100)
            .any(|line| line.trim_start().starts_with("data_"));
    let looks_like_pdb = !looks_like_cif
        && text
            .lines()
            .any(|line| PDB_RECORDS.iter().any(|record| line.starts_with(record)));

    // The reader returns what the file said about bonding along with the coordinates: an
    // mmCIF carries its own component dictionary and its `_struct_conn` records, and the
    // bond graph is built from those rather than from distances between atoms.
    let (mut structure, evidence) = if looks_like_cif {
        cif::parse_reading(text, source, reading)
    } else if looks_like_pdb {
        (
            pdb::parse_reading(text, source, reading.model.as_deref(), reading.alternate),
            bonds::Evidence::default(),
        )
    } else {
        return None;
    };

    bonds::build(&mut structure, &evidence);
    Some(structure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_reader_by_content_not_by_name() {
        let cif = "data_X\nloop_\n_atom_site.type_symbol\n_atom_site.label_atom_id\n\
                   _atom_site.label_comp_id\n_atom_site.label_asym_id\n_atom_site.auth_seq_id\n\
                   _atom_site.Cartn_x\n_atom_site.Cartn_y\n_atom_site.Cartn_z\n\
                   C CA ALA A 1 1.0 2.0 3.0\n";
        // Named .pdb but actually mmCIF, which happens with conversion scripts.
        let structure = parse_text(cif, "mislabelled.pdb").expect("should still parse");
        assert_eq!(structure.atom_count(), 1);
    }

    #[test]
    fn unrecognised_text_is_refused_rather_than_silently_empty() {
        assert!(parse_text("hello, this is not a structure", "notes.txt").is_none());
    }

    #[test]
    fn a_long_header_does_not_hide_the_coordinates() {
        // Real depositions put thousands of REMARK and SEQRES lines before the first
        // ATOM; a windowed sniff rejected them outright.
        let mut text = String::from("HEADER    VIRAL PROTEIN\n");
        for index in 0..5000 {
            text.push_str(&format!("REMARK 999 padding line {index}\n"));
        }
        text.push_str(
            "ATOM      1  N   MET A   1      27.340  24.430   2.614  1.00 45.20           N\n",
        );
        let structure = parse_text(&text, "long.pdb").expect("should parse");
        assert_eq!(structure.atom_count(), 1);
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_format() {
        let cif = "\u{feff}data_BOM\nloop_\n_atom_site.type_symbol\n_atom_site.label_atom_id\n\
                   _atom_site.label_comp_id\n_atom_site.label_asym_id\n_atom_site.auth_seq_id\n\
                   _atom_site.Cartn_x\n_atom_site.Cartn_y\n_atom_site.Cartn_z\n\
                   C CA ALA A 1 1.0 2.0 3.0\n";
        let structure = parse_text(cif, "bom.cif").expect("an mmCIF file with a BOM");
        assert_eq!(structure.atom_count(), 1);
        assert_eq!(structure.entry_id, "BOM");

        let pdb = "\u{feff}ATOM      1  N   MET A   1      27.340  24.430   2.614  1.00 45.20           N\n";
        let structure = parse_text(pdb, "bom.pdb").expect("a PDB file with a BOM");
        assert_eq!(structure.atom_count(), 1);
    }

    #[test]
    fn a_long_comment_banner_does_not_turn_mmcif_into_pdb() {
        let mut cif = String::new();
        for index in 0..300 {
            cif.push_str(&format!("# generated by some tool, line {index}\n"));
        }
        cif.push_str(
            "data_LONG\nloop_\n_atom_site.group_PDB\n_atom_site.id\n_atom_site.type_symbol\n\
             _atom_site.label_atom_id\n_atom_site.label_comp_id\n_atom_site.label_asym_id\n\
             _atom_site.auth_seq_id\n_atom_site.Cartn_x\n_atom_site.Cartn_y\n_atom_site.Cartn_z\n\
             ATOM   1 C CA ALA A 1 1.0 2.0 3.0\n",
        );
        let structure = parse_text(&cif, "banner.cif").expect("is an mmCIF");
        assert_eq!(structure.entry_id, "LONG");
        assert_eq!(structure.position(0), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_file_with_records_but_no_atoms_parses_to_empty() {
        // `load_file` turns this into a "contains no atoms" error rather than an empty view.
        let structure = parse_text("HEADER    NOTHING\nEND\n", "header.pdb").expect("is a PDB");
        assert!(structure.is_empty());
        assert!(structure.bonds.is_empty());
        let structure = parse_text("data_EMPTY\n", "empty.cif").expect("is an mmCIF");
        assert!(structure.is_empty());
    }

    #[test]
    fn far_apart_and_huge_coordinates_load_without_overflow() {
        // Finite but absurd coordinates: the bond grid's cell index saturates, and stepping
        // to the neighbouring cell must not overflow.
        let cif = "data_FAR\nloop_\n_atom_site.type_symbol\n_atom_site.label_atom_id\n\
                   _atom_site.label_comp_id\n_atom_site.label_asym_id\n_atom_site.auth_seq_id\n\
                   _atom_site.Cartn_x\n_atom_site.Cartn_y\n_atom_site.Cartn_z\n\
                   C C1 LIG A 1 3e38 -3e38 3e38\n\
                   O O1 LIG A 1 3e38 -3e38 3e38\n\
                   C C2 LIG A 1 1e12 1e12 1e12\n";
        let structure = parse_text(cif, "far.cif").expect("parses");
        assert_eq!(structure.atom_count(), 3);
        let _ = structure.encode();
    }

    #[test]
    fn bonds_are_inferred_as_part_of_loading() {
        let pdb = "\
ATOM      1  N   MET A   1      27.340  24.430   2.614  1.00 45.20           N
ATOM      2  CA  MET A   1      26.266  25.413   2.842  1.00 43.11           C
";
        let structure = parse_text(pdb, "two.pdb").unwrap();
        assert_eq!(structure.bonds.len(), 1, "N-CA is 1.47 A apart");
    }
}
