//! Colour data.
//!
//! Three sources, all reusable, each chosen for a different job:
//!
//! - **Open Color** (MIT, github.com/yeun/open-color): 13 hues x 10 shades, tuned for
//!   screens. This is the swatch grid a user picks from, and the source of the vivid
//!   defaults.
//! - **Okabe-Ito** (Okabe & Ito, Color Universal Design): eight categorical colours that
//!   stay distinguishable under every common form of colour vision deficiency. The default
//!   for chains, because "which chain is which" must not depend on the reader's eyes.
//! - **Viridis** (matplotlib, CC0): perceptually uniform, for continuous values.
//!
//! AlphaFold's pLDDT bands are deliberately *not* restyled - see `super::color`.

use serde::Serialize;

/// Packs a hex literal into bytes, so the tables below read like the sources they came
/// from rather than triples of decimals.
const fn rgb(hex: u32) -> [u8; 3] {
    [
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    ]
}

/// One named hue from Open Color, shade 0 (lightest) to 9 (darkest).
pub struct Hue {
    pub name: &'static str,
    pub shades: [[u8; 3]; 10],
}

/// Open Color, complete. The UI's swatch grid is this table.
pub const OPEN_COLOR: &[Hue] = &[
    Hue {
        name: "gray",
        shades: [
            rgb(0xf8f9fa),
            rgb(0xf1f3f5),
            rgb(0xe9ecef),
            rgb(0xdee2e6),
            rgb(0xced4da),
            rgb(0xadb5bd),
            rgb(0x868e96),
            rgb(0x495057),
            rgb(0x343a40),
            rgb(0x212529),
        ],
    },
    Hue {
        name: "red",
        shades: [
            rgb(0xfff5f5),
            rgb(0xffe3e3),
            rgb(0xffc9c9),
            rgb(0xffa8a8),
            rgb(0xff8787),
            rgb(0xff6b6b),
            rgb(0xfa5252),
            rgb(0xf03e3e),
            rgb(0xe03131),
            rgb(0xc92a2a),
        ],
    },
    Hue {
        name: "pink",
        shades: [
            rgb(0xfff0f6),
            rgb(0xffdeeb),
            rgb(0xfcc2d7),
            rgb(0xfaa2c1),
            rgb(0xf783ac),
            rgb(0xf06595),
            rgb(0xe64980),
            rgb(0xd6336c),
            rgb(0xc2255c),
            rgb(0xa61e4d),
        ],
    },
    Hue {
        name: "grape",
        shades: [
            rgb(0xf8f0fc),
            rgb(0xf3d9fa),
            rgb(0xeebefa),
            rgb(0xe599f7),
            rgb(0xda77f2),
            rgb(0xcc5de8),
            rgb(0xbe4bdb),
            rgb(0xae3ec9),
            rgb(0x9c36b5),
            rgb(0x862e9c),
        ],
    },
    Hue {
        name: "violet",
        shades: [
            rgb(0xf3f0ff),
            rgb(0xe5dbff),
            rgb(0xd0bfff),
            rgb(0xb197fc),
            rgb(0x9775fa),
            rgb(0x845ef7),
            rgb(0x7950f2),
            rgb(0x7048e8),
            rgb(0x6741d9),
            rgb(0x5f3dc4),
        ],
    },
    Hue {
        name: "indigo",
        shades: [
            rgb(0xedf2ff),
            rgb(0xdbe4ff),
            rgb(0xbac8ff),
            rgb(0x91a7ff),
            rgb(0x748ffc),
            rgb(0x5c7cfa),
            rgb(0x4c6ef5),
            rgb(0x4263eb),
            rgb(0x3b5bdb),
            rgb(0x364fc7),
        ],
    },
    Hue {
        name: "blue",
        shades: [
            rgb(0xe7f5ff),
            rgb(0xd0ebff),
            rgb(0xa5d8ff),
            rgb(0x74c0fc),
            rgb(0x4dabf7),
            rgb(0x339af0),
            rgb(0x228be6),
            rgb(0x1c7ed6),
            rgb(0x1971c2),
            rgb(0x1864ab),
        ],
    },
    Hue {
        name: "cyan",
        shades: [
            rgb(0xe3fafc),
            rgb(0xc5f6fa),
            rgb(0x99e9f2),
            rgb(0x66d9e8),
            rgb(0x3bc9db),
            rgb(0x22b8cf),
            rgb(0x15aabf),
            rgb(0x1098ad),
            rgb(0x0c8599),
            rgb(0x0b7285),
        ],
    },
    Hue {
        name: "teal",
        shades: [
            rgb(0xe6fcf5),
            rgb(0xc3fae8),
            rgb(0x96f2d7),
            rgb(0x63e6be),
            rgb(0x38d9a9),
            rgb(0x20c997),
            rgb(0x12b886),
            rgb(0x0ca678),
            rgb(0x099268),
            rgb(0x087f5b),
        ],
    },
    Hue {
        name: "green",
        shades: [
            rgb(0xebfbee),
            rgb(0xd3f9d8),
            rgb(0xb2f2bb),
            rgb(0x8ce99a),
            rgb(0x69db7c),
            rgb(0x51cf66),
            rgb(0x40c057),
            rgb(0x37b24d),
            rgb(0x2f9e44),
            rgb(0x2b8a3e),
        ],
    },
    Hue {
        name: "lime",
        shades: [
            rgb(0xf4fce3),
            rgb(0xe9fac8),
            rgb(0xd8f5a2),
            rgb(0xc0eb75),
            rgb(0xa9e34b),
            rgb(0x94d82d),
            rgb(0x82c91e),
            rgb(0x74b816),
            rgb(0x66a80f),
            rgb(0x5c940d),
        ],
    },
    Hue {
        name: "yellow",
        shades: [
            rgb(0xfff9db),
            rgb(0xfff3bf),
            rgb(0xffec99),
            rgb(0xffe066),
            rgb(0xffd43b),
            rgb(0xfcc419),
            rgb(0xfab005),
            rgb(0xf59f00),
            rgb(0xf08c00),
            rgb(0xe67700),
        ],
    },
    Hue {
        name: "orange",
        shades: [
            rgb(0xfff4e6),
            rgb(0xffe8cc),
            rgb(0xffd8a8),
            rgb(0xffc078),
            rgb(0xffa94d),
            rgb(0xff922b),
            rgb(0xfd7e14),
            rgb(0xf76707),
            rgb(0xe8590c),
            rgb(0xd9480f),
        ],
    },
];

/// Okabe-Ito, in the order the authors give it. Black is replaced by a dark gray: on a
/// white background a pure black chain reads as a hole rather than a colour.
pub const OKABE_ITO: &[[u8; 3]] = &[
    rgb(0x0072b2), // blue
    rgb(0xe69f00), // orange
    rgb(0x009e73), // bluish green
    rgb(0xcc79a7), // reddish purple
    rgb(0x56b4e9), // sky blue
    rgb(0xd55e00), // vermillion
    rgb(0xf0e442), // yellow
    rgb(0x3b4252), // dark slate, standing in for black
];

/// Vivid categorical set from Open Color, for when maximum separation matters more than
/// colour-vision safety - a 20-chain assembly, say, where eight colours are not enough.
pub const VIVID: &[[u8; 3]] = &[
    rgb(0x228be6), // blue 6
    rgb(0xfa5252), // red 6
    rgb(0x40c057), // green 6
    rgb(0xfab005), // yellow 6
    rgb(0xbe4bdb), // grape 6
    rgb(0x12b886), // teal 6
    rgb(0xfd7e14), // orange 6
    rgb(0xe64980), // pink 6
    rgb(0x4c6ef5), // indigo 6
    rgb(0x82c91e), // lime 6
    rgb(0x15aabf), // cyan 6
    rgb(0x7950f2), // violet 6
];

/// Paul Tol's Bright qualitative scheme (SRON technical note EPS/TN/09-002), grey last.
pub const TOL_BRIGHT: &[[u8; 3]] = &[
    rgb(0x4477aa),
    rgb(0xee6677),
    rgb(0x228833),
    rgb(0xccbb44),
    rgb(0x66ccee),
    rgb(0xaa3377),
    rgb(0xbbbbbb),
];

/// Paul Tol's Muted qualitative scheme, from the same note.
pub const TOL_MUTED: &[[u8; 3]] = &[
    rgb(0xcc6677),
    rgb(0x332288),
    rgb(0xddcc77),
    rgb(0x117733),
    rgb(0x88ccee),
    rgb(0x882255),
    rgb(0x44aa99),
    rgb(0x999933),
    rgb(0xaa4499),
];

/// Tableau 10, as D3 and matplotlib carry it.
pub const TABLEAU_10: &[[u8; 3]] = &[
    rgb(0x4e79a7),
    rgb(0xf28e2b),
    rgb(0xe15759),
    rgb(0x76b7b2),
    rgb(0x59a14f),
    rgb(0xedc948),
    rgb(0xb07aa1),
    rgb(0xff9da7),
    rgb(0x9c755f),
    rgb(0xbab0ac),
];

/// ColorBrewer Set2 (Cynthia Brewer, Apache 2.0).
pub const SET2: &[[u8; 3]] = &[
    rgb(0x66c2a5),
    rgb(0xfc8d62),
    rgb(0x8da0cb),
    rgb(0xe78ac3),
    rgb(0xa6d854),
    rgb(0xffd92f),
    rgb(0xe5c494),
    rgb(0xb3b3b3),
];

/// Eight hues taken about half way to grey, in an order that keeps neighbours apart.
pub const SOFT: &[[u8; 3]] = &[
    rgb(0x86a8cf),
    rgb(0xdba77f),
    rgb(0x8cc0a4),
    rgb(0xc79bbd),
    rgb(0xb3a8d8),
    rgb(0xd3c486),
    rgb(0x8fc3cb),
    rgb(0xd49a9a),
];

/// Which categorical palette a scene uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Categorical {
    /// Colour-vision safe. The default, because which chain is which must not depend on
    /// the reader's eyes.
    #[default]
    OkabeIto,
    /// Twelve vivid hues, for many categories.
    Vivid,
    /// Paul Tol's Bright: seven, colour-vision safe, a little livelier than Okabe-Ito.
    TolBright,
    /// Paul Tol's Muted: nine, colour-vision safe, quiet enough to sit under labels.
    TolMuted,
    /// Tableau 10: the ten most charts are drawn in.
    Tableau10,
    /// ColorBrewer's Set2: eight pastels, for a figure that has to print softly.
    Set2,
    /// This app's own quiet set: eight hues at about half strength, for looking at for
    /// hours rather than for telling twenty chains apart.
    Soft,
}

impl Categorical {
    pub fn colors(self) -> &'static [[u8; 3]] {
        match self {
            Categorical::OkabeIto => OKABE_ITO,
            Categorical::Vivid => VIVID,
            Categorical::TolBright => TOL_BRIGHT,
            Categorical::TolMuted => TOL_MUTED,
            Categorical::Tableau10 => TABLEAU_10,
            Categorical::Set2 => SET2,
            Categorical::Soft => SOFT,
        }
    }

    pub fn pick(self, index: usize) -> [u8; 3] {
        let colors = self.colors();
        colors[index % colors.len()]
    }
}

/// Secondary structure. Open Color's red 6, yellow 6 and teal 5, each taken thirty
/// percent of the way toward its own grey.
///
/// At full strength they were the first thing anyone saw, which was the idea, and the
/// only thing, which was not: a helical protein in the default style is a screenful of
/// one saturated red on white, and it glares. Mixed toward grey - the grey of equal
/// luminance, so the three keep their order of lightness - helix and sheet are still the
/// two things the eye finds first, in colours that can be looked at for an afternoon.
///
/// And then taken further the same way, twice: at thirty percent they still glared on a
/// light background in a dark window, where the viewport is the brightest thing on the
/// screen. These are about half way to grey - a helix is plainly red and a strand plainly
/// gold, and neither is the first thing the eye is pulled to.
pub const HELIX: [u8; 3] = rgb(0xc98686);
pub const SHEET: [u8; 3] = rgb(0xd8bd7c);
pub const TURN: [u8; 3] = rgb(0x86bfae);
pub const COIL_ON_DARK: [u8; 3] = rgb(0xced4da); // gray 4
pub const COIL_ON_LIGHT: [u8; 3] = rgb(0x868e96); // gray 6

/// Solvent, drawn to recede on either background.
pub const SOLVENT_ON_DARK: [u8; 3] = rgb(0x5c6773);
pub const SOLVENT_ON_LIGHT: [u8; 3] = rgb(0xadb5bd);

/// Viridis, sampled at nine anchors and interpolated. Perceptually uniform, so equal
/// steps in value look like equal steps in colour - which a rainbow scale does not.
const VIRIDIS: &[[u8; 3]] = &[
    rgb(0x440154),
    rgb(0x482878),
    rgb(0x3e4a89),
    rgb(0x31688e),
    rgb(0x26828e),
    rgb(0x1f9e89),
    rgb(0x35b779),
    rgb(0x6ece58),
    rgb(0xfde725),
];

/// `t` kept to 0..1 for a ramp, with NaN taken as the start.
///
/// `clamp` passes NaN through, and a NaN position became a colour channel of 0 by way of
/// a float-to-byte cast: an atom with an unreadable B-factor drew black under viridis and
/// a half-red under the rainbow, rather than as the low end of the scale.
fn unit(t: f32) -> f32 {
    if t.is_nan() {
        0.0
    } else {
        t.clamp(0.0, 1.0)
    }
}

/// Samples viridis at `t` in 0..1.
pub fn viridis(t: f32) -> [u8; 3] {
    let t = unit(t) * (VIRIDIS.len() - 1) as f32;
    let low = t.floor() as usize;
    let high = (low + 1).min(VIRIDIS.len() - 1);
    let fraction = t - low as f32;
    let mut out = [0u8; 3];
    for channel in 0..3 {
        let a = VIRIDIS[low][channel] as f32;
        let b = VIRIDIS[high][channel] as f32;
        out[channel] = (a + (b - a) * fraction).round() as u8;
    }
    out
}

/// A rainbow ramp: blue at 0, red at 1.
///
/// PyMOL's `rainbow` palette is `blue cyan green yellow orange red`, interpolated in RGB
/// (`pymol/viewing.py`), and those six are pure hues - so a hue sweep from 240 degrees to
/// 0 lands on the same colours and spaces them evenly, rather than copying PyMOL's own
/// colour table. Evenly is the small difference: PyMOL's equal-spaced anchors put orange
/// at 0.8 where a hue sweep puts it at 0.875.
pub fn rainbow(t: f32) -> [u8; 3] {
    let hue = 240.0 * (1.0 - unit(t));
    // The same sweep of hue, short of full strength. At saturation and value 1 every
    // colour in it is a primary or a secondary at its loudest - pure green, pure magenta
    // - and a tetramer coloured that way on a white ground is hard to look at. The ends
    // are still plainly blue and red, which is all a chainbow needs of them.
    const SATURATION: f32 = 0.62;
    const VALUE: f32 = 0.92;
    let sector = hue / 60.0;
    let x = 1.0 - (sector % 2.0 - 1.0).abs();
    let (r, g, b) = match sector as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        _ => (x, 0.0, 1.0),
    };
    let tone =
        |channel: f32| ((VALUE * (1.0 - SATURATION + SATURATION * channel)) * 255.0).round() as u8;
    [tone(r), tone(g), tone(b)]
}

/// Blue to white to red - the ramp a crystallographer expects over B-factors.
pub fn blue_white_red(t: f32) -> [u8; 3] {
    let t = unit(t);
    let lerp = |from: [f32; 3], to: [f32; 3], amount: f32| {
        [
            (from[0] + (to[0] - from[0]) * amount) * 255.0,
            (from[1] + (to[1] - from[1]) * amount) * 255.0,
            (from[2] + (to[2] - from[2]) * amount) * 255.0,
        ]
    };
    let rgb = if t < 0.5 {
        lerp([0.20, 0.35, 0.85], [0.95, 0.95, 0.95], t * 2.0)
    } else {
        lerp([0.95, 0.95, 0.95], [0.85, 0.20, 0.20], (t - 0.5) * 2.0)
    };
    [
        rgb[0].round() as u8,
        rgb[1].round() as u8,
        rgb[2].round() as u8,
    ]
}

/// CPK element colours, adjusted for the background they will be seen on.
///
/// The convention is fixed for oxygen, nitrogen and sulfur, and nobody would thank us for
/// changing them. Carbon and hydrogen are the problem: CPK's light gray carbon and white
/// hydrogen are invisible on a white background, which is now the default. So those two -
/// and only those two - darken when the background is light. Every viewer that supports
/// white backgrounds does some version of this.
/// Which element table and which secondary-structure colours are in use.
///
/// The app's, not a scene's: how carbon and a helix are coloured is a preference about
/// every figure this person makes, so it is set once (from the preferences, at start and
/// whenever they change) and read here rather than threaded through every style.
static ELEMENT_TABLE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static SECONDARY_SET: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Sets the two from their names in the preferences. A name not known leaves the default.
pub fn set_theme(elements: Option<&str>, secondary: Option<&str>) {
    use std::sync::atomic::Ordering::Relaxed;
    ELEMENT_TABLE.store(
        match elements {
            Some("cpk") => 1,
            Some("pymol") => 2,
            _ => 0,
        },
        Relaxed,
    );
    SECONDARY_SET.store(
        match secondary {
            Some("classic") => 1,
            Some("cool") => 2,
            _ => 0,
        },
        Relaxed,
    );
}

/// Helix, strand and turn, in the set the preferences name.
pub fn secondary_colors() -> ([u8; 3], [u8; 3], [u8; 3]) {
    match SECONDARY_SET.load(std::sync::atomic::Ordering::Relaxed) {
        // The textbook three, at full strength: helix red, strand yellow, turn green.
        1 => (rgb(0xe03131), rgb(0xf5c211), rgb(0x2f9e44)),
        // Blues and a violet, for a figure whose reds are spoken for.
        2 => (rgb(0x4c7fd0), rgb(0x41b8c4), rgb(0x8f7fd6)),
        _ => (HELIX, SHEET, TURN),
    }
}

/// The common elements as two other conventions colour them; the rest keep this app's.
fn themed_element(element: u8) -> Option<[u8; 3]> {
    match ELEMENT_TABLE.load(std::sync::atomic::Ordering::Relaxed) {
        // Corey-Pauling-Koltun, as the plastic models were: black carbon, sky nitrogen.
        1 => Some(match element {
            1 => rgb(0xffffff),
            6 => rgb(0x333333),
            7 => rgb(0x2233ff),
            8 => rgb(0xff2200),
            15 => rgb(0xffa500),
            16 => rgb(0xffcc22),
            9 | 17 => rgb(0x1ff01f),
            _ => return None,
        }),
        // PyMOL's: the green carbon every structural biologist has looked at for years.
        2 => Some(match element {
            1 => rgb(0xe6e6e6),
            6 => rgb(0x33ff33),
            7 => rgb(0x3333ff),
            8 => rgb(0xff4d4d),
            15 => rgb(0xff8000),
            16 => rgb(0xe6c640),
            _ => return None,
        }),
        _ => None,
    }
}

pub fn element_color(element: u8, light_background: bool) -> [u8; 3] {
    if let Some(color) = themed_element(element) {
        return color;
    }
    match element {
        1 => {
            if light_background {
                rgb(0x868e96) // gray 6
            } else {
                rgb(0xffffff)
            }
        }
        6 => {
            if light_background {
                rgb(0x495057) // gray 7
            } else {
                rgb(0xced4da) // gray 4
            }
        }
        _ => crate::structure::chem::color(element),
    }
}

/// True when a background is light enough that light foregrounds vanish into it.
///
/// Rec. 709 luma, because that is what perceived brightness follows - a saturated yellow
/// background is "light" even though its blue channel is near zero.
pub fn is_light(background: [f32; 3]) -> bool {
    0.2126 * background[0] + 0.7152 * background[1] + 0.0722 * background[2] > 0.5
}

/// The swatch grid the colour picker shows.
#[derive(Debug, Clone, Serialize)]
pub struct SwatchGroup {
    pub name: String,
    pub colors: Vec<String>,
}

/// Every palette as hex strings, for the UI.
pub fn swatches() -> Vec<SwatchGroup> {
    let hex = |color: [u8; 3]| format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]);

    let mut groups = vec![
        SwatchGroup {
            name: "Colour-vision safe".into(),
            colors: OKABE_ITO.iter().map(|c| hex(*c)).collect(),
        },
        SwatchGroup {
            name: "Vivid".into(),
            colors: VIVID.iter().map(|c| hex(*c)).collect(),
        },
    ];
    // Open Color's own shade ladders, which is what makes picking a *tint* easy rather
    // than making the user drag a slider.
    for hue in OPEN_COLOR {
        groups.push(SwatchGroup {
            name: hue.name.to_string(),
            colors: hue.shades.iter().map(|c| hex(*c)).collect(),
        });
    }
    groups
}

/// A nucleotide's colour by its base, in the convention the nucleic acid database uses -
/// A red, T blue, U cyan, G green, C yellow - softened to sit with the rest of this palette.
pub fn base_color(residue_name: &str) -> Option<[u8; 3]> {
    Some(match residue_name.trim().trim_start_matches('D') {
        "A" => rgb(0xd9625e),
        "T" => rgb(0x5b8fd9),
        "U" => rgb(0x4fb3c4),
        "G" => rgb(0x59b38c),
        "C" => rgb(0xe0b84a),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_literals_unpack_correctly() {
        assert_eq!(rgb(0xff6b6b), [255, 107, 107]);
        assert_eq!(rgb(0x000000), [0, 0, 0]);
        assert_eq!(rgb(0xffffff), [255, 255, 255]);
    }

    #[test]
    fn open_color_is_complete() {
        assert_eq!(OPEN_COLOR.len(), 13);
        for hue in OPEN_COLOR {
            assert_eq!(hue.shades.len(), 10, "{} is short", hue.name);
            // Shades run light to dark, which is what the picker's ladder relies on.
            let first = luma(hue.shades[0]);
            let last = luma(hue.shades[9]);
            assert!(first > last, "{} is not ordered light to dark", hue.name);
        }
    }

    #[test]
    fn categorical_palettes_wrap_rather_than_panic() {
        // A 40-chain assembly must still get a colour for every chain.
        for index in 0..40 {
            let color = Categorical::OkabeIto.pick(index);
            assert!(color.iter().any(|channel| *channel > 0));
        }
    }

    #[test]
    fn carbon_and_hydrogen_flip_with_the_background() {
        // The reason this exists: CPK carbon and hydrogen are invisible on white.
        let on_dark = element_color(6, false);
        let on_light = element_color(6, true);
        assert!(luma(on_dark) > luma(on_light));
        assert!(luma(element_color(1, false)) > luma(element_color(1, true)));

        // Oxygen is a convention and does not move.
        assert_eq!(element_color(8, true), element_color(8, false));
    }

    #[test]
    fn light_backgrounds_are_recognised_by_perceived_brightness() {
        assert!(is_light([1.0, 1.0, 1.0]));
        assert!(!is_light([0.04, 0.05, 0.07]));
        // A saturated yellow is light despite having almost no blue.
        assert!(is_light([1.0, 0.85, 0.1]));
        // A saturated blue is not, despite being fully on in one channel.
        assert!(!is_light([0.0, 0.0, 1.0]));
    }

    #[test]
    fn viridis_runs_from_purple_to_yellow_without_gaps() {
        let low = viridis(0.0);
        let high = viridis(1.0);
        assert_eq!(low, [68, 1, 84]);
        assert_eq!(high, [253, 231, 37]);
        // Monotone in luma is what makes it readable as a scale.
        let mut previous = luma(low);
        for step in 1..=20 {
            let current = luma(viridis(step as f32 / 20.0));
            assert!(current >= previous - 2.0, "luma dipped at {step}");
            previous = current;
        }
        // Out of range values clamp rather than wrap.
        assert_eq!(viridis(-1.0), low);
        assert_eq!(viridis(2.0), high);
    }

    #[test]
    fn the_swatch_grid_is_valid_hex() {
        let groups = swatches();
        assert!(groups.len() > 10);
        for group in groups {
            for color in group.colors {
                assert_eq!(color.len(), 7, "{color} is not #rrggbb");
                assert!(color.starts_with('#'));
                assert!(u32::from_str_radix(&color[1..], 16).is_ok(), "{color}");
            }
        }
    }

    #[test]
    fn every_palette_hands_out_a_colour_for_any_index() {
        for palette in [
            Categorical::OkabeIto,
            Categorical::Vivid,
            Categorical::TolBright,
            Categorical::TolMuted,
            Categorical::Tableau10,
            Categorical::Set2,
            Categorical::Soft,
        ] {
            let size = palette.colors().len();
            assert!(size > 0, "{palette:?} is empty");
            // Past the end wraps to the start, all the way out to the largest index.
            assert_eq!(palette.pick(size), palette.pick(0), "{palette:?}");
            let _ = palette.pick(usize::MAX);
        }
    }

    #[test]
    fn a_ramp_given_nan_draws_its_low_end_not_black() {
        // An unreadable B-factor reaches the ramp as NaN. Cast to a byte, every channel
        // of it was 0: a black atom under viridis, a half-red one under the rainbow.
        assert_eq!(viridis(f32::NAN), viridis(0.0));
        assert_eq!(rainbow(f32::NAN), rainbow(0.0));
        assert_eq!(blue_white_red(f32::NAN), blue_white_red(0.0));
        // Infinities clamp like any other out-of-range value.
        assert_eq!(viridis(f32::INFINITY), viridis(1.0));
        assert_eq!(rainbow(f32::NEG_INFINITY), rainbow(0.0));
        assert_eq!(blue_white_red(f32::INFINITY), blue_white_red(1.0));
    }

    #[test]
    fn the_rainbow_never_wraps_back_to_blue_or_past_red() {
        for step in 0..=1000 {
            let colour = rainbow(step as f32 / 1000.0);
            // The sweep is blue to red through green; magenta (red and blue both high,
            // green low) never belongs in it.
            let magenta = colour[0] > 150 && colour[2] > 150 && colour[1] < 120;
            assert!(!magenta, "{step}: {colour:?}");
        }
    }

    #[test]
    fn bases_are_named_by_their_letter_with_or_without_the_deoxy_d() {
        assert_eq!(base_color("DA"), base_color("A"));
        assert_eq!(base_color(" DT "), base_color("T"));
        assert!(base_color("U").is_some());
        // Not a base: no colour, rather than a guess.
        for name in ["", "D", "PSU", "HOH", "ALA"] {
            assert!(base_color(name).is_none(), "{name}");
        }
    }

    #[test]
    fn an_unknown_theme_name_leaves_the_defaults() {
        // The preference is free text in a settings file. Something this build does not
        // know must give the shipped colours, not a panic or a blank table.
        set_theme(Some("solarized"), Some("neon"));
        assert_eq!(secondary_colors(), (HELIX, SHEET, TURN));
        set_theme(None, None);
        assert_eq!(secondary_colors(), (HELIX, SHEET, TURN));
    }

    fn luma(color: [u8; 3]) -> f32 {
        0.2126 * color[0] as f32 + 0.7152 * color[1] as f32 + 0.0722 * color[2] as f32
    }
}
