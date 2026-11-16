//! PDB format reader.
//!
//! PDB is a fixed-column format, and that is the whole difficulty: fields are defined by
//! byte position, not by whitespace. Splitting on spaces works on tidy files and then
//! fails on the ones that matter - a residue number of -1 running into the insertion code
//! column, or a four-character atom name with no gap before the residue name. Every field
//! here is sliced by its documented columns.

use super::chem;
use super::model::{RawAtom, SecondaryStructure, Structure};

/// Slices `line` by 1-based inclusive PDB column numbers, tolerating short lines - many
/// writers truncate trailing fields rather than padding them.
fn columns(line: &[u8], from: usize, to: usize) -> &str {
    let start = from - 1;
    let end = to.min(line.len());
    if start >= end {
        return "";
    }
    std::str::from_utf8(&line[start..end]).unwrap_or("").trim()
}

pub fn parse(text: &str, source: &str) -> Structure {
    parse_reading(text, source, None, None)
}

/// Reads a PDB file's `model` (its first, unsaid), keeping for each residue the conformer
/// `alternate` names where it has it, and its most occupied one otherwise.
pub fn parse_reading(
    text: &str,
    source: &str,
    model: Option<&str>,
    alternate: Option<char>,
) -> Structure {
    let mut candidates: Vec<(RawAtom, Option<char>)> = Vec::new();
    let mut conformers = super::model::Conformers::default();
    let mut models: Vec<String> = Vec::new();
    // Whether the lines being read are of the model wanted: the first, or the one named.
    let mut reading = true;
    let mut method_seen: Option<String> = None;
    let mut title = String::new();
    let mut helices: Vec<(String, i32, i32)> = Vec::new();
    let mut sheets: Vec<(String, i32, i32)> = Vec::new();
    let mut model_seen = false;

    for line in text.lines() {
        let bytes = line.as_bytes();
        let record = columns(bytes, 1, 6);

        match record {
            "MODEL" => {
                // One model is loaded. An NMR ensemble or a multi-sample prediction would
                // otherwise appear as several overlapping copies, with bond inference
                // wiring them to each other. The rest are listed, to be chosen instead.
                let number = columns(bytes, 7, 80)
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                reading = match model {
                    Some(wanted) => number == wanted,
                    None => !model_seen,
                };
                model_seen = true;
                models.push(number);
            }
            "ENDMDL" => reading = false,
            "TITLE" => {
                let part = columns(bytes, 11, 80);
                if !part.is_empty() {
                    if !title.is_empty() {
                        title.push(' ');
                    }
                    title.push_str(part);
                }
            }
            "EXPDTA" => {
                let method = columns(bytes, 11, 79);
                if !method.is_empty() && method_seen.is_none() {
                    method_seen = Some(method.to_string());
                }
            }
            "HELIX" => {
                let chain = chain_or_default(columns(bytes, 20, 20));
                if let (Some(from), Some(to)) = (
                    residue_number(columns(bytes, 22, 25)),
                    residue_number(columns(bytes, 34, 37)),
                ) {
                    helices.push((chain, from, to));
                }
            }
            "SHEET" => {
                let chain = chain_or_default(columns(bytes, 22, 22));
                if let (Some(from), Some(to)) = (
                    residue_number(columns(bytes, 23, 26)),
                    residue_number(columns(bytes, 34, 37)),
                ) {
                    sheets.push((chain, from, to));
                }
            }
            "ATOM" | "HETATM" => {
                if !reading {
                    continue;
                }
                if let Some((atom, alt_loc)) = parse_atom(bytes, record == "HETATM") {
                    conformers.note(&atom, alt_loc);
                    candidates.push((atom, alt_loc));
                }
            }
            _ => {}
        }
    }

    // A model asked for that the file does not have: its first, rather than nothing.
    if let Some(wanted) = model {
        if !models.iter().any(|number| number == wanted) {
            return parse_reading(text, source, None, alternate);
        }
    }
    let atoms: Vec<RawAtom> = candidates
        .into_iter()
        .filter(|(atom, alt_loc)| conformers.keep(atom, *alt_loc, alternate))
        .map(|(atom, _)| atom)
        .collect();
    let mut structure = Structure::build(title, source.to_string(), atoms);
    structure.experimental_method = method_seen;
    structure.versions.models = models;
    structure.versions.model = model
        .map(str::to_string)
        .or_else(|| structure.versions.models.first().cloned());
    structure.versions.alternates = conformers.labels().iter().map(char::to_string).collect();
    structure.versions.alternate_residues = conformers.doubled();
    structure.versions.alternate = alternate.map(|label| label.to_string());
    apply_secondary(&mut structure, &helices, SecondaryStructure::Helix);
    apply_secondary(&mut structure, &sheets, SecondaryStructure::Sheet);
    // No entity table in this format, so every chain's kind is read off its residues -
    // the same four words the mmCIF twin would carry.
    structure.assign_entities(&std::collections::HashMap::new());
    structure
}

/// A file with no chain column still has one chain, and every record has to call it the
/// same thing: atoms defaulted to `A` and helices left blank never meet.
fn chain_or_default(id: &str) -> String {
    if id.is_empty() {
        "A".to_string()
    } else {
        id.to_string()
    }
}

/// The atom and its alternate-location label, if it has one.
fn parse_atom(line: &[u8], is_hetatm: bool) -> Option<(RawAtom, Option<char>)> {
    // Rust's float parser accepts `nan` and `inf`, and one such atom poisons the bounds the
    // camera frames on and every distance measured from it. It is not a position.
    let coordinate = |from, to| {
        columns(line, from, to)
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
    };
    let x = coordinate(31, 38)?;
    let y = coordinate(39, 46)?;
    let z = coordinate(47, 54)?;

    let alt_loc = columns(line, 17, 17).chars().next();

    let name_raw = if line.len() >= 13 {
        std::str::from_utf8(&line[12..16.min(line.len())]).unwrap_or("")
    } else {
        ""
    };
    let starts_in_column_13 = !name_raw.starts_with(' ');

    let element_field = columns(line, 77, 78);
    let residue_name = columns(line, 18, 20);
    let element = if element_field.is_empty() {
        chem::from_atom_name_in(name_raw, starts_in_column_13, residue_name)
    } else {
        let parsed = chem::from_symbol(element_field);
        // The column names an element this table does not have - a rare lanthanide, say.
        // That is still an answer, and guessing over it from the atom name's first letter
        // can make a metal a hydrogen: hidden with the hydrogens and drawn a third of an
        // angstrom wide. Only a column that is not a symbol at all falls back to the name.
        if parsed == chem::UNKNOWN && !element_field.chars().all(|c| c.is_ascii_alphabetic()) {
            chem::from_atom_name_in(name_raw, starts_in_column_13, residue_name)
        } else {
            parsed
        }
    };

    let insertion = columns(line, 27, 27).chars().next();

    Some((
        RawAtom {
            name: name_raw.trim().to_string(),
            element,
            position: [x, y, z],
            b_factor: columns(line, 61, 66).parse().unwrap_or(0.0),
            occupancy: columns(line, 55, 60).parse().unwrap_or(1.0),
            residue_name: columns(line, 18, 20).to_string(),
            residue_number: residue_number(columns(line, 23, 26)).unwrap_or(0),
            insertion_code: insertion,
            chain_id: chain_or_default(columns(line, 22, 22)),
            is_hetatm,
            // A PDB file has author ids only; the label falls back to them in `Structure::build`.
            label_asym_id: String::new(),
            entity_id: String::new(),
        },
        alt_loc,
    ))
}

/// The residue number field, decimal or hybrid-36.
///
/// Four columns hold at most 9999, so writers that go past it (Phenix, ChimeraX, cctbx)
/// continue in hybrid-36: `A000` is 10000. Read as 0, every water past 9999 had the same
/// number and name as the one before it and the whole run merged into one residue.
fn residue_number(field: &str) -> Option<i32> {
    field.parse().ok().or_else(|| hybrid36(field))
}

/// Decodes a four-character hybrid-36 number, as cctbx's `hy36decode` does: upper-case
/// digits continue from 10000, lower-case ones from where upper-case runs out.
fn hybrid36(field: &str) -> Option<i32> {
    const WIDTH: u32 = 4;
    let bytes = field.as_bytes();
    if bytes.len() != WIDTH as usize {
        return None;
    }
    let upper = bytes[0].is_ascii_uppercase();
    if !upper && !bytes[0].is_ascii_lowercase() {
        return None;
    }
    let mut value: i64 = 0;
    for &byte in bytes {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'Z' if upper => byte - b'A' + 10,
            b'a'..=b'z' if !upper => byte - b'a' + 10,
            _ => return None,
        };
        value = value * 36 + i64::from(digit);
    }
    let block = 36_i64.pow(WIDTH - 1);
    let decoded = if upper {
        value - 10 * block + 10_i64.pow(WIDTH)
    } else {
        value + 16 * block + 10_i64.pow(WIDTH)
    };
    i32::try_from(decoded).ok()
}

fn apply_secondary(
    structure: &mut Structure,
    ranges: &[(String, i32, i32)],
    kind: SecondaryStructure,
) {
    for (chain, from, to) in ranges {
        let indices: Vec<usize> = structure
            .residue_indices_in_range(chain, *from, *to)
            .collect();
        for index in indices {
            structure.residues[index].secondary = kind;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two residues of a real backbone, plus a ligand and a water, with the columns exactly
    // where the format says they are.
    const SAMPLE: &str = "\
HEADER    TEST STRUCTURE
TITLE     A SMALL TEST
HELIX    1   1 ALA A    1  ALA A    2  1                                   2
ATOM      1  N   MET A   1      27.340  24.430   2.614  1.00 45.20           N
ATOM      2  CA  MET A   1      26.266  25.413   2.842  1.00 43.11           C
ATOM      3  C   MET A   1      26.913  26.639   3.531  1.00 41.90           C
ATOM      4  O   MET A   1      27.886  26.463   4.263  1.00 40.02           O
ATOM      5  CB  MET A   1      25.112  24.880   3.649  1.00 44.50           C
ATOM      6  N   ALA A   2      26.335  27.770   3.258  1.00 39.10           N
ATOM      7  CA  ALA A   2      26.850  29.021   3.898  1.00 38.44           C
HETATM    8 CA    CA B 101      10.000  10.000  10.000  1.00 20.00          CA
HETATM    9  O   HOH C 201       5.000   5.000   5.000  1.00 30.00           O
END
";

    #[test]
    fn reads_atoms_residues_and_chains() {
        let structure = parse(SAMPLE, "test.pdb");
        assert_eq!(structure.atom_count(), 9);
        assert_eq!(structure.chains.len(), 3);
        assert_eq!(structure.residues.len(), 4);
        assert_eq!(structure.title, "A SMALL TEST");
        assert_eq!(structure.position(0), [27.340, 24.430, 2.614]);
        assert_eq!(structure.b_factors[0], 45.20);
    }

    #[test]
    fn calcium_is_not_read_as_carbon() {
        // The single most common element-guessing bug in PDB readers.
        let structure = parse(SAMPLE, "test.pdb");

        // Atom 8 is the HETATM whose name starts in column 13: calcium.
        assert_eq!(structure.atom_names[7], "CA");
        assert_eq!(
            structure.residues[structure.residue_of_atom[7] as usize].name,
            "CA"
        );
        assert_eq!(structure.elements[7], 20, "CA in residue CA is calcium");

        // Atom 2 is MET's alpha carbon, whose name is indented: still carbon.
        assert_eq!(structure.atom_names[1], "CA");
        assert_eq!(structure.elements[1], chem::CARBON);
    }

    #[test]
    fn helix_records_reach_the_residues() {
        let structure = parse(SAMPLE, "test.pdb");
        assert_eq!(structure.residues[0].secondary, SecondaryStructure::Helix);
        assert_eq!(structure.residues[1].secondary, SecondaryStructure::Helix);
        // The ligand is not in the helix range.
        assert_eq!(structure.residues[2].secondary, SecondaryStructure::Coil);
    }

    #[test]
    fn only_the_first_model_is_loaded() {
        let ensemble = format!(
            "MODEL        1\n{}\nENDMDL\nMODEL        2\n{}\nENDMDL\n",
            "ATOM      1  N   MET A   1      27.340  24.430   2.614  1.00 45.20           N",
            "ATOM      1  N   MET A   1      99.000  99.000  99.000  1.00 45.20           N"
        );
        let structure = parse(&ensemble, "ensemble.pdb");
        assert_eq!(structure.atom_count(), 1);
        assert_eq!(structure.position(0)[0], 27.340);
    }

    #[test]
    fn alternate_conformers_are_not_loaded_twice() {
        let alt = "\
ATOM      1  CB ASER A   1      27.340  24.430   2.614  0.50 45.20           C
ATOM      2  CB BSER A   1      27.900  24.900   2.100  0.50 45.20           C
";
        let structure = parse(alt, "alt.pdb");
        assert_eq!(structure.atom_count(), 1, "only conformer A is kept");
    }

    #[test]
    fn negative_coordinates_and_residue_numbers_parse() {
        // Fields that run together are why this parser slices columns instead of
        // splitting on whitespace.
        let line = "ATOM      1  N   MET A  -1     -27.340 -24.430  -2.614  1.00 45.20           N";
        let structure = parse(line, "neg.pdb");
        assert_eq!(structure.atom_count(), 1);
        assert_eq!(structure.residues[0].number, -1);
        assert_eq!(structure.position(0), [-27.340, -24.430, -2.614]);
    }

    #[test]
    fn files_without_an_element_column_still_work() {
        let old_style = "\
ATOM      1  N   MET A   1      27.340  24.430   2.614  1.00 45.20
ATOM      2  SG  CYS A   2      26.266  25.413   2.842  1.00 43.11
";
        let structure = parse(old_style, "old.pdb");
        assert_eq!(structure.elements[0], chem::NITROGEN);
        assert_eq!(structure.elements[1], chem::SULFUR);
    }

    #[test]
    fn a_missing_chain_id_gets_one() {
        // Some tools omit the chain column entirely; an empty chain id would produce a
        // chain nothing can be selected by.
        let line = "ATOM      1  N   MET     1      27.340  24.430   2.614  1.00 45.20           N";
        let structure = parse(line, "nochain.pdb");
        assert_eq!(structure.chains[0].id, "A_1");
        assert_eq!(structure.chains[0].auth_id, "A");
    }

    #[test]
    fn an_element_the_table_lacks_is_unknown_not_a_guess() {
        // Holmium's neighbour dysprosium: named in the element column, absent from the
        // table. Guessed from the atom name's first letter it would be deuterium - a
        // hydrogen.
        let text = "\
HETATM    1 DY    DY A 301      10.000  10.000  10.000  1.00 20.00          DY
ATOM      2  N   ALA A   1       1.000   1.000   1.000  1.00 20.00           N
";
        let structure = parse(text, "dy.pdb");
        assert_eq!(structure.elements[0], chem::UNKNOWN);
        assert_ne!(structure.elements[0], chem::HYDROGEN);
        assert_eq!(structure.elements[1], chem::NITROGEN);
    }

    #[test]
    fn a_residue_keeps_its_first_conformer_whatever_it_is_called() {
        // Conformers B and C, and no A. Keeping only A would drop the side chain.
        let text = "\
ATOM      1  N   SER A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  SER A   1       1.470   0.000   0.000  1.00 20.00           C
ATOM      3  OG BSER A   1       2.000   1.400   0.000  0.60 20.00           O
ATOM      4  OG CSER A   1       2.000  -1.400   0.000  0.40 20.00           O
ATOM      5  N   ALA A   2       4.000   0.000   0.000  1.00 20.00           N
ATOM      6  CB AALA A   2       5.000   1.000   0.000  0.50 20.00           C
ATOM      7  CB BALA A   2       5.000  -1.000   0.000  0.50 20.00           C
";
        let structure = parse(text, "alt.pdb");
        let names: Vec<&str> = structure
            .atom_names
            .iter()
            .map(|name| name.trim())
            .collect();
        assert_eq!(names, vec!["N", "CA", "OG", "N", "CB"]);
        // SER kept B, the first it named; ALA kept A.
        assert_eq!(structure.position(2), [2.0, 1.4, 0.0]);
        assert_eq!(structure.position(4), [5.0, 1.0, 0.0]);
    }

    #[test]
    fn short_and_truncated_lines_are_skipped_not_a_panic() {
        // Every prefix of a real ATOM line, plus records cut off after their name.
        let full = "ATOM      1  N   MET A   1      27.340  24.430   2.614  1.00 45.20           N";
        let mut text = String::new();
        for end in 0..=full.len() {
            text.push_str(&full[..end]);
            text.push('\n');
        }
        text.push_str("HETATM\nHELIX\nSHEET\nTITLE\nEXPDTA\nMODEL\n");
        let structure = parse(&text, "short.pdb");
        // Only the prefixes that reach the z column carry a position.
        assert!(structure.atom_count() > 0);
        assert!(structure.positions.iter().all(|value| value.is_finite()));
        assert!(parse("", "empty.pdb").is_empty());
    }

    #[test]
    fn a_line_cut_after_the_coordinates_keeps_its_atom_with_defaults() {
        let line = "ATOM      1  N   MET A   1      27.340  24.430   2.614";
        let structure = parse(line, "cut.pdb");
        assert_eq!(structure.atom_count(), 1);
        assert_eq!(
            structure.elements[0],
            chem::NITROGEN,
            "guessed from the name"
        );
        assert_eq!(structure.b_factors[0], 0.0);
    }

    #[test]
    fn non_finite_coordinates_are_not_atoms() {
        let text = "\
ATOM      1  N   MET A   1           nan  24.430   2.614  1.00 45.20           N
ATOM      2  CA  MET A   1      26.266       inf   2.842  1.00 43.11           C
ATOM      3  C   MET A   1      26.913  26.639  -inf     1.00 41.90           C
ATOM      4  O   MET A   1      27.886  26.463   4.263  1.00 40.02           O
";
        let structure = parse(text, "nan.pdb");
        assert_eq!(structure.atom_count(), 1);
        assert_eq!(structure.atom_names[0], "O");
        let (min, max) = structure.bounds();
        assert!(min.iter().chain(max.iter()).all(|value| value.is_finite()));
    }

    #[test]
    fn crlf_line_endings_read_the_same() {
        let crlf = SAMPLE.replace('\n', "\r\n");
        let structure = parse(&crlf, "crlf.pdb");
        assert_eq!(structure.atom_count(), 9);
        assert_eq!(structure.title, "A SMALL TEST");
        // The element column sits at the end of the line, next to the CR.
        assert_eq!(structure.elements[7], 20);
        assert_eq!(structure.residues[0].secondary, SecondaryStructure::Helix);
    }

    #[test]
    fn non_ascii_bytes_in_a_line_do_not_panic() {
        // Lossy decoding turns a stray Latin-1 byte into a three-byte replacement
        // character, which moves every column after it.
        let text = "\
REMARK   1 5 \u{c5} RESOLUTION
ATOM      1 \u{fffd}N   MET A   1      27.340  24.430   2.614  1.00 45.20           N
ATOM      2  CA  MET A   1      26.266  25.413   2.842  1.00 43.11           C
";
        let structure = parse(text, "latin.pdb");
        assert!(structure.atom_count() >= 1);
        assert_eq!(structure.atom_names.last().map(String::as_str), Some("CA"));
    }

    #[test]
    fn residue_numbers_past_9999_are_read_as_hybrid_36() {
        assert_eq!(residue_number("9999"), Some(9999));
        assert_eq!(residue_number("A000"), Some(10000));
        assert_eq!(residue_number("A001"), Some(10001));
        assert_eq!(residue_number("ZZZZ"), Some(1_223_055));
        assert_eq!(residue_number("a000"), Some(1_223_056));
        assert_eq!(residue_number("-999"), Some(-999));
        assert_eq!(residue_number("A0a0"), None, "mixed case is not hybrid-36");
        assert_eq!(residue_number("xyz"), None);

        // Waters past 9999: each its own residue, not one merged blob.
        let text = "\
HETATM    1  O   HOH W9999      10.000  10.000  10.000  1.00 20.00           O
HETATM    2  O   HOH WA000      13.000  10.000  10.000  1.00 20.00           O
HETATM    3  O   HOH WA001      16.000  10.000  10.000  1.00 20.00           O
";
        let structure = parse(text, "waters.pdb");
        let numbers: Vec<i32> = structure
            .residues
            .iter()
            .map(|residue| residue.number)
            .collect();
        assert_eq!(numbers, vec![9999, 10000, 10001]);
    }

    #[test]
    fn insertion_codes_and_negative_numbers_keep_residues_apart() {
        let text = "\
ATOM      1  CA  SER A  -1       0.000   0.000   0.000  1.00 20.00           C
ATOM      2  CA  SER A   0       3.800   0.000   0.000  1.00 20.00           C
ATOM      3  CA  GLY A 100       7.600   0.000   0.000  1.00 20.00           C
ATOM      4  CA  GLY A 100A     11.400   0.000   0.000  1.00 20.00           C
ATOM      5  CA  GLY A 100B     15.200   0.000   0.000  1.00 20.00           C
";
        let structure = parse(text, "icode.pdb");
        let ids: Vec<(i32, Option<char>)> = structure
            .residues
            .iter()
            .map(|residue| (residue.number, residue.insertion_code))
            .collect();
        assert_eq!(
            ids,
            vec![
                (-1, None),
                (0, None),
                (100, None),
                (100, Some('A')),
                (100, Some('B'))
            ]
        );
    }

    #[test]
    fn a_file_of_waters_only_is_one_water_chain() {
        let text = "\
HETATM    1  O   HOH A   1      10.000  10.000  10.000  1.00 20.00           O
HETATM    2  O   HOH A   2      13.000  10.000  10.000  1.00 20.00           O
";
        let structure = parse(text, "water.pdb");
        assert_eq!(structure.residues.len(), 2);
        assert_eq!(structure.chains.len(), 1);
        assert_eq!(structure.chains[0].entity_type, "water");
        assert!(structure
            .flags
            .iter()
            .all(|flag| flag & crate::structure::model::flags::WATER != 0));
    }

    #[test]
    fn a_neutron_structures_deuteriums_are_hydrogens() {
        let text = "\
ATOM      1  N   ALA A   1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  D   ALA A   1       0.000   1.000   0.000  1.00 20.00           D
";
        let structure = parse(text, "neutron.pdb");
        assert_eq!(structure.elements[1], chem::HYDROGEN);
        assert!(structure.flags[1] & crate::structure::model::flags::HYDROGEN != 0);
    }

    #[test]
    fn without_an_element_column_a_glutamine_amide_hydrogen_is_not_helium() {
        // HE21 fills columns 13-16, as every four-character hydrogen name does.
        let text = "\
ATOM      1  NE2 GLN A   1       0.000   0.000   0.000  1.00 20.00
ATOM      2 HE21 GLN A   1       1.000   0.000   0.000  1.00 20.00
";
        let structure = parse(text, "gln.pdb");
        assert_eq!(structure.atom_names[1], "HE21");
        assert_eq!(structure.elements[1], chem::HYDROGEN);
    }

    #[test]
    fn a_file_with_no_chain_column_still_gets_its_helices() {
        let text = "\
HELIX    1   1 ALA      1  ALA      2  1                                   2
ATOM      1  N   ALA     1       0.000   0.000   0.000  1.00 20.00           N
ATOM      2  CA  ALA     1       1.470   0.000   0.000  1.00 20.00           C
ATOM      3  N   ALA     2       4.000   0.000   0.000  1.00 20.00           N
ATOM      4  CA  ALA     2       5.470   0.000   0.000  1.00 20.00           C
";
        let structure = parse(text, "nochain.pdb");
        assert!(structure
            .residues
            .iter()
            .all(|residue| residue.secondary == SecondaryStructure::Helix));
    }
}
