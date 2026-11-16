//! Element data: radii and colours.
//!
//! Radii come from the values molecular viewers have converged on (Bondi van der Waals,
//! Cordero covalent). They matter twice over: van der Waals radii are what a spacefill
//! representation draws, and covalent radii are what bond inference measures against, so
//! a wrong number here shows up as either a lumpy surface or a missing bond.

/// Atomic number, used as the compact per-atom element key in the wire format.
pub type Element = u8;

pub const UNKNOWN: Element = 0;
pub const HYDROGEN: Element = 1;
pub const CARBON: Element = 6;
pub const NITROGEN: Element = 7;
pub const OXYGEN: Element = 8;
pub const SULFUR: Element = 16;

struct ElementData {
    symbol: &'static str,
    /// Bondi van der Waals radius in angstroms.
    vdw: f32,
    /// Cordero covalent radius in angstroms.
    covalent: f32,
    /// CPK-style colour, the convention every structural biologist reads by reflex.
    color: [u8; 3],
}

/// Indexed by atomic number. Gaps are filled with a neutral default rather than left
/// blank: an unrecognised atom must still be drawn somewhere sensible.
const DEFAULT: ElementData = ElementData {
    symbol: "X",
    vdw: 1.7,
    covalent: 0.75,
    color: [255, 20, 147],
};

const TABLE: &[(Element, ElementData)] = &[
    (
        1,
        ElementData {
            symbol: "H",
            vdw: 1.20,
            covalent: 0.31,
            color: [255, 255, 255],
        },
    ),
    (
        2,
        ElementData {
            symbol: "He",
            vdw: 1.40,
            covalent: 0.28,
            color: [217, 255, 255],
        },
    ),
    (
        3,
        ElementData {
            symbol: "Li",
            vdw: 1.82,
            covalent: 1.28,
            color: [204, 128, 255],
        },
    ),
    (
        5,
        ElementData {
            symbol: "B",
            vdw: 1.92,
            covalent: 0.84,
            color: [255, 181, 181],
        },
    ),
    (
        6,
        ElementData {
            symbol: "C",
            vdw: 1.70,
            covalent: 0.76,
            color: [144, 154, 166],
        },
    ),
    (
        7,
        ElementData {
            symbol: "N",
            vdw: 1.55,
            covalent: 0.71,
            color: [48, 80, 248],
        },
    ),
    (
        8,
        ElementData {
            symbol: "O",
            vdw: 1.52,
            covalent: 0.66,
            color: [255, 13, 13],
        },
    ),
    (
        9,
        ElementData {
            symbol: "F",
            vdw: 1.47,
            covalent: 0.57,
            color: [144, 224, 80],
        },
    ),
    (
        11,
        ElementData {
            symbol: "Na",
            vdw: 2.27,
            covalent: 1.66,
            color: [171, 92, 242],
        },
    ),
    (
        12,
        ElementData {
            symbol: "Mg",
            vdw: 1.73,
            covalent: 1.41,
            color: [138, 255, 0],
        },
    ),
    (
        13,
        ElementData {
            symbol: "Al",
            vdw: 1.84,
            covalent: 1.21,
            color: [191, 166, 166],
        },
    ),
    (
        14,
        ElementData {
            symbol: "Si",
            vdw: 2.10,
            covalent: 1.11,
            color: [240, 200, 160],
        },
    ),
    (
        15,
        ElementData {
            symbol: "P",
            vdw: 1.80,
            covalent: 1.07,
            color: [255, 128, 0],
        },
    ),
    (
        16,
        ElementData {
            symbol: "S",
            vdw: 1.80,
            covalent: 1.05,
            color: [255, 255, 48],
        },
    ),
    (
        17,
        ElementData {
            symbol: "Cl",
            vdw: 1.75,
            covalent: 1.02,
            color: [31, 240, 31],
        },
    ),
    (
        19,
        ElementData {
            symbol: "K",
            vdw: 2.75,
            covalent: 2.03,
            color: [143, 64, 212],
        },
    ),
    (
        20,
        ElementData {
            symbol: "Ca",
            vdw: 2.31,
            covalent: 1.76,
            color: [61, 255, 0],
        },
    ),
    (
        24,
        ElementData {
            symbol: "Cr",
            vdw: 2.00,
            covalent: 1.39,
            color: [138, 153, 199],
        },
    ),
    (
        25,
        ElementData {
            symbol: "Mn",
            vdw: 2.00,
            covalent: 1.39,
            color: [156, 122, 199],
        },
    ),
    (
        26,
        ElementData {
            symbol: "Fe",
            vdw: 2.00,
            covalent: 1.32,
            color: [224, 102, 51],
        },
    ),
    (
        27,
        ElementData {
            symbol: "Co",
            vdw: 2.00,
            covalent: 1.26,
            color: [240, 144, 160],
        },
    ),
    (
        28,
        ElementData {
            symbol: "Ni",
            vdw: 1.63,
            covalent: 1.24,
            color: [80, 208, 80],
        },
    ),
    (
        29,
        ElementData {
            symbol: "Cu",
            vdw: 1.40,
            covalent: 1.32,
            color: [200, 128, 51],
        },
    ),
    (
        30,
        ElementData {
            symbol: "Zn",
            vdw: 1.39,
            covalent: 1.22,
            color: [125, 128, 176],
        },
    ),
    (
        34,
        ElementData {
            symbol: "Se",
            vdw: 1.90,
            covalent: 1.20,
            color: [255, 161, 0],
        },
    ),
    (
        35,
        ElementData {
            symbol: "Br",
            vdw: 1.85,
            covalent: 1.20,
            color: [166, 41, 41],
        },
    ),
    (
        42,
        ElementData {
            symbol: "Mo",
            vdw: 2.00,
            covalent: 1.54,
            color: [84, 181, 181],
        },
    ),
    (
        53,
        ElementData {
            symbol: "I",
            vdw: 1.98,
            covalent: 1.39,
            color: [148, 0, 148],
        },
    ),
    (
        79,
        ElementData {
            symbol: "Au",
            vdw: 1.66,
            covalent: 1.36,
            color: [255, 209, 35],
        },
    ),
    // The heavy atoms of soaks, cofactors and crystallisation salts. They are few in any
    // one entry and common across the archive, and an atom the table lacks is drawn as an
    // unknown and bonded to nothing. Radii as the rest: van der Waals after Alvarez,
    // covalent after Cordero; colours are Jmol's. Not in atomic-number order - lookups
    // search, they do not index.
    (
        4,
        ElementData {
            symbol: "Be",
            vdw: 1.53,
            covalent: 0.96,
            color: [194, 255, 0],
        },
    ),
    (
        10,
        ElementData {
            symbol: "Ne",
            vdw: 1.54,
            covalent: 0.58,
            color: [179, 227, 245],
        },
    ),
    (
        18,
        ElementData {
            symbol: "Ar",
            vdw: 1.88,
            covalent: 1.06,
            color: [128, 209, 227],
        },
    ),
    (
        22,
        ElementData {
            symbol: "Ti",
            vdw: 2.11,
            covalent: 1.60,
            color: [191, 194, 199],
        },
    ),
    (
        23,
        ElementData {
            symbol: "V",
            vdw: 2.07,
            covalent: 1.53,
            color: [166, 166, 171],
        },
    ),
    (
        31,
        ElementData {
            symbol: "Ga",
            vdw: 1.87,
            covalent: 1.22,
            color: [194, 143, 143],
        },
    ),
    (
        33,
        ElementData {
            symbol: "As",
            vdw: 1.85,
            covalent: 1.19,
            color: [189, 128, 227],
        },
    ),
    (
        36,
        ElementData {
            symbol: "Kr",
            vdw: 2.02,
            covalent: 1.16,
            color: [92, 184, 209],
        },
    ),
    (
        37,
        ElementData {
            symbol: "Rb",
            vdw: 3.03,
            covalent: 2.20,
            color: [112, 46, 176],
        },
    ),
    (
        38,
        ElementData {
            symbol: "Sr",
            vdw: 2.49,
            covalent: 1.95,
            color: [0, 255, 0],
        },
    ),
    (
        39,
        ElementData {
            symbol: "Y",
            vdw: 2.32,
            covalent: 1.90,
            color: [148, 255, 255],
        },
    ),
    (
        40,
        ElementData {
            symbol: "Zr",
            vdw: 2.23,
            covalent: 1.75,
            color: [148, 224, 224],
        },
    ),
    (
        44,
        ElementData {
            symbol: "Ru",
            vdw: 2.13,
            covalent: 1.46,
            color: [36, 143, 143],
        },
    ),
    (
        45,
        ElementData {
            symbol: "Rh",
            vdw: 2.10,
            covalent: 1.42,
            color: [10, 125, 140],
        },
    ),
    (
        46,
        ElementData {
            symbol: "Pd",
            vdw: 2.10,
            covalent: 1.39,
            color: [0, 105, 133],
        },
    ),
    (
        47,
        ElementData {
            symbol: "Ag",
            vdw: 2.11,
            covalent: 1.45,
            color: [192, 192, 192],
        },
    ),
    (
        48,
        ElementData {
            symbol: "Cd",
            vdw: 2.18,
            covalent: 1.44,
            color: [255, 217, 143],
        },
    ),
    (
        49,
        ElementData {
            symbol: "In",
            vdw: 1.93,
            covalent: 1.42,
            color: [166, 117, 115],
        },
    ),
    (
        50,
        ElementData {
            symbol: "Sn",
            vdw: 2.17,
            covalent: 1.39,
            color: [102, 128, 128],
        },
    ),
    (
        51,
        ElementData {
            symbol: "Sb",
            vdw: 2.06,
            covalent: 1.39,
            color: [158, 99, 181],
        },
    ),
    (
        52,
        ElementData {
            symbol: "Te",
            vdw: 2.06,
            covalent: 1.38,
            color: [212, 122, 0],
        },
    ),
    (
        54,
        ElementData {
            symbol: "Xe",
            vdw: 2.16,
            covalent: 1.40,
            color: [66, 158, 176],
        },
    ),
    (
        55,
        ElementData {
            symbol: "Cs",
            vdw: 3.43,
            covalent: 2.44,
            color: [87, 23, 143],
        },
    ),
    (
        56,
        ElementData {
            symbol: "Ba",
            vdw: 2.68,
            covalent: 2.15,
            color: [0, 201, 0],
        },
    ),
    (
        57,
        ElementData {
            symbol: "La",
            vdw: 2.43,
            covalent: 2.07,
            color: [112, 212, 255],
        },
    ),
    (
        58,
        ElementData {
            symbol: "Ce",
            vdw: 2.42,
            covalent: 2.04,
            color: [255, 255, 199],
        },
    ),
    (
        62,
        ElementData {
            symbol: "Sm",
            vdw: 2.36,
            covalent: 1.98,
            color: [143, 255, 199],
        },
    ),
    (
        63,
        ElementData {
            symbol: "Eu",
            vdw: 2.35,
            covalent: 1.98,
            color: [97, 255, 199],
        },
    ),
    (
        64,
        ElementData {
            symbol: "Gd",
            vdw: 2.34,
            covalent: 1.96,
            color: [69, 255, 199],
        },
    ),
    (
        65,
        ElementData {
            symbol: "Tb",
            vdw: 2.33,
            covalent: 1.94,
            color: [48, 255, 199],
        },
    ),
    (
        67,
        ElementData {
            symbol: "Ho",
            vdw: 2.30,
            covalent: 1.92,
            color: [0, 255, 156],
        },
    ),
    (
        70,
        ElementData {
            symbol: "Yb",
            vdw: 2.26,
            covalent: 1.87,
            color: [0, 191, 56],
        },
    ),
    (
        71,
        ElementData {
            symbol: "Lu",
            vdw: 2.24,
            covalent: 1.87,
            color: [0, 171, 36],
        },
    ),
    (
        74,
        ElementData {
            symbol: "W",
            vdw: 2.18,
            covalent: 1.62,
            color: [33, 148, 214],
        },
    ),
    (
        75,
        ElementData {
            symbol: "Re",
            vdw: 2.16,
            covalent: 1.51,
            color: [38, 125, 171],
        },
    ),
    (
        76,
        ElementData {
            symbol: "Os",
            vdw: 2.16,
            covalent: 1.44,
            color: [38, 102, 150],
        },
    ),
    (
        77,
        ElementData {
            symbol: "Ir",
            vdw: 2.13,
            covalent: 1.41,
            color: [23, 84, 135],
        },
    ),
    (
        78,
        ElementData {
            symbol: "Pt",
            vdw: 2.13,
            covalent: 1.36,
            color: [208, 208, 224],
        },
    ),
    (
        80,
        ElementData {
            symbol: "Hg",
            vdw: 2.23,
            covalent: 1.32,
            color: [184, 184, 208],
        },
    ),
    (
        81,
        ElementData {
            symbol: "Tl",
            vdw: 1.96,
            covalent: 1.45,
            color: [166, 84, 77],
        },
    ),
    (
        82,
        ElementData {
            symbol: "Pb",
            vdw: 2.02,
            covalent: 1.46,
            color: [87, 89, 97],
        },
    ),
    (
        83,
        ElementData {
            symbol: "Bi",
            vdw: 2.07,
            covalent: 1.48,
            color: [158, 79, 181],
        },
    ),
    (
        92,
        ElementData {
            symbol: "U",
            vdw: 1.86,
            covalent: 1.96,
            color: [0, 143, 255],
        },
    ),
];

fn data(element: Element) -> &'static ElementData {
    TABLE
        .iter()
        .find(|(number, _)| *number == element)
        .map(|(_, data)| data)
        .unwrap_or(&DEFAULT)
}

pub fn symbol(element: Element) -> &'static str {
    data(element).symbol
}

pub fn vdw_radius(element: Element) -> f32 {
    data(element).vdw
}

pub fn covalent_radius(element: Element) -> f32 {
    data(element).covalent
}

pub fn color(element: Element) -> [u8; 3] {
    data(element).color
}

/// Parses an element symbol, case-insensitively.
pub fn from_symbol(symbol: &str) -> Element {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return UNKNOWN;
    }
    let normalized: String = symbol
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    // Deuterium and tritium are hydrogen as far as drawing and bonding go. Neutron
    // structures name every exchanged hydrogen `D`, and reading those as unknown drew them
    // as large pink spheres that "hide hydrogens" left on screen.
    if normalized == "D" || normalized == "T" {
        return HYDROGEN;
    }
    TABLE
        .iter()
        .find(|(_, data)| data.symbol.to_ascii_uppercase() == normalized)
        .map(|(number, _)| *number)
        .unwrap_or(UNKNOWN)
}

/// Guesses the element from a PDB atom name when the element column is absent.
///
/// Pre-2000 PDB files routinely omit columns 77-78, and plenty of modelling tools still
/// write files without them. Getting this wrong turns every calcium into a carbon.
///
/// The rule exploits PDB's column convention: a name starting in column 13 (i.e. with a
/// leading space stripped, `name_started_at_13 == false`) belongs to a one-letter element,
/// while a two-letter element like `CA` for calcium starts in column 13 itself.
pub fn from_atom_name(name: &str, starts_in_column_13: bool) -> Element {
    from_atom_name_in(name, starts_in_column_13, "")
}

/// As `from_atom_name`, knowing which residue the atom is in.
///
/// The column rule - a name starting in column 13 is a two-letter element - is only as
/// good as the file's alignment, and plenty of writers left-justify every name. In a
/// standard amino acid or nucleotide there is nothing to guess: every atom is C, N, O, S,
/// P or H, so `CA` is the alpha carbon and never calcium, `CD` never cadmium, `HG` never
/// mercury, however the name is aligned.
pub fn from_atom_name_in(name: &str, starts_in_column_13: bool, residue: &str) -> Element {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return UNKNOWN;
    }
    if is_plain_polymer_residue(residue.trim()) {
        return match trimmed.chars().find(|c| c.is_ascii_alphabetic()) {
            // A leading digit is a hydrogen's (1HB), and so is a leading H.
            _ if trimmed.starts_with(|c: char| c.is_ascii_digit()) => HYDROGEN,
            Some(letter) => from_symbol(&letter.to_string()),
            None => UNKNOWN,
        };
    }
    // Hydrogens are often named 1HB, 2HG1, HD22 - a leading digit means hydrogen.
    let first = trimmed.chars().next().unwrap();
    if first.is_ascii_digit() {
        return HYDROGEN;
    }
    // A four-character hydrogen name - GLN's HE21, ASN's HD22 - fills columns 13 to 16, so
    // it starts in column 13 like a two-letter element would. Read by the two-letter rule
    // HE21 is helium. No two-letter element's atom is named with four characters starting
    // with H, so these are hydrogens.
    if starts_in_column_13 && trimmed.len() == 4 && first.eq_ignore_ascii_case(&'H') {
        return HYDROGEN;
    }
    if starts_in_column_13 && trimmed.len() >= 2 {
        let two: String = trimmed.chars().take(2).collect();
        let element = from_symbol(&two);
        if element != UNKNOWN {
            return element;
        }
    }
    from_symbol(&first.to_string())
}

/// The residues made only of C, N, O, S, P and H: the twenty amino acids and the eight
/// nucleotides. Selenocysteine and selenomethionine are deliberately not here.
fn is_plain_polymer_residue(name: &str) -> bool {
    matches!(
        name,
        "ALA"
            | "ARG"
            | "ASN"
            | "ASP"
            | "CYS"
            | "GLN"
            | "GLU"
            | "GLY"
            | "HIS"
            | "ILE"
            | "LEU"
            | "LYS"
            | "MET"
            | "PHE"
            | "PRO"
            | "SER"
            | "THR"
            | "TRP"
            | "TYR"
            | "VAL"
            | "A"
            | "C"
            | "G"
            | "U"
            | "DA"
            | "DC"
            | "DG"
            | "DT"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standard_residue_s_atoms_are_never_metals() {
        // Left-justified names with no element column: by the column rule alone these are
        // calcium, cadmium and mercury.
        assert_eq!(from_atom_name_in("CA  ", true, "ALA"), CARBON);
        assert_eq!(from_atom_name_in("CD  ", true, "GLU"), CARBON);
        assert_eq!(from_atom_name_in("HG  ", true, "SER"), HYDROGEN);
        assert_eq!(from_atom_name_in("1HB ", true, "ALA"), HYDROGEN);
        assert_eq!(from_atom_name_in("OP1 ", true, "DA"), 8);
        // Outside one, the column still decides: a calcium ion, and selenium in MSE.
        assert_eq!(from_atom_name_in("CA  ", true, "CA"), 20);
        assert_eq!(from_atom_name_in("SE  ", true, "MSE"), 34);
        assert_eq!(from_atom_name_in(" CA ", false, "CA"), CARBON);
    }

    #[test]
    fn heavy_atoms_are_known() {
        for symbol in ["Hg", "Cd", "Pt", "Sr", "Cs", "Gd", "W", "U", "Xe", "As"] {
            let element = from_symbol(symbol);
            assert_ne!(element, UNKNOWN, "{symbol}");
            assert!(
                vdw_radius(element) > covalent_radius(element) * 0.9,
                "{symbol}"
            );
        }
        // No two entries share a number or a symbol.
        for (index, (number, data)) in TABLE.iter().enumerate() {
            for (other, other_data) in &TABLE[index + 1..] {
                assert_ne!(number, other, "{}", data.symbol);
                assert_ne!(data.symbol, other_data.symbol);
            }
        }
    }

    #[test]
    fn symbols_round_trip() {
        assert_eq!(from_symbol("C"), CARBON);
        assert_eq!(from_symbol(" fe "), 26);
        assert_eq!(from_symbol("SE"), 34);
        assert_eq!(from_symbol("Xx"), UNKNOWN);
        assert_eq!(symbol(CARBON), "C");
    }

    #[test]
    fn atom_names_disambiguate_calcium_from_alpha_carbon() {
        // The distinction that a naive parser gets wrong: protein CA is a carbon whose
        // name is indented, calcium CA is not.
        assert_eq!(from_atom_name(" CA ", false), CARBON);
        assert_eq!(from_atom_name("CA", true), 20);
        assert_eq!(from_atom_name(" CB ", false), CARBON);
        assert_eq!(from_atom_name(" N  ", false), NITROGEN);
        assert_eq!(from_atom_name(" SG ", false), SULFUR);
    }

    #[test]
    fn hydrogen_names_starting_with_a_digit_are_hydrogen() {
        assert_eq!(from_atom_name("1HB ", true), HYDROGEN);
        assert_eq!(from_atom_name("2HG1", true), HYDROGEN);
        assert_eq!(from_atom_name(" HD1", false), HYDROGEN);
    }

    #[test]
    fn deuterium_is_drawn_and_hidden_as_hydrogen() {
        // Neutron structures write D in the element column.
        assert_eq!(from_symbol("D"), HYDROGEN);
        assert_eq!(from_symbol(" d"), HYDROGEN);
        assert_eq!(from_atom_name(" D1 ", false), HYDROGEN);
    }

    #[test]
    fn four_character_hydrogen_names_are_not_helium() {
        // GLN HE21 and ASN HD22 start in column 13 because they fill all four columns.
        assert_eq!(from_atom_name("HE21", true), HYDROGEN);
        assert_eq!(from_atom_name("HD22", true), HYDROGEN);
        // A two-letter element named in column 13 is still read as one.
        assert_eq!(from_atom_name("FE  ", true), 26);
        assert_eq!(from_atom_name("CL1 ", true), 17);
    }

    #[test]
    fn garbage_symbols_are_unknown_not_a_panic() {
        assert_eq!(from_symbol(""), UNKNOWN);
        assert_eq!(from_symbol("12"), UNKNOWN);
        assert_eq!(from_symbol("Fe2+"), 26);
        assert_eq!(from_symbol("é"), UNKNOWN);
        assert_eq!(from_atom_name("    ", true), UNKNOWN);
        assert_eq!(from_atom_name("é", true), UNKNOWN);
    }

    #[test]
    fn unknown_elements_still_have_usable_geometry() {
        assert!(vdw_radius(UNKNOWN) > 0.0);
        assert!(covalent_radius(200) > 0.0);
    }
}

/// One-letter code for a residue name.
///
/// The twenty standard amino acids, the nucleotides, and `X` for anything else - which
/// covers modified residues, ligands and ions without pretending to know what they are.
/// Here rather than in the frontend because it is chemistry, and the frontend already
/// gets its element colours and radii from this module.
pub fn residue_code(name: &str) -> char {
    match name.trim().to_ascii_uppercase().as_str() {
        "ALA" => 'A',
        "ARG" => 'R',
        "ASN" => 'N',
        "ASP" => 'D',
        "CYS" => 'C',
        "GLN" => 'Q',
        "GLU" => 'E',
        "GLY" => 'G',
        "HIS" => 'H',
        "ILE" => 'I',
        "LEU" => 'L',
        "LYS" => 'K',
        "MET" => 'M',
        "PHE" => 'F',
        "PRO" => 'P',
        "SER" => 'S',
        "THR" => 'T',
        "TRP" => 'W',
        "TYR" => 'Y',
        "VAL" => 'V',
        // Common non-standard residues that stand in for a standard one.
        "MSE" => 'M',
        "SEC" => 'U',
        "PYL" => 'O',
        // Nucleotides, in both the PDB's one-letter and older three-letter spellings.
        "A" | "DA" | "ADE" => 'a',
        "C" | "DC" | "CYT" => 'c',
        "G" | "DG" | "GUA" => 'g',
        "T" | "DT" | "THY" => 't',
        "U" | "DU" | "URA" => 'u',
        _ => 'X',
    }
}

#[cfg(test)]
mod code_tests {
    use super::residue_code;

    #[test]
    fn the_standard_twenty_are_all_distinct() {
        let names = [
            "ALA", "ARG", "ASN", "ASP", "CYS", "GLN", "GLU", "GLY", "HIS", "ILE", "LEU", "LYS",
            "MET", "PHE", "PRO", "SER", "THR", "TRP", "TYR", "VAL",
        ];
        let codes: std::collections::BTreeSet<char> =
            names.iter().map(|name| residue_code(name)).collect();
        assert_eq!(codes.len(), 20, "{codes:?}");
        assert!(!codes.contains(&'X'), "none of them is unknown");
    }

    #[test]
    fn selenomethionine_reads_as_methionine() {
        // MSE is how methionine appears in most crystal structures; showing it as X would
        // put a hole in every sequence.
        assert_eq!(residue_code("MSE"), 'M');
    }

    #[test]
    fn a_ligand_is_unknown_and_case_does_not_matter() {
        assert_eq!(residue_code("HEM"), 'X');
        assert_eq!(residue_code("hoh"), 'X');
        assert_eq!(residue_code(" ala "), 'A');
    }
}
