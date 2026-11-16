//! Colour schemes.
//!
//! These are conventions, not decoration. A structural biologist reads CPK colouring and
//! the AlphaFold confidence bands by reflex, so both are reproduced exactly rather than
//! reinterpreted.

use super::palette::{self, Categorical};
use crate::structure::model::{ResidueKind, SecondaryStructure};
use crate::structure::Structure;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorScheme {
    Element,
    /// One colour per chain, from the categorical palette - PyMOL's `util.cbc`.
    Chain,
    ResidueKind,
    Secondary,
    /// pLDDT for a predicted model, B-factor for an experimental one.
    Confidence,
    /// A ramp over the B-factor column, PyMOL's `spectrum b`.
    ///
    /// Separate from `Confidence`, which reads the same column but paints a predicted
    /// model in AlphaFold's bands. This one is the crystallographer's question - which
    /// parts of this structure are poorly ordered - and it wants a ramp over the range the
    /// structure actually has.
    BFactor,
    /// A ramp along the sequence, N to C, over the whole molecule.
    ///
    /// PyMOL's `spectrum count, rainbow, byres=1`: one sweep across everything, so a
    /// complex reads as one chain after another rather than as four sweeps.
    Rainbow,
    /// The same ramp, restarted for each chain - PyMOL's `util.chainbow`.
    ///
    /// A whole-molecule scheme by nature: every chain runs blue to red, so what the colour
    /// tells you is where in *its own* chain a residue is.
    Chainbow,
}

impl ColorScheme {
    pub fn label(self) -> &'static str {
        match self {
            ColorScheme::Element => "Element (CPK)",
            ColorScheme::Chain => "Chain",
            ColorScheme::ResidueKind => "Residue type",
            ColorScheme::Secondary => "Secondary structure",
            ColorScheme::Confidence => "Confidence / B-factor",
            ColorScheme::BFactor => "B-factor (spectrum)",
            ColorScheme::Rainbow => "Rainbow (N to C)",
            ColorScheme::Chainbow => "Chainbow (per chain)",
        }
    }

    /// Whether the scheme is a ramp, and so has a palette to choose.
    pub fn is_ramp(self) -> bool {
        matches!(
            self,
            ColorScheme::BFactor | ColorScheme::Rainbow | ColorScheme::Chainbow
        )
    }

    /// Whether the scheme only means anything computed over the whole molecule.
    ///
    /// Chainbow needs every chain to know where its own ends are, and a colour per chain
    /// needs the chain list to hand out colours from - so both are set on the molecule
    /// group and shared by its chains, rather than being a per-chain decision. A chain
    /// styled `Chainbow` on its own would just be a rainbow.
    pub fn is_molecule_wide(self) -> bool {
        matches!(
            self,
            ColorScheme::Chain | ColorScheme::Chainbow | ColorScheme::Rainbow
        )
    }
}

/// Which ramp a continuous scheme uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ramp {
    /// Blue to red through cyan, green and yellow. PyMOL's default palette, and what
    /// "chainbow" means to anyone who has used it.
    #[default]
    Rainbow,
    /// Perceptually uniform, and readable in greyscale. The honest choice for a quantity.
    Viridis,
    /// Blue to white to red, which is what a B-factor picture usually looks like.
    BlueWhiteRed,
}

impl Ramp {
    pub fn label(self) -> &'static str {
        match self {
            Ramp::Rainbow => "Rainbow",
            Ramp::Viridis => "Viridis",
            Ramp::BlueWhiteRed => "Blue-white-red",
        }
    }

    pub fn sample(self, t: f32) -> [u8; 3] {
        match self {
            Ramp::Rainbow => palette::rainbow(t),
            Ramp::Viridis => palette::viridis(t),
            Ramp::BlueWhiteRed => palette::blue_white_red(t),
        }
    }
}

/// One row of the legend the UI shows beside the viewport.
#[derive(Debug, Clone, Serialize)]
pub struct LegendEntry {
    pub color: [u8; 3],
    pub label: String,
}

/// AlphaFold's own pLDDT bands, in AlphaFold's own colours.
///
/// Reusing them exactly is the point: someone who has seen a model on the AlphaFold site
/// must be able to compare the two views without recalibrating their eye. These are the
/// one set of colours in the app that the palette settings do not touch.
const CONFIDENCE_BANDS: &[(f32, [u8; 3], &str)] = &[
    (90.0, [0, 83, 214], "Very high (>90)"),
    (70.0, [101, 203, 243], "Confident (70-90)"),
    (50.0, [255, 219, 70], "Low (50-70)"),
    (f32::NEG_INFINITY, [255, 125, 69], "Very low (<50)"),
];

/// Everything colour depends on besides the structure itself.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ColorContext {
    /// The background the result will be seen against, in linear 0..1.
    ///
    /// Carried here because CPK's light carbon and white hydrogen disappear on a white
    /// background, and a scheme that ignores that produces an invisible molecule.
    #[serde(default = "white")]
    pub background: [f32; 3],
    #[serde(default)]
    pub categorical: Categorical,
    /// Use viridis for continuous values instead of the AlphaFold bands. Sensible for a
    /// crystal structure's real B-factors, which are not confidences.
    #[serde(default)]
    pub continuous: bool,
    /// Which ramp the continuous schemes use.
    #[serde(default)]
    pub ramp: Ramp,
}

fn white() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

impl Default for ColorContext {
    fn default() -> Self {
        Self {
            background: white(),
            categorical: Categorical::default(),
            continuous: false,
            ramp: Ramp::default(),
        }
    }
}

impl ColorContext {
    fn light(&self) -> bool {
        palette::is_light(self.background)
    }

    fn coil(&self) -> [u8; 3] {
        if self.light() {
            palette::COIL_ON_LIGHT
        } else {
            palette::COIL_ON_DARK
        }
    }

    fn solvent(&self) -> [u8; 3] {
        if self.light() {
            palette::SOLVENT_ON_LIGHT
        } else {
            palette::SOLVENT_ON_DARK
        }
    }

    fn secondary(&self, secondary: SecondaryStructure) -> [u8; 3] {
        let (helix, sheet, turn) = palette::secondary_colors();
        match secondary {
            SecondaryStructure::Helix => helix,
            SecondaryStructure::Sheet => sheet,
            SecondaryStructure::Turn => turn,
            SecondaryStructure::Coil => self.coil(),
        }
    }
}

/// Residue-type colours, from the categorical palette so they match the chain colours in
/// character rather than clashing with them.
fn residue_kind_color(kind: ResidueKind, context: &ColorContext) -> [u8; 3] {
    match kind {
        ResidueKind::AminoAcid => context.categorical.pick(0),
        ResidueKind::Nucleotide => context.categorical.pick(1),
        ResidueKind::Water => context.solvent(),
        ResidueKind::Ligand => context.categorical.pick(3),
    }
}

/// Per-atom colours in 0..1, ready to bake into instance attributes and mesh vertices.
pub fn atom_colors(
    structure: &Structure,
    scheme: ColorScheme,
    context: &ColorContext,
) -> Vec<[f32; 3]> {
    let count = structure.atom_count();
    let mut colors = Vec::with_capacity(count);
    let light = context.light();
    let plddt = looks_like_plddt(structure);
    let (min, max) = b_factor_range(structure);
    let span = (max - min).max(1e-6);
    // Where each residue sits along its sweep, 0 at the N terminus and 1 at the C. Built
    // once, because it is a property of the structure rather than of an atom.
    let sequence_position = match scheme {
        ColorScheme::Rainbow => Some(sequence_positions(structure, false)),
        ColorScheme::Chainbow => Some(sequence_positions(structure, true)),
        _ => None,
    };

    for atom in 0..count {
        let residue_index = structure.residue_of_atom.get(atom).copied();
        let residue = residue_index.and_then(|index| structure.residues.get(index as usize));

        let rgb = match scheme {
            ColorScheme::Element => palette::element_color(structure.elements[atom], light),
            ColorScheme::Chain => match residue {
                Some(residue) if residue.kind == ResidueKind::Water => context.solvent(),
                Some(residue) => context.categorical.pick(residue.chain_index as usize),
                None => context.solvent(),
            },
            ColorScheme::ResidueKind => {
                residue_kind_color(residue.map_or(ResidueKind::Ligand, |r| r.kind), context)
            }
            ColorScheme::Secondary => match residue {
                Some(residue) if residue.kind == ResidueKind::Water => context.solvent(),
                // A nucleic acid has no helices and sheets to colour by. What it has in
                // their place is which base each nucleotide carries, and that is what this
                // scheme shows for one - otherwise a strand is coil grey from end to end.
                Some(residue) if residue.kind == ResidueKind::Nucleotide => {
                    palette::base_color(&residue.name).unwrap_or_else(|| context.coil())
                }
                Some(residue) => context.secondary(residue.secondary),
                None => context.coil(),
            },
            ColorScheme::Confidence => {
                let raw = structure.b_factors[atom];
                if context.continuous || !plddt {
                    // Not a confidence: a crystal structure's B-factors are a different
                    // quantity on a different scale, and reading them as pLDDT would paint
                    // an ordinary structure as garbage. Viridis instead, normalised.
                    palette::viridis((raw - min) / span)
                } else {
                    CONFIDENCE_BANDS
                        .iter()
                        .find(|(threshold, _, _)| raw >= *threshold)
                        .map(|(_, color, _)| *color)
                        .unwrap_or([200, 200, 200])
                }
            }
            ColorScheme::BFactor => context
                .ramp
                .sample((structure.b_factors[atom] - min) / span),
            ColorScheme::Rainbow | ColorScheme::Chainbow => match residue {
                // Waters are not part of the sequence, and painting them along it would
                // put a red water beside a blue N terminus for no reason.
                Some(residue) if residue.kind == ResidueKind::Water => context.solvent(),
                _ => {
                    let position = residue_index
                        .and_then(|index| {
                            sequence_position
                                .as_ref()
                                .and_then(|table| table.get(index as usize))
                                .copied()
                        })
                        .unwrap_or(1.0);
                    context.ramp.sample(position)
                }
            },
        };

        colors.push(to_linear(rgb));
    }
    colors
}

/// Where each residue sits along its sweep: 0 at the start, 1 at the end.
///
/// `per_chain` restarts the sweep for every chain, which is the whole difference between
/// PyMOL's `rainbow` and its `chainbow`. Only polymer residues are counted - a chain with
/// forty waters after it should not spend half of its ramp on them - and anything else
/// (a ligand, a modified residue with no polymer kind) takes the end of its chain's
/// sweep, so a heteroatom is coloured like the chain it belongs to rather than like the
/// start of a different one.
fn sequence_positions(structure: &Structure, per_chain: bool) -> Vec<f32> {
    let polymer =
        |kind: ResidueKind| matches!(kind, ResidueKind::AminoAcid | ResidueKind::Nucleotide);

    // How many polymer residues each sweep covers, and where each residue falls in it.
    let mut ordinal = vec![0u32; structure.residues.len()];
    let mut totals: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for (index, residue) in structure.residues.iter().enumerate() {
        if !polymer(residue.kind) {
            continue;
        }
        let key = if per_chain { residue.chain_index } else { 0 };
        let seen = totals.entry(key).or_insert(0);
        ordinal[index] = *seen;
        *seen += 1;
    }

    structure
        .residues
        .iter()
        .enumerate()
        .map(|(index, residue)| {
            let key = if per_chain { residue.chain_index } else { 0 };
            let total = totals.get(&key).copied().unwrap_or(0);
            if !polymer(residue.kind) {
                return 1.0;
            }
            if total <= 1 {
                // A single residue has no sweep to be part of; the start of the ramp is a
                // better answer than dividing by zero.
                return 0.0;
            }
            ordinal[index] as f32 / (total - 1) as f32
        })
        .collect()
}

/// Converts an sRGB byte triple to linear 0..1.
///
/// Palettes are authored in sRGB, because that is what a hex code means, but light does
/// not add up in sRGB - shading in it makes everything muddy and two lights never sum
/// correctly. Both renderers therefore receive linear colour and encode back to sRGB at
/// the very end: Blender treats vertex colours as linear already, and the viewport's
/// composite pass does the encode itself. Skipping this is why a render can come out
/// noticeably darker and more saturated than the palette it was given.
pub fn to_linear(srgb: [u8; 3]) -> [f32; 3] {
    let channel = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    [channel(srgb[0]), channel(srgb[1]), channel(srgb[2])]
}

/// Legend rows for whichever scheme is showing.
pub fn legend(
    structure: &Structure,
    scheme: ColorScheme,
    context: &ColorContext,
) -> Vec<LegendEntry> {
    let entry = |color: [u8; 3], label: &str| LegendEntry {
        color,
        label: label.to_string(),
    };

    match scheme {
        ColorScheme::Confidence => {
            if context.continuous || !looks_like_plddt(structure) {
                let (min, max) = b_factor_range(structure);
                // A continuous scale needs its ends labelled with the values, not with
                // band names that would be a lie about what is being shown.
                (0..5)
                    .map(|step| {
                        let t = step as f32 / 4.0;
                        entry(
                            palette::viridis(t),
                            &format!("{:.0}", min + (max - min) * t),
                        )
                    })
                    .collect()
            } else {
                CONFIDENCE_BANDS
                    .iter()
                    .map(|(_, color, label)| entry(*color, label))
                    .collect()
            }
        }
        ColorScheme::Chain => structure
            .chains
            .iter()
            .enumerate()
            .map(|(index, chain)| {
                entry(
                    context.categorical.pick(index),
                    &format!("Chain {}", chain.id),
                )
            })
            .collect(),
        ColorScheme::ResidueKind => [
            (ResidueKind::AminoAcid, "Amino acid"),
            (ResidueKind::Nucleotide, "Nucleotide"),
            (ResidueKind::Water, "Water"),
            (ResidueKind::Ligand, "Ligand"),
        ]
        .iter()
        .map(|(kind, label)| entry(residue_kind_color(*kind, context), label))
        .collect(),
        ColorScheme::Secondary => [
            (SecondaryStructure::Coil, "Coil"),
            (SecondaryStructure::Helix, "Helix"),
            (SecondaryStructure::Sheet, "Sheet"),
            (SecondaryStructure::Turn, "Turn"),
        ]
        .iter()
        .map(|(secondary, label)| entry(context.secondary(*secondary), label))
        .collect(),
        // Only the elements actually present, most abundant first. A full periodic table
        // would be noise.
        ColorScheme::Element => {
            let mut counts = std::collections::BTreeMap::new();
            for element in &structure.elements {
                *counts.entry(*element).or_insert(0usize) += 1;
            }
            let mut ordered: Vec<_> = counts.into_iter().collect();
            ordered.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
            ordered
                .into_iter()
                .take(8)
                .map(|(element, count)| {
                    entry(
                        palette::element_color(element, context.light()),
                        &format!("{} ({count})", crate::structure::chem::symbol(element)),
                    )
                })
                .collect()
        }
        // A ramp's legend is its ends and the middle, labelled with what the ends mean:
        // a B-factor picture is unreadable without the numbers, and a chainbow is
        // unreadable without knowing which end is the N terminus.
        ColorScheme::BFactor => {
            let (min, max) = b_factor_range(structure);
            (0..5)
                .map(|step| {
                    let t = step as f32 / 4.0;
                    entry(
                        context.ramp.sample(t),
                        &format!("{:.0}", min + (max - min) * t),
                    )
                })
                .collect()
        }
        ColorScheme::Rainbow | ColorScheme::Chainbow => {
            let ends = if scheme == ColorScheme::Chainbow {
                ["N (each chain)", "C (each chain)"]
            } else {
                ["N terminus", "C terminus"]
            };
            vec![
                entry(context.ramp.sample(0.0), ends[0]),
                entry(context.ramp.sample(0.5), ""),
                entry(context.ramp.sample(1.0), ends[1]),
            ]
        }
    }
}

/// True when the B-factor column plausibly holds pLDDT rather than true B-factors.
///
/// The range alone cannot tell: most crystal structures have B-factors between 0 and 100
/// too, and read as confidence a well-ordered core at B = 15 is "very low". So a file that
/// names the experiment it came from is taken at its word - its column holds B-factors -
/// and the range decides only for a file that names none, which is what a prediction is.
pub fn looks_like_plddt(structure: &Structure) -> bool {
    if let Some(method) = &structure.experimental_method {
        let method = method.to_ascii_uppercase();
        let is_model = ["THEORETICAL", "PREDICT", "COMPUTATIONAL", "MODEL"]
            .iter()
            .any(|word| method.contains(word));
        if !is_model {
            return false;
        }
    }
    let (min, max) = b_factor_range(structure);
    min >= 0.0 && max <= 100.0 && max > 1.0
}

pub fn b_factor_range(structure: &Structure) -> (f32, f32) {
    // Finite values only. One overflowed column value used to make the whole range
    // infinite, which fell back to (0, 0) and painted every other atom the top of the ramp.
    let finite = || {
        structure
            .b_factors
            .iter()
            .copied()
            .filter(|value| value.is_finite())
    };
    let min = finite().fold(f32::INFINITY, f32::min);
    let max = finite().fold(f32::NEG_INFINITY, f32::max);
    if min.is_finite() && max.is_finite() {
        (min, max)
    } else {
        (0.0, 0.0)
    }
}

/// Colour of a residue, taken from the atom the cartoon spline passes through so a ribbon
/// and the atoms it replaces agree.
pub fn residue_color(colors: &[[f32; 3]], structure: &Structure, residue_index: usize) -> [f32; 3] {
    let atom = structure
        .residues
        .get(residue_index)
        .map(|residue| {
            if residue.trace_atom >= 0 {
                residue.trace_atom as usize
            } else {
                residue.first_atom as usize
            }
        })
        .unwrap_or(0);
    colors.get(atom).copied().unwrap_or([0.8, 0.8, 0.8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::model::RawAtom;

    fn structure_with_b_factors(values: &[f32]) -> Structure {
        let atoms: Vec<RawAtom> = values
            .iter()
            .enumerate()
            .map(|(index, b)| RawAtom {
                name: " CA ".into(),
                element: 6,
                position: [index as f32, 0.0, 0.0],
                b_factor: *b,
                occupancy: 1.0,
                residue_name: "ALA".into(),
                residue_number: index as i32 + 1,
                insertion_code: None,
                chain_id: "A".into(),
                is_hetatm: false,
                label_asym_id: String::new(),
                entity_id: String::new(),
            })
            .collect();
        Structure::build("t".into(), "test".into(), atoms)
    }

    /// Undoes `to_linear`, so tests can assert on the palette values they were written
    /// against rather than on linear numbers.
    fn as_255(color: [f32; 3]) -> [u8; 3] {
        let channel = |value: f32| {
            let encoded = if value <= 0.0031308 {
                value * 12.92
            } else {
                1.055 * value.powf(1.0 / 2.4) - 0.055
            };
            (encoded * 255.0).round() as u8
        };
        [channel(color[0]), channel(color[1]), channel(color[2])]
    }

    /// A structure with residues spread over the given chains, one CA each.
    fn structure_with_chains(chains: &[(&str, usize)]) -> Structure {
        let mut atoms: Vec<RawAtom> = Vec::new();
        for (chain, residues) in chains {
            for index in 0..*residues {
                atoms.push(RawAtom {
                    name: " CA ".into(),
                    element: 6,
                    position: [index as f32, 0.0, 0.0],
                    b_factor: 20.0,
                    occupancy: 1.0,
                    residue_name: "ALA".into(),
                    residue_number: index as i32 + 1,
                    insertion_code: None,
                    chain_id: (*chain).into(),
                    is_hetatm: false,
                    label_asym_id: String::new(),
                    entity_id: String::new(),
                });
            }
        }
        Structure::build("t".into(), "test".into(), atoms)
    }

    #[test]
    fn the_rainbow_ramp_runs_blue_to_red_through_green() {
        // PyMOL's `rainbow` palette is blue cyan green yellow orange red, and those are
        // pure hues - so the ends and the middle are exact.
        assert_eq!(Ramp::Rainbow.sample(0.0), [89, 89, 235]);
        assert_eq!(Ramp::Rainbow.sample(0.5), [89, 235, 89]);
        assert_eq!(Ramp::Rainbow.sample(1.0), [235, 89, 89]);
        // A quarter in is cyan, three quarters is yellow.
        assert_eq!(Ramp::Rainbow.sample(0.25), [89, 235, 235]);
        assert_eq!(Ramp::Rainbow.sample(0.75), [235, 235, 89]);
        // Out of range is clamped rather than wrapping back to blue.
        assert_eq!(Ramp::Rainbow.sample(-1.0), [89, 89, 235]);
        assert_eq!(Ramp::Rainbow.sample(2.0), [235, 89, 89]);
    }

    #[test]
    fn chainbow_restarts_the_sweep_for_every_chain() {
        // util.chainbow: each chain runs blue to red on its own, which is what makes the
        // colour say where in *its* chain a residue is.
        let structure = structure_with_chains(&[("A", 5), ("B", 3)]);
        let colors = atom_colors(&structure, ColorScheme::Chainbow, &ColorContext::default());

        assert_eq!(as_255(colors[0]), [89, 89, 235], "chain A starts blue");
        assert_eq!(as_255(colors[4]), [235, 89, 89], "chain A ends red");
        assert_eq!(
            as_255(colors[5]),
            [89, 89, 235],
            "chain B starts blue again"
        );
        assert_eq!(as_255(colors[7]), [235, 89, 89], "chain B ends red");
        // The middle of the shorter chain is the middle of the ramp, not of chain A's.
        assert_eq!(as_255(colors[6]), [89, 235, 89]);
    }

    #[test]
    fn rainbow_is_one_sweep_across_the_whole_molecule() {
        // spectrum count over everything: chain B carries on where chain A stopped, so a
        // complex reads as one chain after another.
        let structure = structure_with_chains(&[("A", 5), ("B", 3)]);
        let colors = atom_colors(&structure, ColorScheme::Rainbow, &ColorContext::default());

        assert_eq!(as_255(colors[0]), [89, 89, 235], "the N terminus is blue");
        assert_eq!(as_255(colors[7]), [235, 89, 89], "the last residue is red");
        assert_ne!(
            as_255(colors[5]),
            [89, 89, 235],
            "chain B does not start over"
        );
        // Eight residues, so residue five sits at 4/7 of the sweep - past green, not on
        // it, which is the difference between one sweep of eight and two of four.
        assert_eq!(as_255(colors[4]), Ramp::Rainbow.sample(4.0 / 7.0));
        // And it runs one way: red rises and blue falls all the way along.
        for pair in colors.windows(2) {
            let (before, after) = (as_255(pair[0]), as_255(pair[1]));
            assert!(after[0] >= before[0], "{before:?} then {after:?}");
            assert!(after[2] <= before[2], "{before:?} then {after:?}");
        }
    }

    #[test]
    fn a_b_factor_spectrum_spans_the_range_the_structure_has() {
        // spectrum b: the ends of the ramp are the lowest and highest B-factor present,
        // not 0 and 100 - a structure whose B-factors run 12 to 40 would otherwise be
        // painted in one colour.
        let structure = structure_with_b_factors(&[12.0, 26.0, 40.0]);
        let colors = atom_colors(&structure, ColorScheme::BFactor, &ColorContext::default());
        assert_eq!(as_255(colors[0]), [89, 89, 235]);
        assert_eq!(as_255(colors[2]), [235, 89, 89]);
        // 26 of 12..40 is halfway, and the ramp is chosen rather than fixed.
        assert_eq!(as_255(colors[1]), [89, 235, 89]);

        let viridis = atom_colors(
            &structure,
            ColorScheme::BFactor,
            &ColorContext {
                ramp: Ramp::Viridis,
                ..ColorContext::default()
            },
        );
        assert_eq!(as_255(viridis[0]), palette::viridis(0.0));
        assert_ne!(as_255(viridis[0]), as_255(colors[0]));
    }

    #[test]
    fn the_whole_molecule_schemes_are_marked_as_such() {
        // The panel uses this to offer them on the molecule group rather than per chain:
        // a lone chain styled Chainbow is just a rainbow.
        assert!(ColorScheme::Chainbow.is_molecule_wide());
        assert!(ColorScheme::Chain.is_molecule_wide());
        assert!(ColorScheme::Rainbow.is_molecule_wide());
        assert!(!ColorScheme::BFactor.is_molecule_wide());
        assert!(!ColorScheme::Secondary.is_molecule_wide());
        // And this one decides whether a ramp picker is shown.
        assert!(ColorScheme::BFactor.is_ramp());
        assert!(ColorScheme::Chainbow.is_ramp());
        assert!(!ColorScheme::Chain.is_ramp());
    }

    #[test]
    fn confidence_uses_alphafolds_own_bands() {
        let structure = structure_with_b_factors(&[95.0, 80.0, 60.0, 30.0]);
        let colors = atom_colors(
            &structure,
            ColorScheme::Confidence,
            &ColorContext::default(),
        );
        assert_eq!(as_255(colors[0]), [0, 83, 214]);
        assert_eq!(as_255(colors[1]), [101, 203, 243]);
        assert_eq!(as_255(colors[2]), [255, 219, 70]);
        assert_eq!(as_255(colors[3]), [255, 125, 69]);
    }

    #[test]
    fn b_factors_off_the_plddt_scale_are_normalised() {
        // Crystallographic B-factors run past 100 and mean something else entirely.
        let structure = structure_with_b_factors(&[12.0, 400.0]);
        assert!(!looks_like_plddt(&structure));
        let colors = atom_colors(
            &structure,
            ColorScheme::Confidence,
            &ColorContext::default(),
        );
        // Viridis, from its purple end to its yellow end.
        assert_eq!(as_255(colors[0]), palette::viridis(0.0));
        assert_eq!(as_255(colors[1]), palette::viridis(1.0));
    }

    #[test]
    fn every_scheme_colours_every_atom() {
        let structure = structure_with_b_factors(&[50.0, 50.0, 50.0]);
        for scheme in [
            ColorScheme::Element,
            ColorScheme::Chain,
            ColorScheme::ResidueKind,
            ColorScheme::Secondary,
            ColorScheme::Confidence,
        ] {
            let colors = atom_colors(&structure, scheme, &ColorContext::default());
            assert_eq!(colors.len(), 3);
            for color in colors {
                for channel in color {
                    assert!(
                        (0.0..=1.0).contains(&channel),
                        "{scheme:?} produced {channel}"
                    );
                }
            }
        }
    }

    #[test]
    fn colours_leave_in_linear_space() {
        // Mid-gray in sRGB is far from mid-gray in linear; getting this backwards is what
        // makes a render look darker than the palette.
        let linear = to_linear([128, 128, 128]);
        assert!(linear[0] > 0.2 && linear[0] < 0.24, "got {}", linear[0]);
        assert_eq!(to_linear([0, 0, 0]), [0.0, 0.0, 0.0]);
        assert_eq!(to_linear([255, 255, 255]), [1.0, 1.0, 1.0]);
        // Round trip through the test helper.
        assert_eq!(as_255(to_linear([250, 219, 37])), [250, 219, 37]);
    }

    #[test]
    fn carbon_follows_the_background() {
        // The whole reason colour takes a context: on the new white default, CPK carbon
        // would otherwise be a pale gray molecule on a pale gray page.
        let structure = structure_with_b_factors(&[40.0, 40.0]);
        let on_white = atom_colors(
            &structure,
            ColorScheme::Element,
            &ColorContext {
                background: [1.0, 1.0, 1.0],
                ..Default::default()
            },
        );
        let on_dark = atom_colors(
            &structure,
            ColorScheme::Element,
            &ColorContext {
                background: [0.04, 0.05, 0.07],
                ..Default::default()
            },
        );
        assert!(
            on_white[0][0] < on_dark[0][0],
            "carbon should darken on white"
        );
    }

    #[test]
    fn the_legend_describes_the_scheme_showing() {
        let structure = structure_with_b_factors(&[90.0, 40.0]);
        let context = ColorContext::default();
        assert_eq!(
            legend(&structure, ColorScheme::Confidence, &context).len(),
            4
        );
        assert_eq!(
            legend(&structure, ColorScheme::Chain, &context)[0].label,
            "Chain A_1"
        );
        assert!(legend(&structure, ColorScheme::Element, &context)[0]
            .label
            .starts_with('C'));
    }

    const EVERY_SCHEME: [ColorScheme; 8] = [
        ColorScheme::Element,
        ColorScheme::Chain,
        ColorScheme::ResidueKind,
        ColorScheme::Secondary,
        ColorScheme::Confidence,
        ColorScheme::BFactor,
        ColorScheme::Rainbow,
        ColorScheme::Chainbow,
    ];

    fn in_range(colors: &[[f32; 3]]) -> bool {
        colors
            .iter()
            .flatten()
            .all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel))
    }

    #[test]
    fn one_bad_b_factor_does_not_repaint_the_rest() {
        // An overflowed column reads as infinity. It used to make the range infinite, fall
        // back to 0..0, and push every ordinary atom to the top of the ramp.
        let structure = structure_with_b_factors(&[12.0, 26.0, 40.0, f32::INFINITY, f32::NAN]);
        assert_eq!(b_factor_range(&structure), (12.0, 40.0));
        let colors = atom_colors(&structure, ColorScheme::BFactor, &ColorContext::default());
        assert!(in_range(&colors));
        assert_eq!(as_255(colors[0]), Ramp::Rainbow.sample(0.0));
        assert_eq!(as_255(colors[1]), Ramp::Rainbow.sample(0.5));
        assert_eq!(as_255(colors[2]), Ramp::Rainbow.sample(1.0));
        for scheme in EVERY_SCHEME {
            assert!(
                in_range(&atom_colors(&structure, scheme, &ColorContext::default())),
                "{scheme:?}"
            );
        }
    }

    #[test]
    fn a_structure_with_one_b_factor_throughout_gets_one_colour() {
        // Every model from some predictors, and every structure refined without B-factors.
        let structure = structure_with_b_factors(&[30.0, 30.0, 30.0]);
        for scheme in [ColorScheme::BFactor, ColorScheme::Confidence] {
            let colors = atom_colors(
                &structure,
                scheme,
                &ColorContext {
                    continuous: true,
                    ..Default::default()
                },
            );
            assert!(in_range(&colors), "{scheme:?}");
            assert!(
                colors.windows(2).all(|pair| pair[0] == pair[1]),
                "{scheme:?}"
            );
        }
        // And its legend still has numbers on it, not NaN.
        let rows = legend(&structure, ColorScheme::BFactor, &ColorContext::default());
        assert!(rows.iter().all(|row| row.label == "30"), "{rows:?}");
    }

    #[test]
    fn an_empty_structure_colours_and_legends_without_panicking() {
        let structure = Structure::build("t".into(), "test".into(), Vec::new());
        for scheme in EVERY_SCHEME {
            assert!(atom_colors(&structure, scheme, &ColorContext::default()).is_empty());
            for row in legend(&structure, scheme, &ColorContext::default()) {
                assert!(
                    !row.label.contains("NaN") && !row.label.contains("inf"),
                    "{row:?}"
                );
            }
        }
        assert_eq!(residue_color(&[], &structure, 0), [0.8, 0.8, 0.8]);
    }

    #[test]
    fn a_residue_past_the_end_is_grey_not_a_panic() {
        let structure = structure_with_b_factors(&[50.0]);
        let colors = atom_colors(&structure, ColorScheme::Chain, &ColorContext::default());
        assert_eq!(residue_color(&colors, &structure, 99), colors[0]);
        assert_eq!(residue_color(&[], &structure, 0), [0.8, 0.8, 0.8]);
    }

    #[test]
    fn a_ramp_over_a_single_residue_has_somewhere_to_be() {
        // One residue has no sweep; dividing by its length minus one is a division by zero.
        let structure = structure_with_chains(&[("A", 1), ("B", 1)]);
        for scheme in [ColorScheme::Rainbow, ColorScheme::Chainbow] {
            let colors = atom_colors(&structure, scheme, &ColorContext::default());
            assert!(in_range(&colors), "{scheme:?}");
        }
        let chainbow = atom_colors(&structure, ColorScheme::Chainbow, &ColorContext::default());
        assert_eq!(as_255(chainbow[0]), Ramp::Rainbow.sample(0.0));
    }

    #[test]
    fn many_chains_wrap_the_chain_palette_rather_than_running_out() {
        let chains: Vec<(String, usize)> = (0..30).map(|index| (format!("C{index}"), 1)).collect();
        let named: Vec<(&str, usize)> = chains.iter().map(|(id, n)| (id.as_str(), *n)).collect();
        let structure = structure_with_chains(&named);
        let colors = atom_colors(&structure, ColorScheme::Chain, &ColorContext::default());
        assert_eq!(colors.len(), 30);
        assert!(in_range(&colors));
        assert_eq!(
            legend(&structure, ColorScheme::Chain, &ColorContext::default()).len(),
            30
        );
    }

    #[test]
    fn a_ligand_only_structure_takes_every_scheme() {
        // No polymer: nothing for secondary structure or a sequence ramp to say.
        let atoms: Vec<RawAtom> = (0..3)
            .map(|index| RawAtom {
                name: format!("C{index}"),
                element: 6,
                position: [index as f32 * 1.5, 0.0, 0.0],
                b_factor: 15.0,
                occupancy: 1.0,
                residue_name: "ATP".into(),
                residue_number: 1,
                insertion_code: None,
                chain_id: "B".into(),
                is_hetatm: true,
                label_asym_id: String::new(),
                entity_id: String::new(),
            })
            .collect();
        let structure = Structure::build("t".into(), "test".into(), atoms);
        for scheme in EVERY_SCHEME {
            let colors = atom_colors(&structure, scheme, &ColorContext::default());
            assert_eq!(colors.len(), 3, "{scheme:?}");
            assert!(in_range(&colors), "{scheme:?}");
        }
    }

    #[test]
    fn a_file_that_names_its_experiment_has_b_factors_not_confidence() {
        // Ordinary crystal B-factors, all inside 0..100.
        let mut structure = structure_with_b_factors(&[12.0, 35.0, 60.0]);
        assert!(looks_like_plddt(&structure), "nothing says otherwise");
        structure.experimental_method = Some("X-RAY DIFFRACTION".into());
        assert!(!looks_like_plddt(&structure));
        // A file that calls itself a model is still a model.
        structure.experimental_method = Some("THEORETICAL MODEL".into());
        assert!(looks_like_plddt(&structure));
    }
}
