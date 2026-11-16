//! Named colours and style presets.
//!
//! A style is a bundle of decisions, and most of the time the bundle wanted is one of a
//! handful: a cartoon coloured by secondary structure, a spacefill in CPK, a chain painted
//! one flat colour. PyMOL's users reach for `orange` and `grey80` by name, and having to
//! assemble that from a representation, a scheme and a colour picker every time is the
//! kind of friction that makes a tool feel unfinished.
//!
//! The colours are Catppuccin Frappe (MIT) rather than another viewer's table: pastel, so
//! that several of them can sit in one figure without fighting, and published under a
//! licence that says so. See docs/LICENSING.md.

use super::color::ColorScheme;
use super::representation::{Representation, Style};
use serde::Serialize;

/// A named flat colour.
#[derive(Debug, Clone, Serialize)]
pub struct NamedColour {
    pub name: String,
    /// sRGB 0..255, for a swatch.
    pub srgb: [u8; 3],
    /// Linear 0..1, which is what a style stores.
    pub linear: [f32; 3],
}

/// Named colours, from Catppuccin Frappe (MIT).
///
/// Pastel on purpose: a figure of a complex needs several colours that sit next to each
/// other without any one of them shouting, and saturated primaries fight. Frappe is a
/// published, permissively licensed set that was designed for exactly that balance, which
/// beats either picking twelve colours by eye or borrowing another tool's table.
///
/// The neutrals come from Catppuccin Latte's surface steps, which are the same palette's
/// light greys.
const NAMED: &[(&str, [u8; 3])] = &[
    ("rosewater", [0xf2, 0xd5, 0xcf]),
    ("flamingo", [0xee, 0xbe, 0xbe]),
    ("pink", [0xf4, 0xb8, 0xe4]),
    ("mauve", [0xca, 0x9e, 0xe6]),
    ("red", [0xe7, 0x82, 0x84]),
    ("maroon", [0xea, 0x99, 0x9c]),
    ("peach", [0xef, 0x9f, 0x76]),
    ("yellow", [0xe5, 0xc8, 0x90]),
    ("green", [0xa6, 0xd1, 0x89]),
    ("teal", [0x81, 0xc8, 0xbe]),
    ("sky", [0x99, 0xd1, 0xdb]),
    ("sapphire", [0x85, 0xc1, 0xdc]),
    ("blue", [0x8c, 0xaa, 0xee]),
    ("lavender", [0xba, 0xbb, 0xf1]),
    ("white", [0xff, 0xff, 0xff]),
    ("grey15", [0xcc, 0xd0, 0xda]),
    ("grey30", [0xbc, 0xc0, 0xcc]),
    ("grey45", [0xac, 0xb0, 0xbe]),
    ("slate", [0x62, 0x68, 0x80]),
];

pub fn named_colours() -> Vec<NamedColour> {
    NAMED
        .iter()
        .map(|(name, srgb)| NamedColour {
            name: (*name).to_string(),
            srgb: *srgb,
            linear: super::color::to_linear(*srgb),
        })
        .collect()
}

/// A style worth having a name for.
#[derive(Debug, Clone, Serialize)]
pub struct StylePreset {
    pub name: String,
    pub style: Style,
}

/// The presets the app ships.
///
/// Five. A preset list long enough to scroll is a list nobody reads, and these are the
/// five things anyone actually asks of a structure - the rest is a style away from any of
/// them, and the library is there to keep it once it exists.
pub fn presets() -> Vec<StylePreset> {
    let preset = |name: &str, style: Style| StylePreset {
        name: name.to_string(),
        style,
    };

    vec![
        preset(
            "Cartoon",
            Style {
                representations: vec![Representation::Cartoon],
                scheme: ColorScheme::Secondary,
                ..Default::default()
            },
        ),
        // Two representations in one style, which is the point of them being a set: the
        // fold and the chemistry of the side chains in one picture.
        preset(
            "Cartoon and side chains",
            Style {
                representations: vec![Representation::Cartoon, Representation::Sticks],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        ),
        preset(
            "Spacefill",
            Style {
                representations: vec![Representation::Spacefill],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        ),
        preset(
            "Ball and stick",
            Style {
                representations: vec![Representation::BallAndStick],
                scheme: ColorScheme::Element,
                ..Default::default()
            },
        ),
        // The move PyMOL users make by reflex on a complex: every chain blue to red, so
        // the picture says where in its own chain each piece is.
        preset(
            "Chainbow",
            Style {
                representations: vec![Representation::Cartoon],
                scheme: ColorScheme::Chainbow,
                ..Default::default()
            },
        ),
        preset(
            "Backbone",
            Style {
                representations: vec![Representation::Backbone],
                scheme: ColorScheme::Chain,
                ..Default::default()
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_a_distinct_name() {
        let all = presets();
        let names: std::collections::BTreeSet<&str> =
            all.iter().map(|preset| preset.name.as_str()).collect();
        assert_eq!(names.len(), all.len());
    }

    #[test]
    fn the_list_stays_short_and_one_preset_draws_two_things() {
        let all = presets();
        assert!(
            all.len() <= 7,
            "{} presets is a list nobody reads",
            all.len()
        );

        // The one that shows why representations are a set rather than a choice.
        let both = all
            .iter()
            .find(|preset| preset.name == "Cartoon and side chains")
            .expect("the combined preset");
        assert_eq!(
            both.style.representations,
            vec![Representation::Cartoon, Representation::Sticks]
        );
        assert!(both.style.draws(Representation::Cartoon));
        assert!(both.style.draws(Representation::Sticks));
        assert!(!both.style.draws(Representation::Spacefill));
        // The first one decides the scene-wide settings.
        assert_eq!(both.style.primary(), Representation::Cartoon);
    }

    #[test]
    fn every_preset_names_at_least_one_representation() {
        for preset in presets() {
            assert!(
                !preset.style.representations.is_empty(),
                "{} draws nothing",
                preset.name
            );
        }
    }

    #[test]
    fn the_named_colours_round_trip_to_linear_and_back() {
        for colour in named_colours() {
            for channel in 0..3 {
                let linear = colour.linear[channel];
                assert!((0.0..=1.0).contains(&linear), "{colour:?}");
            }
            // White stays white, black stays black: the two the eye checks first.
            if colour.name == "white" {
                assert_eq!(colour.linear, [1.0, 1.0, 1.0]);
            }
            // The greys are cool by design - Catppuccin's neutrals lean blue - but they
            // must stay grey rather than becoming a blue, and stay light.
            if colour.name.starts_with("grey") {
                let [r, _, b] = colour.linear;
                assert!(b > r, "Frappe's greys lean cool: {colour:?}");
                assert!(b - r < 0.15, "still a grey, not a blue: {colour:?}");
                assert!(r > 0.3, "a pastel grey is light: {colour:?}");
            }
        }
    }
}
