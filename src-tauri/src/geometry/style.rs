//! How a scene is lit and coloured.
//!
//! One definition, used by three consumers: the colour schemes (which need the background
//! to keep CPK readable), the interactive viewport, and the Blender renderer. The
//! alternative - the viewport lighting itself and the render script lighting itself - is
//! how a preview stops predicting its own render, which is the one thing a preview is for.
//!
//! The frontend owns the editable copy and sends it with each request. Nothing is stored
//! here, so there is no state to get out of sync.

use super::palette::Categorical;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LightKind {
    /// Parallel rays. Energy is irradiance in W/m^2, so it is independent of distance and
    /// of how large the structure is - the same rig lights a ligand and a capsid.
    Sun,
    /// Falls off with distance. Placed relative to the structure's radius.
    Point,
    /// A soft rectangular source, for the gentlest shadows.
    Area,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Light {
    pub id: String,
    pub name: String,
    pub kind: LightKind,
    /// Linear RGB, 0..1.
    pub color: [f32; 3],
    /// Irradiance for a sun; watts for a point or area light.
    pub energy: f32,
    /// Unit vector from the subject *towards* the light.
    ///
    /// Must be normalised: for a sun only its direction is read, but a point or area light
    /// is *placed* at `direction * distance * radius`, so a short vector quietly moves the
    /// light closer than the distance says it is.
    pub direction: [f32; 3],
    /// Distance from the subject in units of its radius, for point and area lights.
    #[serde(default = "default_distance")]
    pub distance: f32,
    /// Angular diameter in degrees for a sun, or emitter size in radius units for an area
    /// light. Larger means softer shadows.
    #[serde(default = "default_softness")]
    pub softness: f32,
    /// Whether `direction` is read relative to the camera rather than to the world.
    ///
    /// On by default, and this is what PyMOL does - its lights are specified in camera
    /// space. A light fixed in the world means half of every orbit is spent looking at the
    /// unlit side of the molecule, which is a lighting rig for a scene rather than for an
    /// object you turn over in your hands. With this on, the key light stays over the
    /// viewer's shoulder however the structure is turned.
    ///
    /// Turn it off for a light that belongs to the scene - lighting a figure from a fixed
    /// direction so two renders from different angles are lit alike.
    #[serde(default = "yes")]
    pub follow_view: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn default_distance() -> f32 {
    4.0
}
fn default_softness() -> f32 {
    8.0
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Material {
    /// 0 is a mirror, 1 is chalk. Around 0.3 reads as the soft plastic that molecular
    /// figures conventionally use.
    #[serde(default = "default_roughness")]
    pub roughness: f32,
    #[serde(default = "default_specular")]
    pub specular: f32,
    #[serde(default)]
    pub metallic: f32,
}

fn default_roughness() -> f32 {
    0.28
}
fn default_specular() -> f32 {
    0.5
}

impl Default for Material {
    fn default() -> Self {
        Self {
            roughness: default_roughness(),
            specular: default_specular(),
            metallic: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneStyle {
    /// Linear RGB. White by default: a figure ends up on a white page far more often than
    /// on a dark slide, and a light background is also what makes ambient occlusion read.
    #[serde(default = "default_background")]
    pub background: [f32; 3],
    #[serde(default = "default_lights")]
    pub lights: Vec<Light>,
    #[serde(default)]
    pub material: Material,
    /// Uniform light from every direction, standing in for a sky. Higher on a light
    /// background, or the shadows turn muddy.
    #[serde(default = "default_ambient")]
    pub ambient: f32,
    #[serde(default)]
    pub categorical: Categorical,
    /// Use viridis for continuous values instead of the AlphaFold confidence bands.
    #[serde(default)]
    pub continuous_colors: bool,
    /// Which ramp the continuous schemes use, for the scene-wide default.
    #[serde(default)]
    pub ramp: super::color::Ramp,
    /// Screen-space ambient occlusion in the viewport. A Blender render path-traces it
    /// properly and ignores this.
    #[serde(default = "yes")]
    pub ambient_occlusion: bool,
    /// A dark edge around silhouettes, which is what makes a crowded structure legible.
    #[serde(default)]
    pub outline: bool,
}

/// A light neutral, not paper white. The window around the viewport is dark, and a pure
/// white rectangle in the middle of it is the brightest thing on the desk by a distance;
/// this is light enough that ambient occlusion still reads and a figure still looks like
/// one, and no longer the first thing the eye is pulled to.
fn default_background() -> [f32; 3] {
    // sRGB, as the background always is. A light grey well short of white: the viewport sits in a dark window, and a white
    // one is a lamp in the middle of it.
    [0.835, 0.847, 0.867]
}

fn default_ambient() -> f32 {
    0.55
}

/// A studio three-point rig, aimed by direction only.
///
/// Key from the upper right and towards the viewer, fill from the opposite side at a third
/// of the strength, and a rim from behind - the arrangement that separates a subject from
/// its background without looking lit from nowhere.
fn default_lights() -> Vec<Light> {
    vec![
        Light {
            id: "key".into(),
            name: "Light".into(),
            kind: LightKind::Sun,
            color: [1.0, 0.98, 0.95],
            energy: 3.0,
            direction: [0.4511, 0.5514, 0.7018],
            distance: default_distance(),
            softness: 12.0,
            follow_view: true,
            enabled: true,
        },
        Light {
            id: "fill".into(),
            name: "Fill light".into(),
            kind: LightKind::Sun,
            color: [0.92, 0.95, 1.0],
            energy: 1.0,
            direction: [-0.834, -0.3208, 0.4491],
            distance: default_distance(),
            softness: 20.0,
            follow_view: true,
            enabled: true,
        },
        Light {
            id: "rim".into(),
            name: "Rim light".into(),
            kind: LightKind::Sun,
            color: [1.0, 0.96, 0.9],
            energy: 1.6,
            direction: [-0.1081, 0.3785, -0.9193],
            distance: default_distance(),
            softness: 8.0,
            follow_view: true,
            enabled: true,
        },
    ]
}

impl Default for SceneStyle {
    fn default() -> Self {
        Self {
            background: default_background(),
            lights: default_lights(),
            material: Material::default(),
            ambient: default_ambient(),
            categorical: Categorical::default(),
            continuous_colors: false,
            ramp: super::color::Ramp::default(),
            ambient_occlusion: true,
            outline: false,
        }
    }
}

impl SceneStyle {
    pub fn color_context(&self) -> super::color::ColorContext {
        super::color::ColorContext {
            background: self.background,
            categorical: self.categorical,
            continuous: self.continuous_colors,
            ramp: self.ramp,
        }
    }

    pub fn enabled_lights(&self) -> impl Iterator<Item = &Light> {
        self.lights.iter().filter(|light| light.enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_rig_is_a_usable_three_point_setup() {
        let style = SceneStyle::default();
        assert_eq!(style.lights.len(), 3);
        assert_eq!(style.enabled_lights().count(), 3);
        // The key must be the brightest, or it is not a key light.
        let key = &style.lights[0];
        assert!(style.lights[1..]
            .iter()
            .all(|other| other.energy < key.energy));
        for light in &style.lights {
            let length = light
                .direction
                .iter()
                .map(|value| value * value)
                .sum::<f32>()
                .sqrt();
            assert!(
                (length - 1.0).abs() < 1e-3,
                "{} has direction length {length}, which skews where a point light is placed",
                light.name
            );
        }
    }

    #[test]
    fn the_background_reaches_the_colour_schemes() {
        // The link that keeps CPK readable: change the background and the colour context
        // changes with it.
        let mut style = SceneStyle::default();
        assert!(super::super::palette::is_light(
            style.color_context().background
        ));
        style.background = [0.02, 0.02, 0.03];
        assert!(!super::super::palette::is_light(
            style.color_context().background
        ));
    }

    #[test]
    fn a_partial_style_from_the_ui_fills_in_the_rest() {
        // The frontend sends what the user changed; everything else has to default to
        // something that renders.
        let style: SceneStyle = serde_json::from_str(r#"{"background":[0.0,0.0,0.0]}"#).unwrap();
        assert_eq!(style.background, [0.0, 0.0, 0.0]);
        assert_eq!(style.lights.len(), 3);
        assert!(style.ambient > 0.0);
        assert!(style.material.roughness > 0.0);
    }

    #[test]
    fn disabling_a_light_removes_it_from_the_rig() {
        let mut style = SceneStyle::default();
        style.lights[1].enabled = false;
        assert_eq!(style.enabled_lights().count(), 2);
    }
}
