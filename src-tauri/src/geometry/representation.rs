//! Choosing what to draw.
//!
//! A representation is a decision about which atoms are visible and what shape stands in
//! for them. Everything here is a filter plus a radius rule - the tessellation lives in
//! `cartoon`, and the spheres and cylinders are instance lists that need no geometry at
//! all until the renderer or Blender turns them into shapes.

use super::cartoon;
use super::color::{self, ColorScheme, Ramp};
use super::mesh::Scene;
use super::palette::Categorical;
use super::style::Material;
use crate::structure::chem;
use crate::structure::model::flags;
use crate::structure::Structure;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Representation {
    Cartoon,
    Spacefill,
    BallAndStick,
    Sticks,
    Backbone,
    /// A smooth surface over the atoms, drawn as glass over whatever else they are shown
    /// as: a cartoon in glass is the usual pairing. It draws no atoms of its own, and is
    /// built after the others (`Scene::glass`).
    Glass,
}

/// Every representation that draws atoms, in the order they are drawn. Cartoon first so its
/// ribbon is the first thing in the mesh, which keeps the scene's primary representation
/// meaningful. Glass is not among them: it is a surface over the others, built after them.
pub const ALL_REPRESENTATIONS: [Representation; 5] = [
    Representation::Cartoon,
    Representation::Spacefill,
    Representation::BallAndStick,
    Representation::Sticks,
    Representation::Backbone,
];

impl Representation {
    pub fn label(self) -> &'static str {
        match self {
            Representation::Cartoon => "Cartoon",
            Representation::Spacefill => "Spacefill",
            Representation::BallAndStick => "Ball and stick",
            Representation::Sticks => "Sticks",
            Representation::Backbone => "Backbone",
            Representation::Glass => "Glass",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Representation::Cartoon => "cartoon",
            Representation::Spacefill => "spacefill",
            Representation::BallAndStick => "ball-and-stick",
            Representation::Sticks => "sticks",
            Representation::Backbone => "backbone",
            Representation::Glass => "glass",
        }
    }

    /// Half-thickness of a bond, in angstroms.
    ///
    /// Ball-and-stick was 0.16 against a 0.43 A carbon ball - a ratio of 2.7, which on a
    /// ligand reads fine and on a whole chain reads as beads threaded on wire. The balls
    /// are now a little smaller and the sticks a lot thicker, for a ratio near 1.5.
    ///
    /// Thicker again since, for the representations where the stick *is* the model: 0.22 A
    /// half-thickness is under half an angstrom across, which on a whole chain reads as a
    /// wireframe rather than a model and is thin enough that its own antialiasing eats it
    /// at a distance. Ball-and-stick keeps its thinner stick, because there the ball sets
    /// the scale and a fatter stick only spoils the proportion.
    fn bond_radius(self) -> f32 {
        match self {
            Representation::BallAndStick => 0.24,
            Representation::Sticks => 0.30,
            Representation::Backbone => 0.36,
            Representation::Cartoon | Representation::Spacefill | Representation::Glass => 0.22,
        }
    }

    /// Sphere radius for one atom, from its van der Waals radius.
    fn atom_radius(self, vdw: f32) -> f32 {
        match self {
            Representation::Spacefill => vdw,
            Representation::BallAndStick => (vdw * 0.21).max(0.3),
            // A cartoon's atoms are its ligands, which are read on their own and can
            // afford to be a touch chunkier than a whole chain of them.
            Representation::Cartoon => (vdw * 0.25).max(0.3),
            Representation::Sticks | Representation::Backbone | Representation::Glass => {
                self.bond_radius()
            }
        }
    }

    fn draws_bonds(self) -> bool {
        !matches!(self, Representation::Spacefill | Representation::Glass)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneOptions {
    #[serde(default)]
    pub show_hydrogens: bool,
    #[serde(default)]
    pub show_waters: bool,
    #[serde(default = "yes")]
    pub show_ligands: bool,
}

fn yes() -> bool {
    true
}

/// How a molecule, or one chain of it, is drawn.
///
/// The analogue of a Blender material: representation, colour and surface, bundled into
/// one assignable thing. Which atoms *exist* is not in here - hydrogens, waters and
/// ligands are a property of the molecule, the way an object's contents are not part of
/// its material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    /// Which representations to draw. Several at once: a cartoon with its side chains as
    /// sticks is one style, not two, and a viewer that makes you choose is a viewer you
    /// fight. PyMOL's `show` is additive for the same reason.
    #[serde(default = "default_representations")]
    pub representations: Vec<Representation>,
    #[serde(default = "default_scheme")]
    pub scheme: ColorScheme,
    #[serde(default)]
    pub categorical: Categorical,
    /// Use viridis for continuous values instead of the AlphaFold confidence bands.
    #[serde(default)]
    pub continuous: bool,
    /// A flat colour, used instead of the scheme when set. This is what a named colour
    /// like PyMOL's `orange` is: not a rule, one colour.
    #[serde(default)]
    pub solid: Option<[f32; 3]>,
    /// Which ramp the continuous schemes use - rainbow, viridis, blue-white-red.
    #[serde(default)]
    pub ramp: Ramp,
    /// The surface. Part of the style for the same reason a Blender material's surface is
    /// part of the material: "cartoon, orange, matte" is one description of one thing.
    #[serde(default)]
    pub material: Material,
    /// Colour everything but carbon by element, whatever the scheme says.
    ///
    /// On by default, because it is what a figure looks like: a chain painted one colour
    /// with its oxygens red and its nitrogens blue reads as chemistry, and the same chain
    /// painted uniformly reads as a shape. The scheme still decides carbon, which is most
    /// of the atoms, so the chain's identity survives. Turn it off to get a flat colour
    /// with no exceptions.
    #[serde(default = "yes")]
    pub heteroatoms_by_element: bool,
    /// The glass's own colour. When unset the glass takes the colour of the atoms under
    /// it - its chain's, under the default scheme - so the glass and the ribbon inside it
    /// agree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glass: Option<[f32; 3]>,
}

fn default_scheme() -> ColorScheme {
    ColorScheme::Chain
}

fn default_representations() -> Vec<Representation> {
    vec![Representation::Cartoon]
}

impl Default for Style {
    fn default() -> Self {
        Style {
            representations: default_representations(),
            scheme: ColorScheme::Chain,
            categorical: Categorical::default(),
            continuous: false,
            solid: None,
            ramp: Ramp::default(),
            material: Material::default(),
            heteroatoms_by_element: true,
            glass: None,
        }
    }
}

/// What makes two chains share a colour array: the same scheme, palette, scale and flat
/// colour. The flat colour is compared by bits because a float is not `Eq`.
type Colouring = (ColorScheme, Categorical, bool, Ramp, Option<[u32; 3]>);

impl Style {
    /// Whether this style draws a given representation.
    pub fn draws(&self, representation: Representation) -> bool {
        self.representations.contains(&representation)
    }

    /// The representation that decides scene-wide settings - the bond radius, and what the
    /// legend describes. The first in the list, so the order is the user's priority.
    pub fn primary(&self) -> Representation {
        // Glass is a surface over the others rather than a way of drawing atoms, so it
        // decides nothing while there is another.
        self.representations
            .iter()
            .copied()
            .find(|representation| *representation != Representation::Glass)
            .or_else(|| self.representations.first().copied())
            .unwrap_or(Representation::Cartoon)
    }
}

/// A style and the chain it applies to. `None` means every chain without one of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    #[serde(default)]
    pub chain: Option<String>,
    pub style: Style,
}

/// One feature of a molecule: something said about a run of its residues.
///
/// This is the BioMol's own record of how it looks, below the level a style reaches - a
/// style is a chain's, a feature is "residues 80 to 95 of chain A": drawn as sticks as
/// well, or coloured, or hidden. Named the way a person cites residues - a chain and
/// author numbers - so it survives the file being read again and means the same thing to
/// an agent, a script and a saved world.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    /// The chain, by label id (`A_1`) or author letter.
    pub chain: String,
    /// Author residue numbers, both ends included.
    pub from: i32,
    pub to: i32,
    /// Representations these residues are drawn in on top of their chain's style.
    #[serde(default)]
    pub show: Vec<Representation>,
    /// Not drawn at all, in any representation that draws atoms.
    #[serde(default)]
    pub hide: bool,
    /// A flat colour, linear RGB like a style's `solid`. Carbon takes it under
    /// heteroatoms-by-element, everything takes it otherwise.
    #[serde(default)]
    pub colour: Option<[f32; 3]>,
    /// Several ranges at once, as `"53,57-58,61"`; `from` and `to` are then their first
    /// and last. One feature for every scattered residue a selection covers was a hundred
    /// bytes each, and a capsid's hydrophobic core coloured was a hundred thousand of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranges: Option<String>,
}

impl Feature {
    /// The ranges it covers, each as (first, last) with first <= last.
    pub fn spans(&self) -> Vec<(i32, i32)> {
        let one = (self.from.min(self.to), self.from.max(self.to));
        let Some(ranges) = &self.ranges else {
            return vec![one];
        };
        let parsed: Vec<(i32, i32)> = ranges
            .split(',')
            .filter_map(|part| {
                let part = part.trim();
                // A negative number is a residue number too: "-3--1" is -3 to -1.
                let split = part
                    .char_indices()
                    .skip(1)
                    .find(|&(_, c)| c == '-')
                    .map(|(at, _)| at);
                match split {
                    Some(at) => {
                        let (a, b): (i32, i32) =
                            (part[..at].parse().ok()?, part[at + 1..].parse().ok()?);
                        Some((a.min(b), a.max(b)))
                    }
                    None => part.parse().ok().map(|n: i32| (n, n)),
                }
            })
            .collect();
        if parsed.is_empty() {
            vec![one]
        } else {
            parsed
        }
    }
}

/// The same features, as few as say the same thing.
///
/// A colour given to a selection is added as features over the residues it covers, and
/// colouring again adds more over the old ones rather than replacing them: a capsid
/// coloured a few dozen times carried two hundred thousand features, twenty-four megabytes
/// read back every time its world was listed, opened or saved. This works out what each
/// residue ends up as - the way `build_scene_among` applies them: the last colour, the last
/// representations asked for, hidden if any feature hid it - and says that again as runs
/// of neighbouring residues that end up alike.
///
/// Features are grouped by the chain name they use. A name used in a range too long to
/// spread out (more than 20,000 numbers) is left exactly as it was.
pub fn compact_features(features: &[Feature]) -> Vec<Feature> {
    #[derive(Clone, PartialEq)]
    struct End {
        show: Vec<Representation>,
        hide: bool,
        colour: Option<[u32; 3]>,
    }
    const WIDEST: i64 = 20_000;
    // Each chain name in the order of its last feature, so that where two names reach the
    // same residues (an author letter and a label), the one used last is still applied last.
    let mut names: Vec<&str> = Vec::new();
    for feature in features.iter().rev() {
        if !names.contains(&feature.chain.as_str()) {
            names.push(&feature.chain);
        }
    }
    names.reverse();
    let mut out = Vec::new();
    for name in names {
        let mine: Vec<&Feature> = features
            .iter()
            .filter(|feature| feature.chain == name)
            .collect();
        if mine.iter().any(|feature| {
            feature
                .spans()
                .iter()
                .any(|&(from, to)| i64::from(to) - i64::from(from) > WIDEST)
        }) {
            out.extend(mine.into_iter().cloned());
            continue;
        }
        let mut ends: std::collections::BTreeMap<i32, End> = std::collections::BTreeMap::new();
        for feature in &mine {
            for number in feature.spans().into_iter().flat_map(|(from, to)| from..=to) {
                let end = ends.entry(number).or_insert(End {
                    show: Vec::new(),
                    hide: false,
                    colour: None,
                });
                if !feature.show.is_empty() {
                    end.show = feature.show.clone();
                }
                end.hide |= feature.hide;
                if let Some(colour) = feature.colour {
                    end.colour = Some(colour.map(f32::to_bits));
                }
            }
        }
        // Runs of neighbouring numbers that end up alike, then one feature for each way of
        // ending up, carrying all its runs.
        let mut runs: Vec<(i32, i32, End)> = Vec::new();
        for (number, end) in ends {
            match runs.last_mut() {
                Some((_, to, same)) if *to + 1 == number && *same == end => *to = number,
                _ => runs.push((number, number, end)),
            }
        }
        let mut kinds: Vec<(End, Vec<(i32, i32)>)> = Vec::new();
        for (from, to, end) in runs {
            if end.show.is_empty() && !end.hide && end.colour.is_none() {
                continue;
            }
            match kinds.iter_mut().find(|(kind, _)| *kind == end) {
                Some((_, spans)) => spans.push((from, to)),
                None => kinds.push((end, vec![(from, to)])),
            }
        }
        for (end, spans) in kinds {
            let ranges = (spans.len() > 1).then(|| {
                spans
                    .iter()
                    .map(|&(from, to)| {
                        if from == to {
                            from.to_string()
                        } else {
                            format!("{from}-{to}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            });
            out.push(Feature {
                chain: name.to_string(),
                from: spans[0].0,
                to: spans[spans.len() - 1].1,
                show: end.show,
                hide: end.hide,
                colour: end.colour.map(|bits| bits.map(f32::from_bits)),
                ranges,
            });
        }
    }
    out
}

impl Assignment {
    pub fn everything(style: Style) -> Self {
        Assignment { chain: None, style }
    }
}

impl Default for SceneOptions {
    fn default() -> Self {
        // Hydrogens and waters hidden is what every viewer defaults to: a model with
        // hydrogens is unreadable as ball-and-stick, and ordered waters bury the protein.
        Self {
            show_hydrogens: false,
            show_waters: false,
            show_ligands: true,
        }
    }
}

/// Builds everything needed to draw one representation of one structure.
/// Builds the scene, chain by chain.
///
/// Per chain rather than in one pass because a style is assignable per chain: a graph can
/// say "chain A as a cartoon, its ligand as sticks", which is most of what a figure of a
/// complex is. Chains with no style of their own get the unassigned one.
pub fn build_scene(
    structure: &Structure,
    options: SceneOptions,
    assignments: &[Assignment],
    scene_style: &super::SceneStyle,
    // `coarse` builds the ribbon at its coarsest, for a preview rather than for study.
    coarse: bool,
) -> Scene {
    build_scene_with(structure, options, assignments, &[], scene_style, coarse)
}

/// `build_scene`, with the molecule's residue-level features laid over its chain styles.
pub fn build_scene_with(
    structure: &Structure,
    options: SceneOptions,
    assignments: &[Assignment],
    features: &[Feature],
    scene_style: &super::SceneStyle,
    coarse: bool,
) -> Scene {
    build_scene_among(
        structure,
        options,
        assignments,
        features,
        scene_style,
        coarse,
        structure.residues.len(),
    )
}

/// `build_scene_with`, for one structure of a scene that holds `scene_residues` in all.
///
/// How finely a ribbon is drawn is a budget for the whole viewport, not for one molecule:
/// two capsids side by side are twice the triangles of one, and it was the second copy of
/// one that took a window over its memory.
pub fn build_scene_among(
    structure: &Structure,
    options: SceneOptions,
    assignments: &[Assignment],
    features: &[Feature],
    scene_style: &super::SceneStyle,
    coarse: bool,
    scene_residues: usize,
) -> Scene {
    // Features resolved to atoms once: which extra representations each atom is in, which
    // are hidden, and which have a colour of their own. Later features win.
    let atom_count = structure.atom_count();
    let mut extra: Vec<Vec<Representation>> = Vec::new();
    let mut extra_of: Vec<u32> = vec![0; if features.is_empty() { 0 } else { atom_count }];
    let mut hidden: Vec<bool> = vec![false; extra_of.len()];
    let mut own_color: Vec<Option<[f32; 3]>> = vec![None; extra_of.len()];
    let mut named: std::collections::HashMap<String, Vec<(i32, usize)>> =
        std::collections::HashMap::new();
    for feature in features {
        // A range written backwards - "95 to 80", as a person or an agent may well say it -
        // means the same residues. Taken literally it matched nothing, and the feature
        // was silently dropped.
        // Each chain name's residues sorted by number once, then each range found in
        // them by bisection: asked of the whole structure for every range, a capsid
        // coloured residue by residue was billions of steps and never drew.
        let numbered = named
            .entry(feature.chain.clone())
            .or_insert_with(|| structure.numbered_residues(&feature.chain));
        let residues: Vec<usize> = feature
            .spans()
            .into_iter()
            .flat_map(|(from, to)| {
                let start = numbered.partition_point(|&(number, _)| number < from);
                let end = numbered.partition_point(|&(number, _)| number <= to);
                numbered[start..end].iter().map(|&(_, index)| index)
            })
            .collect();
        let slot = if feature.show.is_empty() {
            0
        } else {
            extra.push(feature.show.clone());
            extra.len() as u32
        };
        // Polymer residues only, which is what `residue_indices_in_range` answers for: a
        // water that happens to be numbered inside the range is not part of it.
        for index in residues {
            let Some(residue) = structure.residues.get(index) else {
                continue;
            };
            let first = residue.first_atom as usize;
            for atom in first..(first + residue.atom_count as usize).min(atom_count) {
                if slot > 0 {
                    extra_of[atom] = slot;
                }
                if feature.hide {
                    hidden[atom] = true;
                }
                if feature.colour.is_some() {
                    own_color[atom] = feature.colour;
                }
            }
        }
    }
    let featured = !extra_of.is_empty();
    let extra_draws = |atom: usize, representation: Representation| -> bool {
        featured
            && extra_of[atom] > 0
            && extra[extra_of[atom] as usize - 1].contains(&representation)
    };
    let is_hidden = |atom: usize| featured && hidden[atom];

    let fallback = assignments
        .iter()
        .find(|assignment| assignment.chain.is_none())
        .map(|assignment| assignment.style.clone())
        .unwrap_or_default();

    // One style per chain, resolved once so the per-atom loop is a lookup.
    let chain_style: Vec<Style> = structure
        .chains
        .iter()
        .map(|chain| {
            assignments
                .iter()
                .find(|assignment| assignment.chain.as_deref() == Some(chain.id.as_str()))
                .map(|assignment| assignment.style.clone())
                .unwrap_or_else(|| fallback.clone())
        })
        .collect();

    // Colours are computed per distinct colouring rather than per chain: two chains
    // sharing a style share the work, and a whole-structure scheme like Chain has to see
    // the whole structure to assign its colours consistently. They then go into one array
    // by atom - an atom is in one chain, so it takes one colouring - and the ribbon is built
    // once from it. A colouring per chain used to be its own array the size of the whole
    // structure and its own pass over every ribbon: a capsid with each of its sixty chains
    // coloured was sixty of both, one after another.
    enum Paint {
        Solid([f32; 3]),
        Scheme(usize),
    }
    let mut schemes: Vec<(Colouring, Vec<[f32; 3]>)> = Vec::new();
    let mut paint_of: Vec<Paint> = Vec::with_capacity(chain_style.len() + 1);
    for style in chain_style.iter().chain(std::iter::once(&fallback)) {
        // A flat colour is already linear: it came from a swatch the core defined, not
        // from a rule over the structure.
        if let Some(rgb) = style.solid {
            paint_of.push(Paint::Solid(rgb));
            continue;
        }
        let key: Colouring = (
            style.scheme,
            style.categorical,
            style.continuous,
            // The ramp belongs in the key: two chains whose styles differ only in their
            // ramp are two colourings, and leaving it out would hand the second one the
            // first one's colours out of the cache.
            style.ramp,
            None,
        );
        let index = match schemes.iter().position(|(existing, _)| *existing == key) {
            Some(index) => index,
            None => {
                let context = super::color::ColorContext {
                    background: scene_style.background,
                    categorical: style.categorical,
                    continuous: style.continuous,
                    ramp: style.ramp,
                };
                schemes.push((key, color::atom_colors(structure, style.scheme, &context)));
                schemes.len() - 1
            }
        };
        paint_of.push(Paint::Scheme(index));
    }
    // The last entry is the fallback's, for atoms in no chain at all.
    let fallback_paint = paint_of.pop().unwrap_or(Paint::Solid([0.8, 0.8, 0.8]));
    let mut atom_colors: Vec<[f32; 3]> = (0..structure.atom_count())
        .map(|atom| {
            let paint = structure
                .residue_of_atom
                .get(atom)
                .and_then(|residue| structure.residues.get(*residue as usize))
                .and_then(|residue| paint_of.get(residue.chain_index as usize))
                .unwrap_or(&fallback_paint);
            match paint {
                Paint::Solid(rgb) => *rgb,
                Paint::Scheme(index) => schemes[*index].1[atom],
            }
        })
        .collect();
    drop(schemes);
    // A feature's colour goes over whichever colouring the atom had, so the ribbon, the
    // sticks and the spheres of those residues all take it.
    if featured {
        for (atom, color) in own_color.iter().enumerate() {
            if let (Some(color), Some(slot)) = (color, atom_colors.get_mut(atom)) {
                *slot = *color;
            }
        }
    }

    // Carbon and hydrogen follow the scheme; everything else is itself. Hydrogen is in
    // that group because a scheme's colour is easier to read on it than white is, and
    // because it is usually hidden anyway.
    let element_override = |element: u8| -> bool { !matches!(element, 1 | 6) };
    let light_background = super::palette::is_light(scene_style.background);
    let hetero_color = |element: u8| -> [f32; 3] {
        super::color::to_linear(super::palette::element_color(element, light_background))
    };

    let style_of_atom = |atom: usize| -> &Style {
        structure
            .residue_of_atom
            .get(atom)
            .and_then(|residue| structure.residues.get(*residue as usize))
            .and_then(|residue| chain_style.get(residue.chain_index as usize))
            .unwrap_or(&fallback)
    };

    let mut scene = Scene {
        representation: fallback.primary().as_str().to_string(),
        ..Default::default()
    };
    // A pass per representation, because a style draws several: a cartoon *and* its side
    // chains as sticks is one style, and an atom can legitimately be drawn twice - a
    // spacefill sphere with a stick through it is what a ball-and-stick over spacefill
    // looks like in every other viewer.
    let mut drawn: Vec<u32> = Vec::new();
    let ribboned = cartoon::covered(structure);
    for representation in ALL_REPRESENTATIONS {
        let mut visible: Vec<u32> = Vec::new();
        for atom in 0..structure.atom_count() {
            let style = style_of_atom(atom);
            if is_hidden(atom) {
                continue;
            }
            // Asked for by a feature: drawn whole, backbone included, as "show sticks" on
            // a few residues of a cartoon always means.
            let by_feature = extra_draws(atom, representation)
                && is_visible(structure, atom, options, representation, &ribboned);
            if by_feature
                || (style.draws(representation)
                    && is_visible(structure, atom, options, representation, &ribboned)
                    && !buried_by_cartoon(structure, atom, style, representation, &ribboned))
            {
                visible.push(atom as u32);
            }
        }
        if visible.is_empty() && representation != Representation::Cartoon {
            continue;
        }

        for &atom in &visible {
            let index = atom as usize;
            let style = style_of_atom(index);
            // The trace atom kept under a cartoon is kept for its bond, not for itself: a
            // 0.3 A sphere at the ribbon's axis pokes out of both faces of a ribbon thinner
            // than that, and reads as a ball sitting on the cartoon. The side chain's stick
            // still starts there; the sphere does not.
            if under_a_ribbon(structure, index, &ribboned)
                && trace_under_cartoon(structure, index, style, representation)
                && !extra_draws(index, representation)
            {
                continue;
            }
            let element = structure.elements[index];
            // Cartoon still draws ligands as atoms: a cartoon with the bound drug hidden
            // answers the wrong question.
            let radius = representation.atom_radius(chem::vdw_radius(element));
            let color = if style.heteroatoms_by_element && element_override(element) {
                hetero_color(element)
            } else {
                atom_colors[index]
            };
            scene
                .spheres
                .push(structure.position(index), radius, color, atom);
        }
        drawn.extend(visible.iter().copied());

        if representation.draws_bonds() {
            let mut shown = vec![false; structure.atom_count()];
            for &atom in &visible {
                shown[atom as usize] = true;
            }
            for bond in &structure.bonds {
                // Metal coordination is not a stick: it is in the graph as a contact, and
                // an interaction view is where it belongs. PyMOL draws no His-Fe stick
                // either.
                if !bond.is_covalent() {
                    continue;
                }
                let (a, b) = (bond.atoms[0] as usize, bond.atoms[1] as usize);
                // A bond needs both ends drawn in this representation. A cartoon chain
                // disulfide-bonded to a spacefill one would otherwise sprout a stick from
                // a solid sphere.
                if !shown[a] || !shown[b] {
                    continue;
                }
                let style_a = style_of_atom(a);
                let style_b = style_of_atom(b);
                let element_a = structure.elements[a];
                let element_b = structure.elements[b];
                let color_a = if style_a.heteroatoms_by_element && element_override(element_a) {
                    hetero_color(element_a)
                } else {
                    atom_colors[a]
                };
                let color_b = if style_b.heteroatoms_by_element && element_override(element_b) {
                    hetero_color(element_b)
                } else {
                    atom_colors[b]
                };
                scene.cylinders.push(
                    structure.position(a),
                    structure.position(b),
                    // This representation's radius, per bond: the joints are spheres of
                    // the same radius, and they only disappear into the stick if the two
                    // agree.
                    representation.bond_radius(),
                    color_a,
                    color_b,
                    (bond.atoms[0], bond.atoms[1]),
                );
            }
        }

        // The ribbon, once, for every chain whose style draws one.
        if representation == Representation::Cartoon {
            let chains: Vec<u32> = chain_style
                .iter()
                .enumerate()
                .filter(|(_, style)| style.draws(Representation::Cartoon))
                .map(|(chain, _)| chain as u32)
                .collect();
            if !chains.is_empty() {
                cartoon::build_among(
                    structure,
                    &atom_colors,
                    &mut scene.mesh,
                    Some(&chains),
                    coarse,
                    scene_residues,
                );
            }
        }
    }

    // The glass, over every atom some style or feature shows as glass: the atoms that
    // exist (hydrogens and, unless shown, waters left out), not hidden. In each atom's own
    // colour - its chain's, unless something coloured it - or the style's glass colour.
    let glass_atoms: Vec<usize> = (0..structure.atom_count())
        .filter(|&atom| {
            !is_hidden(atom)
                && (style_of_atom(atom).draws(Representation::Glass)
                    || extra_draws(atom, Representation::Glass))
                && structure.elements[atom] != 1
                && is_visible(structure, atom, options, Representation::Spacefill, &ribboned)
        })
        .collect();
    if !glass_atoms.is_empty() {
        scene.glass = super::envelope::envelope_of(structure, &glass_atoms, &|atom| {
            style_of_atom(atom).glass.unwrap_or(atom_colors[atom])
        });
        drawn.extend(glass_atoms.iter().map(|&atom| atom as u32));
    }

    // Counted once even when two representations drew it.
    drawn.sort_unstable();
    drawn.dedup();
    let visible = drawn;

    scene.drawn_atoms = visible.len() as u32;
    // The id space this scene's atom and residue ids live in, which is this structure's -
    // until it is merged with another, and `Scene::append` offsets them.
    scene.atom_count = structure.atom_count() as u32;
    scene.residue_count = structure.residues.len() as u32;
    let (min, max) = bounds(structure, &scene, fallback.primary());
    scene.bounds_min = min;
    scene.bounds_max = max;
    scene
}

/// Whether an atom is drawn: the molecule decides which atoms exist, the representation
/// decides which of those it draws.
/// PyMOL's `cartoon_side_chain_helper`, which is on in every preset that draws both.
///
/// A cartoon and sticks over the same residues is not two pictures added together: the
/// ribbon runs *through* where the backbone sticks are, so N, C and O end up inside it and
/// only their outward halves show - a mess of red and blue stubs poking out of the ribbon,
/// which reads as bonds that have come apart. PyMOL's answer is to drop the backbone from
/// the sticks when a cartoon covers it, keeping the alpha carbon so each side chain still
/// has something to grow out of, and that is this.
///
/// The trace atom is the exception rather than the name "CA", because the same rule has to
/// hold for nucleic acids, where the trace is the phosphorus.
fn buried_by_cartoon(
    structure: &Structure,
    atom: usize,
    style: &Style,
    representation: Representation,
    ribboned: &[bool],
) -> bool {
    // Nothing is buried by a ribbon that was never drawn over it.
    if !under_a_ribbon(structure, atom, ribboned) {
        return false;
    }
    if !matches!(
        representation,
        Representation::Sticks | Representation::BallAndStick
    ) {
        return false;
    }
    if !style.draws(Representation::Cartoon) {
        return false;
    }
    let flag = structure.flags[atom];
    if flag & flags::POLYMER == 0 || flag & flags::BACKBONE == 0 {
        return false;
    }
    let trace = structure
        .residue_of_atom
        .get(atom)
        .and_then(|index| structure.residues.get(*index as usize))
        .map(|residue| residue.trace_atom)
        .unwrap_or(-1);
    trace != atom as i32
}

/// Whether a ribbon is drawn over this atom's residue - see `cartoon::covered`.
fn under_a_ribbon(structure: &Structure, atom: usize, ribboned: &[bool]) -> bool {
    structure
        .residue_of_atom
        .get(atom)
        .and_then(|residue| ribboned.get(*residue as usize))
        .copied()
        .unwrap_or(false)
}

/// A polymer trace atom that a cartoon of the same style covers.
fn trace_under_cartoon(
    structure: &Structure,
    atom: usize,
    style: &Style,
    representation: Representation,
) -> bool {
    if !matches!(
        representation,
        Representation::Sticks | Representation::BallAndStick
    ) || !style.draws(Representation::Cartoon)
    {
        return false;
    }
    let flag = structure.flags[atom];
    if flag & flags::POLYMER == 0 || flag & flags::BACKBONE == 0 {
        return false;
    }
    structure
        .residue_of_atom
        .get(atom)
        .and_then(|index| structure.residues.get(*index as usize))
        .map(|residue| residue.trace_atom == atom as i32)
        .unwrap_or(false)
}

fn is_visible(
    structure: &Structure,
    atom: usize,
    options: SceneOptions,
    representation: Representation,
    ribboned: &[bool],
) -> bool {
    let flag = structure.flags[atom];
    if !options.show_hydrogens && flag & flags::HYDROGEN != 0 {
        return false;
    }
    if !options.show_waters && flag & flags::WATER != 0 {
        return false;
    }
    if !options.show_ligands && flag & flags::LIGAND != 0 {
        return false;
    }
    // Cartoon replaces the polymer's atoms with a ribbon - where there is a ribbon. A
    // residue the ribbon cannot reach keeps its atoms, or it is simply gone.
    if representation == Representation::Cartoon
        && flag & flags::POLYMER != 0
        && under_a_ribbon(structure, atom, ribboned)
    {
        return false;
    }
    // Backbone mode keeps ligands and ions: they are usually the reason someone is
    // looking at a large structure at all.
    if representation == Representation::Backbone
        && flag & flags::POLYMER != 0
        && flag & flags::BACKBONE == 0
    {
        return false;
    }
    true
}

/// Bounds of what is actually drawn, including radii.
///
/// Computed here rather than in the camera because it needs every drawn position and
/// radius, and this is the pass that already has them. A camera framing the structure's
/// atom centres clips a spacefill view; one framing the whole structure when only the
/// ligands are shown puts them off screen.
fn bounds(
    structure: &Structure,
    scene: &Scene,
    representation: Representation,
) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for sphere in 0..scene.spheres.len() {
        let base = sphere * 4;
        include(
            &mut min,
            &mut max,
            [
                scene.spheres.instances[base],
                scene.spheres.instances[base + 1],
                scene.spheres.instances[base + 2],
            ],
            scene.spheres.instances[base + 3],
        );
    }
    for vertex in 0..scene.mesh.vertex_count() {
        include(
            &mut min,
            &mut max,
            [
                scene.mesh.positions[vertex * 3],
                scene.mesh.positions[vertex * 3 + 1],
                scene.mesh.positions[vertex * 3 + 2],
            ],
            0.0,
        );
    }

    // A cartoon of a structure with no ligands would otherwise have no bounds at all when
    // its ribbon is empty; fall back to the trace.
    if !min[0].is_finite() {
        for residue in &structure.residues {
            if residue.trace_atom >= 0 {
                include(
                    &mut min,
                    &mut max,
                    structure.position(residue.trace_atom as usize),
                    1.0,
                );
            }
        }
    }
    if !min[0].is_finite() {
        let _ = representation;
        return ([0.0; 3], [0.0; 3]);
    }
    (min, max)
}

fn include(min: &mut [f32; 3], max: &mut [f32; 3], position: [f32; 3], radius: f32) {
    for axis in 0..3 {
        min[axis] = min[axis].min(position[axis] - radius);
        max[axis] = max[axis].max(position[axis] + radius);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::model::RawAtom;

    fn protein_with_ligand() -> Structure {
        let mut atoms = Vec::new();
        // Three residues of backbone plus side chains.
        for index in 0..3 {
            let x = index as f32 * 3.8;
            for (name, element, offset) in [
                (" N  ", 7u8, [-0.7, 0.0, 0.0]),
                (" CA ", 6, [0.0, 0.0, 0.0]),
                (" C  ", 6, [0.7, 0.0, 0.0]),
                (" O  ", 8, [1.0, 1.0, 0.0]),
                (" CB ", 6, [0.0, 1.5, 0.0]),
                (" HB1", 1, [0.0, 2.2, 0.0]),
            ] {
                atoms.push(RawAtom {
                    name: name.into(),
                    element,
                    position: [x + offset[0], offset[1], offset[2]],
                    b_factor: 40.0,
                    occupancy: 1.0,
                    residue_name: "ALA".into(),
                    residue_number: index + 1,
                    insertion_code: None,
                    chain_id: "A".into(),
                    is_hetatm: false,
                    label_asym_id: String::new(),
                    entity_id: String::new(),
                });
            }
        }
        // A water and a magnesium ion, well away from the chain.
        atoms.push(RawAtom {
            name: " O  ".into(),
            element: 8,
            position: [40.0, 0.0, 0.0],
            b_factor: 30.0,
            occupancy: 1.0,
            residue_name: "HOH".into(),
            residue_number: 100,
            insertion_code: None,
            chain_id: "B".into(),
            is_hetatm: true,
            label_asym_id: String::new(),
            entity_id: String::new(),
        });
        atoms.push(RawAtom {
            name: "MG  ".into(),
            element: 12,
            position: [50.0, 0.0, 0.0],
            b_factor: 25.0,
            occupancy: 1.0,
            residue_name: "MG".into(),
            residue_number: 101,
            insertion_code: None,
            chain_id: "C".into(),
            is_hetatm: true,
            label_asym_id: String::new(),
            entity_id: String::new(),
        });

        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        crate::structure::bonds::build(&mut structure, &Default::default());
        structure
    }

    /// One style for the whole structure, which is what most of these tests are about.
    fn draw(structure: &Structure, style: Style) -> Scene {
        draw_with(structure, SceneOptions::default(), style)
    }

    /// The same, with the molecule's own atom filters spelled out.
    fn draw_with(structure: &Structure, options: SceneOptions, style: Style) -> Scene {
        build_scene(
            structure,
            options,
            &[Assignment::everything(style)],
            &super::super::SceneStyle::default(),
            false,
        )
    }

    #[test]
    fn heteroatoms_keep_their_element_colour_under_any_scheme() {
        let structure = protein_with_ligand();
        // A flat colour, which is the strongest case: everything would be one colour if
        // the exception did not exist.
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::BallAndStick],
                solid: Some([0.2, 0.6, 0.9]),
                ..Default::default()
            },
        );

        let mut flat = 0;
        let mut coloured = 0;
        for index in 0..scene.spheres.len() {
            let atom = scene.spheres.atoms[index] as usize;
            let colour = [
                scene.spheres.colors[index * 3],
                scene.spheres.colors[index * 3 + 1],
                scene.spheres.colors[index * 3 + 2],
            ];
            let is_flat = (colour[0] - 0.2).abs() < 1e-4
                && (colour[1] - 0.6).abs() < 1e-4
                && (colour[2] - 0.9).abs() < 1e-4;
            match structure.elements[atom] {
                6 => {
                    assert!(is_flat, "carbon should take the style's colour");
                    flat += 1;
                }
                _ => {
                    assert!(!is_flat, "a heteroatom should be itself");
                    coloured += 1;
                }
            }
        }
        assert!(
            flat > 0 && coloured > 0,
            "{flat} carbons, {coloured} others"
        );
    }

    #[test]
    fn turning_the_exception_off_gives_a_flat_colour() {
        let structure = protein_with_ligand();
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::Spacefill],
                solid: Some([0.2, 0.6, 0.9]),
                heteroatoms_by_element: false,
                ..Default::default()
            },
        );
        for index in 0..scene.spheres.len() {
            assert!((scene.spheres.colors[index * 3] - 0.2).abs() < 1e-4);
        }
    }

    #[test]
    fn one_style_can_draw_a_cartoon_and_its_side_chains() {
        let structure = protein_with_ligand();

        let cartoon = draw(
            &structure,
            Style {
                representations: vec![Representation::Cartoon],
                ..Default::default()
            },
        );
        let both = draw(
            &structure,
            Style {
                representations: vec![Representation::Cartoon, Representation::Sticks],
                ..Default::default()
            },
        );

        // The cartoon alone hides the polymer's atoms and draws a ribbon. Adding sticks
        // brings the atoms back *and* keeps the ribbon - which is the whole reason
        // representations are a set: it is one style, not two.
        assert!(!cartoon.mesh.is_empty(), "a ribbon");
        assert!(!both.mesh.is_empty(), "still a ribbon");
        assert!(
            both.drawn_atoms > cartoon.drawn_atoms,
            "and now the side chains"
        );
        assert!(both.cylinders.len() > cartoon.cylinders.len(), "with bonds");
    }

    #[test]
    fn a_cartoon_over_sticks_drops_the_backbone_but_keeps_the_trace() {
        // PyMOL's `cartoon_side_chain_helper`. Without it the backbone sticks are inside
        // the ribbon and only the outward halves of N, C and O show, which is what "the
        // bonds are broken" looks like.
        let structure = protein_with_ligand();
        let both = draw(
            &structure,
            Style {
                representations: vec![Representation::Cartoon, Representation::Sticks],
                ..Default::default()
            },
        );
        let sticks_only = draw(
            &structure,
            Style {
                representations: vec![Representation::Sticks],
                ..Default::default()
            },
        );

        let drawn: std::collections::HashSet<u32> = both.spheres.atoms.iter().copied().collect();
        let mut kept_traces = 0;
        for (index, residue) in structure.residues.iter().enumerate() {
            for atom in residue.first_atom..residue.first_atom + residue.atom_count {
                let flag = structure.flags[atom as usize];
                if flag & flags::POLYMER == 0 || flag & flags::BACKBONE == 0 {
                    continue;
                }
                if residue.trace_atom == atom as i32 {
                    // Kept for its bond - the stick still hangs off it - but drawn as no
                    // sphere of its own, so it is not in the sphere list.
                    assert!(
                        !drawn.contains(&atom),
                        "residue {index}'s trace atom has a bond, not a sphere",
                    );
                    kept_traces += 1;
                } else {
                    assert!(
                        !drawn.contains(&atom),
                        "backbone atom {atom} is inside the ribbon and should not be a stick",
                    );
                }
            }
        }
        assert!(kept_traces > 0, "the fixture has a backbone to test");

        // The kept trace atom has its bond and not its sphere: a sphere at the ribbon's
        // axis is a ball on the cartoon.
        let sphere_atoms: std::collections::HashSet<u32> =
            both.spheres.atoms.iter().copied().collect();
        let bond_atoms: std::collections::HashSet<u32> =
            both.cylinders.atoms.iter().copied().collect();
        for residue in &structure.residues {
            if residue.trace_atom >= 0 {
                let trace = residue.trace_atom as u32;
                assert!(
                    !sphere_atoms.contains(&trace),
                    "no sphere at trace atom {trace}"
                );
                if bond_atoms.contains(&trace) {
                    // still the start of its side chain's stick
                }
            }
        }
        assert!(
            structure
                .residues
                .iter()
                .any(|residue| residue.trace_atom >= 0
                    && bond_atoms.contains(&(residue.trace_atom as u32))),
            "side chains still start at their trace atom",
        );

        // Sticks on their own are unaffected: this is about a cartoon covering them.
        assert!(
            sticks_only.spheres.len() > both.spheres.len(),
            "sticks alone draw the whole backbone: {} vs {}",
            sticks_only.spheres.len(),
            both.spheres.len(),
        );
        // And the ribbon is still there, with side chains still attached to it.
        assert!(!both.mesh.is_empty(), "still a ribbon");
        assert!(!both.cylinders.is_empty(), "still bonded side chains");
    }

    #[test]
    fn an_atom_drawn_twice_is_counted_once() {
        let structure = protein_with_ligand();
        // Spacefill and sticks both draw every heavy atom, so the sphere list is longer
        // than the atom count - deliberately, that is what a ball-and-stick over a
        // spacefill looks like - but the count of drawn atoms is not doubled.
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::Spacefill, Representation::Sticks],
                ..Default::default()
            },
        );
        assert_eq!(scene.drawn_atoms, 16);
        assert!(scene.spheres.len() > 16, "{} spheres", scene.spheres.len());
    }

    #[test]
    fn a_chain_can_be_drawn_differently_from_the_rest() {
        // Chain A is the polymer, B the water, C the magnesium.
        let structure = protein_with_ligand();

        // Everything as spacefill: 15 heavy polymer atoms + the ion. Then chain A alone
        // switched to cartoon, which replaces its atoms with a ribbon.
        let all_spacefill = draw(
            &structure,
            Style {
                representations: vec![Representation::Spacefill],
                ..Default::default()
            },
        );
        let mixed = build_scene(
            &structure,
            SceneOptions::default(),
            &[
                Assignment::everything(Style {
                    representations: vec![Representation::Spacefill],
                    ..Default::default()
                }),
                Assignment {
                    chain: Some("A_1".into()),
                    style: Style {
                        representations: vec![Representation::Cartoon],
                        ..Default::default()
                    },
                },
            ],
            &super::super::SceneStyle::default(),
            false,
        );

        assert_eq!(all_spacefill.drawn_atoms, 16);
        // The polymer's atoms are gone, the magnesium stays, and a ribbon has appeared.
        assert_eq!(mixed.drawn_atoms, 1);
        assert!(all_spacefill.mesh.is_empty(), "spacefill draws no ribbon");
        assert!(!mixed.mesh.is_empty(), "chain A should have a ribbon");
    }

    #[test]
    fn a_ribbon_is_built_only_for_the_chains_asking_for_one() {
        let structure = protein_with_ligand();
        // Cartoon assigned to the water's chain, which has no polymer at all: the polymer
        // chain keeps its atoms and nothing gets a ribbon.
        let scene = build_scene(
            &structure,
            SceneOptions::default(),
            &[
                Assignment::everything(Style {
                    representations: vec![Representation::BallAndStick],
                    ..Default::default()
                }),
                Assignment {
                    chain: Some("B_1".into()),
                    style: Style {
                        representations: vec![Representation::Cartoon],
                        ..Default::default()
                    },
                },
            ],
            &super::super::SceneStyle::default(),
            false,
        );
        assert!(scene.mesh.is_empty(), "chain B is a water, not a polymer");
        assert_eq!(scene.drawn_atoms, 16, "the polymer is still drawn as atoms");
    }

    #[test]
    fn a_bond_needs_both_ends_to_want_one() {
        let structure = protein_with_ligand();
        // Sticks everywhere draws bonds; making chain A spacefill leaves nothing bonded,
        // because a stick growing out of a solid sphere is what this avoids.
        let sticks = draw(
            &structure,
            Style {
                representations: vec![Representation::Sticks],
                ..Default::default()
            },
        );
        assert!(!sticks.cylinders.is_empty());

        let mixed = build_scene(
            &structure,
            SceneOptions::default(),
            &[
                Assignment::everything(Style {
                    representations: vec![Representation::Sticks],
                    ..Default::default()
                }),
                Assignment {
                    chain: Some("A_1".into()),
                    style: Style {
                        representations: vec![Representation::Spacefill],
                        ..Default::default()
                    },
                },
            ],
            &super::super::SceneStyle::default(),
            false,
        );
        assert!(mixed.cylinders.is_empty(), "spacefill draws no bonds");
    }

    #[test]
    fn an_unassigned_chain_falls_back_rather_than_vanishing() {
        let structure = protein_with_ligand();
        // Only chain C is assigned. A and B must still be drawn, by the fallback.
        let scene = build_scene(
            &structure,
            SceneOptions::default(),
            &[Assignment {
                chain: Some("C_1".into()),
                style: Style {
                    representations: vec![Representation::Spacefill],
                    ..Default::default()
                },
            }],
            &super::super::SceneStyle::default(),
            false,
        );
        // The fallback is Style::default(), which is cartoon - so the polymer becomes a
        // ribbon and the magnesium is a full-size sphere.
        assert!(!scene.mesh.is_empty());
        assert!(scene.drawn_atoms > 0);
    }

    #[test]
    fn ball_and_stick_draws_both_balls_and_sticks() {
        let structure = protein_with_ligand();
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::BallAndStick],
                ..Default::default()
            },
        );
        assert!(!scene.spheres.is_empty(), "balls");
        assert!(!scene.cylinders.is_empty(), "sticks");
        // The proportion is the whole point: balls were 2.7x the stick radius, which on a
        // ligand reads fine and on a chain reads as beads threaded on wire. Nearer 1.5 now.
        let ball = scene.spheres.instances[3];
        let stick = scene.cylinders.radius(0);
        let ratio = ball / stick;
        assert!(ratio > 1.2 && ratio < 1.8, "ball {ball} vs stick {stick}");
    }

    #[test]
    fn hydrogens_and_waters_are_hidden_by_default() {
        let structure = protein_with_ligand();
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::BallAndStick],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        );
        // 3 residues x 5 heavy atoms + the magnesium; the water and the three hydrogens
        // are hidden.
        assert_eq!(scene.drawn_atoms, 16);
    }

    #[test]
    fn showing_hydrogens_and_waters_adds_them_back() {
        let structure = protein_with_ligand();
        // Which atoms exist is the molecule's business, not the style's - so this is
        // asked of the molecule and the style says nothing about it.
        let scene = draw_with(
            &structure,
            SceneOptions {
                show_hydrogens: true,
                show_waters: true,
                show_ligands: true,
            },
            Style {
                representations: vec![Representation::BallAndStick],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        );
        assert_eq!(scene.drawn_atoms, 20);
    }

    #[test]
    fn spacefill_uses_van_der_waals_radii_and_draws_no_bonds() {
        let structure = protein_with_ligand();
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::Spacefill],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        );
        assert!(scene.cylinders.is_empty());
        // Carbon's Bondi radius.
        let carbon = (0..scene.spheres.len())
            .find(|index| structure.elements[scene.spheres.atoms[*index] as usize] == 6)
            .unwrap();
        assert!((scene.spheres.instances[carbon * 4 + 3] - 1.70).abs() < 1e-5);
    }

    #[test]
    fn cartoon_hides_the_polymer_but_keeps_the_ligand() {
        let structure = protein_with_ligand();
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::Cartoon],
                scheme: ColorScheme::Chain,
                ..Default::default()
            },
        );
        // Only the magnesium is drawn as an atom.
        assert_eq!(scene.drawn_atoms, 1);
        assert_eq!(structure.elements[scene.spheres.atoms[0] as usize], 12);
        // ...and the ribbon exists.
        assert!(scene.mesh.triangle_count() > 0);
    }

    #[test]
    fn backbone_drops_side_chains_but_not_ligands() {
        let structure = protein_with_ligand();
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::Backbone],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        );
        // N, CA, C, O per residue, plus the magnesium. CB is a side chain.
        assert_eq!(scene.drawn_atoms, 13);
    }

    #[test]
    fn bonds_to_hidden_atoms_are_not_drawn() {
        let structure = protein_with_ligand();
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::BallAndStick],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        );
        // A bond whose far atom is hidden would be a stick ending in mid air.
        for bond in 0..scene.cylinders.len() {
            for atom in [
                scene.cylinders.atoms[bond * 2],
                scene.cylinders.atoms[bond * 2 + 1],
            ] {
                assert_eq!(
                    structure.flags[atom as usize] & flags::HYDROGEN,
                    0,
                    "a bond reaches a hidden hydrogen"
                );
            }
        }
    }

    #[test]
    fn bounds_cover_the_radii_that_are_drawn() {
        let structure = protein_with_ligand();
        let spacefill = draw(
            &structure,
            Style {
                representations: vec![Representation::Spacefill],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        );
        let sticks = draw(
            &structure,
            Style {
                representations: vec![Representation::Sticks],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        );
        // Spacefill reaches further than sticks over the same atoms, so a camera framed on
        // one must not be used for the other.
        assert!(spacefill.bounds_max[0] > sticks.bounds_max[0]);
    }

    fn atom(
        name: &str,
        element: u8,
        position: [f32; 3],
        residue: (&str, i32),
        chain: &str,
        is_hetatm: bool,
    ) -> RawAtom {
        RawAtom {
            name: name.into(),
            element,
            position,
            b_factor: 30.0,
            occupancy: 1.0,
            residue_name: residue.0.into(),
            residue_number: residue.1,
            insertion_code: None,
            chain_id: chain.into(),
            is_hetatm,
            label_asym_id: String::new(),
            entity_id: String::new(),
        }
    }

    /// A peptide of `count` residues, with every residue's atoms placed by `place`.
    fn peptide(count: usize, place: impl Fn(usize) -> [f32; 3]) -> Structure {
        let mut atoms = Vec::new();
        for index in 0..count {
            let at = place(index);
            for (name, element, offset) in [
                (" N  ", 7u8, [-0.7, 0.3, 0.0]),
                (" CA ", 6, [0.0, 0.0, 0.0]),
                (" C  ", 6, [0.7, 0.3, 0.0]),
                (" O  ", 8, [1.0, 1.0, 0.0]),
                (" CB ", 6, [0.0, -1.5, 0.0]),
            ] {
                atoms.push(atom(
                    name,
                    element,
                    [at[0] + offset[0], at[1] + offset[1], at[2] + offset[2]],
                    ("ALA", index as i32 + 1),
                    "A",
                    false,
                ));
            }
        }
        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        crate::structure::bonds::build(&mut structure, &Default::default());
        structure
    }

    /// Every number a renderer will read is finite, every index is in range, and the
    /// bounds are a box rather than an inverted or infinite one.
    fn assert_drawable(scene: &Scene, what: &str) {
        let buffers: [(&str, &[f32]); 6] = [
            ("sphere instances", &scene.spheres.instances),
            ("sphere colours", &scene.spheres.colors),
            ("cylinder instances", &scene.cylinders.instances),
            ("cylinder colours", &scene.cylinders.colors),
            ("mesh positions", &scene.mesh.positions),
            ("mesh colours", &scene.mesh.colors),
        ];
        for (name, values) in buffers {
            assert!(
                values.iter().all(|value| value.is_finite()),
                "{what}: {name} holds a NaN or an infinity"
            );
        }
        for (index, normal) in scene.mesh.normals.chunks(3).enumerate() {
            let length =
                (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            assert!(
                (length - 1.0).abs() < 1e-3,
                "{what}: normal {index} is {normal:?}"
            );
        }
        assert_eq!(
            scene.mesh.normals.len(),
            scene.mesh.positions.len(),
            "{what}"
        );
        assert_eq!(
            scene.mesh.colors.len(),
            scene.mesh.positions.len(),
            "{what}"
        );
        assert_eq!(
            scene.mesh.vertex_residue.len(),
            scene.mesh.vertex_count(),
            "{what}"
        );
        let vertices = scene.mesh.vertex_count() as u32;
        assert!(
            scene.mesh.indices.iter().all(|index| *index < vertices),
            "{what}: an index points past the vertices"
        );
        for axis in 0..3 {
            assert!(
                scene.bounds_min[axis].is_finite(),
                "{what}: {:?}",
                scene.bounds_min
            );
            assert!(
                scene.bounds_max[axis].is_finite(),
                "{what}: {:?}",
                scene.bounds_max
            );
            assert!(
                scene.bounds_min[axis] <= scene.bounds_max[axis],
                "{what}: inverted bounds"
            );
        }
    }

    const EVERY_STYLE: [&[Representation]; 6] = [
        &[Representation::Cartoon],
        &[Representation::Spacefill],
        &[Representation::BallAndStick],
        &[Representation::Sticks],
        &[Representation::Backbone],
        &[Representation::Cartoon, Representation::Sticks],
    ];

    fn draw_every_way(structure: &Structure, what: &str) {
        for representations in EVERY_STYLE {
            for scheme in [
                ColorScheme::Chain,
                ColorScheme::Secondary,
                ColorScheme::Rainbow,
                ColorScheme::Chainbow,
                ColorScheme::BFactor,
                ColorScheme::Confidence,
            ] {
                for coarse in [false, true] {
                    let scene = build_scene(
                        structure,
                        SceneOptions::default(),
                        &[Assignment::everything(Style {
                            representations: representations.to_vec(),
                            scheme,
                            ..Default::default()
                        })],
                        &super::super::SceneStyle::default(),
                        coarse,
                    );
                    assert_drawable(
                        &scene,
                        &format!("{what}, {representations:?} by {scheme:?}"),
                    );
                }
            }
        }
    }

    #[test]
    fn chains_of_one_two_and_three_residues_draw_cleanly() {
        for count in 1..=3 {
            let structure = peptide(count, |index| [index as f32 * 3.8, 0.0, 0.0]);
            draw_every_way(&structure, &format!("{count} residues"));
        }
        // And as a helix and a strand, which take different paths through the cartoon.
        for secondary in [
            crate::structure::model::SecondaryStructure::Helix,
            crate::structure::model::SecondaryStructure::Sheet,
        ] {
            for count in 1..=3 {
                let mut structure = peptide(count, |index| [index as f32 * 3.8, 0.0, 0.0]);
                for residue in &mut structure.residues {
                    residue.secondary = secondary;
                }
                draw_every_way(&structure, &format!("{count} residues of {secondary:?}"));
            }
        }
    }

    #[test]
    fn a_single_residue_keeps_its_atoms_under_a_cartoon() {
        // No ribbon can be built through one point, so hiding its atoms would leave
        // nothing at all where the residue is.
        let structure = peptide(1, |_| [0.0; 3]);
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::Cartoon],
                ..Default::default()
            },
        );
        assert!(scene.mesh.is_empty());
        assert!(scene.drawn_atoms > 0, "a lone residue vanished");
    }

    #[test]
    fn coincident_atoms_do_not_poison_the_geometry() {
        // Every residue on top of the last: the zero-length tangents and bonds that make a
        // normal NaN, which blanks the whole mesh in the viewport.
        let stacked = peptide(6, |_| [5.0, 5.0, 5.0]);
        draw_every_way(&stacked, "six stacked residues");

        // Every atom of every residue at one point.
        let atoms: Vec<RawAtom> = (0..4)
            .flat_map(|index| {
                [(" N  ", 7u8), (" CA ", 6), (" C  ", 6), (" O  ", 8)].map(|(name, element)| {
                    atom(
                        name,
                        element,
                        [1.0, 2.0, 3.0],
                        ("GLY", index + 1),
                        "A",
                        false,
                    )
                })
            })
            .collect();
        let mut point = Structure::build("t".into(), "test".into(), atoms);
        crate::structure::bonds::build(&mut point, &Default::default());
        draw_every_way(&point, "a chain collapsed to a point");

        // A chain that doubles straight back on itself, the 180 degree turn that has no
        // well-defined frame.
        let hairpin = peptide(6, |index| {
            let x = if index < 3 {
                index as f32
            } else {
                5.0 - index as f32
            };
            [x * 3.8, 0.0, 0.0]
        });
        draw_every_way(&hairpin, "a hairpin");

        // And a dead straight chain along each axis, which is where a fixed helper vector
        // for a cross product goes parallel.
        for axis in 0..3 {
            let line = peptide(5, |index| {
                let mut at = [0.0; 3];
                at[axis] = index as f32 * 3.8;
                at
            });
            draw_every_way(&line, &format!("a straight chain along axis {axis}"));
        }
    }

    #[test]
    fn an_empty_structure_builds_an_empty_scene() {
        let structure = Structure::build("t".into(), "test".into(), Vec::new());
        for representations in EVERY_STYLE {
            let scene = draw(
                &structure,
                Style {
                    representations: representations.to_vec(),
                    ..Default::default()
                },
            );
            assert_eq!(scene.drawn_atoms, 0);
            assert!(scene.spheres.is_empty() && scene.mesh.is_empty());
            assert_drawable(&scene, "empty");
        }
        // No assignments at all is the default style, not a panic.
        let scene = build_scene(
            &structure,
            SceneOptions::default(),
            &[],
            &super::super::SceneStyle::default(),
            false,
        );
        assert_drawable(&scene, "no assignments");
    }

    #[test]
    fn a_structure_of_only_water_frames_nothing_when_water_is_hidden() {
        let atoms: Vec<RawAtom> = (0..5)
            .map(|index| {
                atom(
                    " O  ",
                    8,
                    [index as f32 * 3.0, 0.0, 0.0],
                    ("HOH", index + 1),
                    "W",
                    true,
                )
            })
            .collect();
        let structure = Structure::build("t".into(), "test".into(), atoms);
        let hidden = draw(&structure, Style::default());
        assert_eq!(hidden.drawn_atoms, 0);
        assert_drawable(&hidden, "hidden water");

        let shown = draw_with(
            &structure,
            SceneOptions {
                show_waters: true,
                ..Default::default()
            },
            Style::default(),
        );
        // Under a cartoon, which has no ribbon to give water, they stay as atoms.
        assert_eq!(shown.drawn_atoms, 5);
        assert_drawable(&shown, "shown water");
    }

    #[test]
    fn a_lone_ligand_atom_has_a_box_with_size() {
        // A single ion is the degenerate case for framing: its bounds have to include its
        // radius, or the camera fits a box of zero size and divides by it.
        let structure = Structure::build(
            "t".into(),
            "test".into(),
            vec![atom("ZN  ", 30, [7.0, -3.0, 2.0], ("ZN", 1), "B", true)],
        );
        for representations in EVERY_STYLE {
            let scene = draw(
                &structure,
                Style {
                    representations: representations.to_vec(),
                    ..Default::default()
                },
            );
            assert_eq!(scene.drawn_atoms, 1, "{representations:?}");
            assert_drawable(&scene, "a zinc");
            for axis in 0..3 {
                assert!(
                    scene.bounds_max[axis] - scene.bounds_min[axis] > 0.1,
                    "{representations:?}: {:?} to {:?}",
                    scene.bounds_min,
                    scene.bounds_max
                );
            }
        }
    }

    #[test]
    fn glass_is_a_surface_over_the_atoms_in_their_colours() {
        let structure = protein_with_ligand();
        let red = [0.9, 0.1, 0.1];
        let style = Style {
            representations: vec![Representation::Cartoon, Representation::Glass],
            solid: Some(red),
            ..Style::default()
        };
        let scene = draw(&structure, style.clone());
        assert!(!scene.glass.is_empty(), "glass makes a surface");
        // The glass is not atoms: no spheres for it, and the cartoon still decides.
        assert_eq!(style.primary(), Representation::Cartoon);
        // In the atoms' own colour unless it has one of its own.
        assert!(scene
            .glass
            .colors
            .chunks(3)
            .all(|rgb| (rgb[0] - red[0]).abs() < 1e-6 && (rgb[1] - red[1]).abs() < 1e-6));
        let blue = [0.1, 0.2, 0.9];
        let own = draw(
            &structure,
            Style {
                glass: Some(blue),
                ..style
            },
        );
        assert!(own.glass.colors.chunks(3).all(|rgb| (rgb[2] - blue[2]).abs() < 1e-6));
        // And none without it.
        assert!(draw(&structure, Style::default()).glass.is_empty());
        // Over a few residues only, when a feature asks for it there.
        let feature = Feature {
            chain: "A".into(),
            from: 1,
            to: 1,
            show: vec![Representation::Glass],
            hide: false,
            colour: None,
            ranges: None,
        };
        let some = build_scene_with(
            &structure,
            SceneOptions::default(),
            &[Assignment::everything(Style::default())],
            &[feature],
            &Default::default(),
            false,
        );
        assert!(!some.glass.is_empty());
        assert!(some.glass.vertex_count() < scene.glass.vertex_count());
    }

    #[test]
    fn a_nucleic_acid_strand_draws_cleanly() {
        // Phosphates 6.8 A apart, with a pyrimidine ring on each nucleotide.
        let mut atoms = Vec::new();
        for index in 0..4 {
            let x = index as f32 * 6.8;
            atoms.push(atom(
                " P  ",
                15,
                [x, 0.0, 0.0],
                ("DC", index + 1),
                "A",
                false,
            ));
            atoms.push(atom(
                " C1'",
                6,
                [x, 3.0, 0.0],
                ("DC", index + 1),
                "A",
                false,
            ));
            for (ring, name) in ["N1", "C2", "N3", "C4", "C5", "C6"].iter().enumerate() {
                let angle = ring as f32 / 6.0 * std::f32::consts::TAU;
                atoms.push(atom(
                    name,
                    if name.starts_with('N') { 7 } else { 6 },
                    [x + 1.4 * angle.cos(), 6.0 + 1.4 * angle.sin(), 0.0],
                    ("DC", index + 1),
                    "A",
                    false,
                ));
            }
        }
        let mut structure = Structure::build("t".into(), "test".into(), atoms);
        crate::structure::bonds::build(&mut structure, &Default::default());
        draw_every_way(&structure, "a DNA strand");
        let scene = draw(&structure, Style::default());
        assert!(!scene.mesh.is_empty(), "the strand gets a ribbon");
    }

    fn feature(chain: &str, from: i32, to: i32) -> Feature {
        Feature {
            chain: chain.into(),
            from,
            to,
            show: Vec::new(),
            hide: true,
            colour: None,
            ranges: None,
        }
    }

    fn draw_featured(structure: &Structure, features: &[Feature]) -> Scene {
        build_scene_with(
            structure,
            SceneOptions::default(),
            &[Assignment::everything(Style {
                representations: vec![Representation::Spacefill],
                ..Default::default()
            })],
            features,
            &super::super::SceneStyle::default(),
            false,
        )
    }

    #[test]
    fn a_range_written_backwards_means_the_same_residues() {
        let structure = protein_with_ligand();
        let plain = draw_featured(&structure, &[]);
        let forwards = draw_featured(&structure, &[feature("A", 1, 2)]);
        let backwards = draw_featured(&structure, &[feature("A", 2, 1)]);
        assert!(
            forwards.drawn_atoms < plain.drawn_atoms,
            "the feature hides something"
        );
        assert_eq!(
            backwards.drawn_atoms, forwards.drawn_atoms,
            "and so does it backwards"
        );
    }

    #[test]
    fn a_feature_naming_nothing_changes_nothing() {
        let structure = protein_with_ligand();
        let plain = draw_featured(&structure, &[]);
        for missing in [
            feature("Z", 1, 3),
            feature("A", 500, 900),
            feature("A", -40, -1),
            feature("", 1, 3),
            feature("A", i32::MIN, i32::MIN),
            // Waters are not polymer, so a range over one is not a range over anything.
            feature("B", 100, 100),
        ] {
            let scene = draw_featured(&structure, std::slice::from_ref(&missing));
            assert_eq!(scene.drawn_atoms, plain.drawn_atoms, "{missing:?}");
            assert_drawable(&scene, "a feature naming nothing");
        }
        // The whole number line is every polymer residue, without overflowing.
        let everything = draw_featured(&structure, &[feature("A", i32::MIN, i32::MAX)]);
        assert_eq!(everything.drawn_atoms, 1, "only the magnesium is left");
    }

    #[test]
    fn a_feature_colour_and_extra_representation_reach_the_atoms() {
        let structure = protein_with_ligand();
        let red = [1.0, 0.0, 0.0];
        let scene = build_scene_with(
            &structure,
            SceneOptions::default(),
            &[Assignment::everything(Style {
                representations: vec![Representation::Cartoon],
                heteroatoms_by_element: false,
                ..Default::default()
            })],
            &[Feature {
                chain: "A".into(),
                from: 2,
                to: 2,
                show: vec![Representation::Sticks],
                hide: false,
                colour: Some(red),
                ranges: None,
            }],
            &super::super::SceneStyle::default(),
            false,
        );
        assert_drawable(&scene, "a featured residue");
        // Residue 2's heavy atoms come back as sticks over the ribbon, in the feature's
        // colour.
        let residue = &structure.residues[1];
        let featured: Vec<usize> = (0..scene.spheres.len())
            .filter(|index| {
                let atom = scene.spheres.atoms[*index];
                atom >= residue.first_atom && atom < residue.first_atom + residue.atom_count
            })
            .collect();
        assert!(!featured.is_empty(), "the feature's sticks are drawn");
        for index in featured {
            assert_eq!(&scene.spheres.colors[index * 3..index * 3 + 3], &red);
        }
    }

    #[test]
    fn an_assignment_for_a_chain_that_does_not_exist_is_ignored() {
        let structure = protein_with_ligand();
        let scene = build_scene(
            &structure,
            SceneOptions::default(),
            &[
                Assignment::everything(Style {
                    representations: vec![Representation::Spacefill],
                    ..Default::default()
                }),
                Assignment {
                    chain: Some("no such chain".into()),
                    style: Style {
                        representations: Vec::new(),
                        ..Default::default()
                    },
                },
            ],
            &super::super::SceneStyle::default(),
            false,
        );
        assert_eq!(scene.drawn_atoms, 16);
    }

    #[test]
    fn cartoon_bounds_follow_the_ribbon_not_the_hidden_atoms() {
        let structure = protein_with_ligand();
        let scene = draw(
            &structure,
            Style {
                representations: vec![Representation::Cartoon],
                scheme: ColorScheme::Chain,
                ..Default::default()
            },
        );
        // The ribbon spans the chain near the origin; the magnesium sits at x=50. Both
        // must be inside the bounds, or one of them is off screen.
        assert!(scene.bounds_min[0] < 1.0);
        assert!(scene.bounds_max[0] > 49.0);
    }

    #[test]
    fn compacted_features_draw_the_same_and_are_few() {
        let red = Some([1.0, 0.0, 0.0]);
        let blue = Some([0.0, 0.0, 1.0]);
        let feature = |chain: &str,
                       from: i32,
                       to: i32,
                       show: Vec<Representation>,
                       hide: bool,
                       colour: Option<[f32; 3]>| Feature {
            chain: chain.into(),
            from,
            to,
            show,
            hide,
            colour,
            ranges: None,
        };
        // Coloured red, then part of it blue, then all of it red again, residue by residue:
        // the way repeated colouring piles features up.
        let mut piled = vec![
            feature("A_1", 1, 10, vec![], false, red),
            feature("A_1", 4, 6, vec![], false, blue),
        ];
        for number in 1..=10 {
            piled.push(feature("A_1", number, number, vec![], false, red));
        }
        piled.push(feature(
            "A_1",
            8,
            9,
            vec![Representation::Sticks],
            false,
            None,
        ));
        piled.push(feature("B_1", 3, 3, vec![], true, None));
        let compact = compact_features(&piled);
        // The two red runs either side of the sticks are one feature with two ranges.
        let mut both = feature("A_1", 1, 10, vec![], false, red);
        both.ranges = Some("1-7,10".into());
        assert_eq!(
            compact,
            vec![
                both,
                feature("A_1", 8, 9, vec![Representation::Sticks], false, red),
                feature("B_1", 3, 3, vec![], true, None),
            ]
        );
        assert_eq!(compact[0].spans(), vec![(1, 7), (10, 10)]);
        // Compacting what is compact changes nothing.
        assert_eq!(compact_features(&compact), compact);
    }

    #[test]
    fn a_range_too_long_to_spread_out_is_kept_as_it_was() {
        let wide = Feature {
            chain: "A".into(),
            from: 1,
            to: 100_000,
            show: vec![],
            hide: true,
            colour: None,
            ranges: None,
        };
        assert_eq!(compact_features(std::slice::from_ref(&wide)), vec![wide]);
    }
}
