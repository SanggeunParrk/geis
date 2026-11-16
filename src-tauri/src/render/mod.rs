//! Offline rendering through Blender.
//!
//! Blender runs as a separate process, never as a library. Two reasons, and the second is
//! the one that matters:
//!
//!   - `import bpy` makes the importing program a GPL derivative. This is exactly why
//!     Molecular Nodes relicensed from MIT to GPLv3. A subprocess is arm's length.
//!   - Cycles is for offline work anyway. The interactive viewport is our own renderer;
//!     Blender is for the path-traced version - real ambient occlusion, soft shadows,
//!     depth of field - that a world card or a figure needs and 60fps cannot give.
//!
//! Geometry is not rebuilt here. It comes from `crate::geometry`, the same code the
//! viewport draws, tessellated for a renderer that has no analytic spheres.

use crate::geometry::{self, Quality};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const RENDER_SCRIPT: &str = include_str!("../../assets/render_scene.py");

/// Where Blender is and what it is.
#[derive(Debug, Clone, Serialize)]
pub struct BlenderInfo {
    pub path: PathBuf,
    pub version: String,
}

/// Finds a Blender to use.
///
/// `GEIS_BLENDER` first, so a specific build can be pinned; then PATH; then the places the
/// official installers put it. Looking inside the app bundle matters on macOS, where a
/// drag-installed Blender is never on PATH.
pub fn locate() -> Option<PathBuf> {
    if let Some(path) = crate::legacy::env_path("BLENDER") {
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(found) = which("blender") {
        return Some(found);
    }
    let candidates = [
        "/Applications/Blender.app/Contents/MacOS/Blender",
        "/usr/bin/blender",
        "/usr/local/bin/blender",
        "/opt/homebrew/bin/blender",
        "/snap/bin/blender",
        r"C:\Program Files\Blender Foundation\Blender\blender.exe",
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("Blender was not found. Install it, or set GEIS_BLENDER to its executable.")]
    NotFound,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Failed(String),
    #[error("Blender ran but wrote no image to {0}: {1}")]
    NoOutput(PathBuf, String),
}

/// Everything the render script needs that is not geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderSettings {
    /// Camera position, target and up vector, so a render matches the viewport exactly
    /// rather than approximately.
    pub eye: [f32; 3],
    pub center: [f32; 3],
    pub up: [f32; 3],
    #[serde(default = "default_fov")]
    pub fov_degrees: f32,
    /// The viewport's width/height at the moment the render was asked for.
    ///
    /// Needed because `fov_degrees` is vertical: without knowing the shape of the view it
    /// was framed in, a render at a different shape either crops the sides or the top.
    #[serde(default)]
    pub view_aspect: f32,
    /// Bounding radius, used to scale the lighting rig and the clipping planes.
    pub radius: f32,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_samples")]
    pub samples: u32,
    /// `cycles` or `eevee`. Cycles is slower and is the reason to render at all.
    #[serde(default = "default_engine")]
    pub engine: String,
    #[serde(default)]
    pub transparent: bool,
    /// Draft tessellation for thumbnails, fine for figures.
    #[serde(default)]
    pub draft: bool,
}

fn default_fov() -> f32 {
    40.0
}
fn default_width() -> u32 {
    1600
}
fn default_height() -> u32 {
    1200
}
fn default_samples() -> u32 {
    64
}
fn default_engine() -> String {
    "cycles".into()
}

/// Result of a completed render.
#[derive(Debug, Clone, Serialize)]
pub struct RenderOutcome {
    pub image: PathBuf,
    pub triangles: u32,
    pub seconds: f32,
    pub engine: String,
    pub log: String,
}

/// Renders a scene and returns where the image landed.
#[allow(clippy::too_many_arguments)]
pub async fn render(
    scene: &geometry::Scene,
    style: &geometry::SceneStyle,
    settings: &RenderSettings,
    workdir: &Path,
    output: &Path,
) -> Result<RenderOutcome, RenderError> {
    render_reporting(scene, style, settings, workdir, output, |_| {}).await
}

/// Rung to stop every render under way: the Cancel on the status bar. Each render waits
/// on it alongside Blender, and the one that is woken drops Blender, which kills it.
pub static CANCEL: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// How far a render has got, from Blender's own status line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderProgress {
    /// 0 to 1 once sampling has started; None while Blender is still preparing.
    pub fraction: Option<f32>,
    /// What Blender says it is doing: "Sample 12/64", "Loading render kernels".
    pub stage: String,
}

/// Reads one `[progress]` line of the render script's output.
pub fn read_progress(line: &str) -> Option<RenderProgress> {
    let stage = line.strip_prefix("[progress]")?.trim();
    let fraction = stage.split('|').find_map(|part| {
        let sample = part.trim().strip_prefix("Sample ")?;
        let (done, of) = sample.split_whitespace().next()?.split_once('/')?;
        let (done, of) = (done.parse::<f32>().ok()?, of.parse::<f32>().ok()?);
        (of > 0.0).then(|| (done / of).clamp(0.0, 1.0))
    });
    // The useful words: the sample count when there is one, otherwise the last part.
    let words = stage
        .split('|')
        .map(str::trim)
        .find(|part| part.starts_with("Sample "))
        .unwrap_or_else(|| stage.rsplit('|').next().unwrap_or(stage).trim());
    let words = words.split(" (").next().unwrap_or(words).to_string();
    Some(RenderProgress {
        fraction,
        stage: words,
    })
}

/// `render`, telling `report` how far Blender has got as it goes.
pub async fn render_reporting(
    scene: &geometry::Scene,
    style: &geometry::SceneStyle,
    settings: &RenderSettings,
    workdir: &Path,
    output: &Path,
    report: impl Fn(RenderProgress) + Send + Sync,
) -> Result<RenderOutcome, RenderError> {
    let blender = locate().ok_or(RenderError::NotFound)?;
    // A directory of this render's own, under the one it was given. Two renders can run
    // at once - there may be several Render nodes - and sharing the payload's path means
    // the second overwrites the scene the first Blender has not finished reading.
    let workdir = &workdir.join(crate::util::new_id());
    std::fs::create_dir_all(workdir)?;
    struct Cleanup<'a>(&'a Path);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            // The tessellated scene is tens to hundreds of megabytes and of no use after.
            let _ = std::fs::remove_dir_all(self.0);
        }
    }
    let _cleanup = Cleanup(workdir);
    let quality = if settings.draft {
        Quality::Draft
    } else {
        Quality::Fine
    };
    let mesh = geometry::tessellate(scene, quality);
    if mesh.is_empty() {
        return Err(RenderError::Failed(
            "this representation produced no geometry to render".into(),
        ));
    }

    // The script is written beside the payload rather than referenced in the app bundle,
    // so a packaged build and a dev build behave identically.
    let mesh_path = workdir.join("scene.mmsh");
    let settings_path = workdir.join("settings.json");
    let script_path = workdir.join("render_scene.py");
    std::fs::write(&mesh_path, geometry::tessellate::encode_mesh_file(&mesh))?;
    std::fs::write(
        &settings_path,
        serde_json::to_vec_pretty(&script_settings(settings, style)).unwrap_or_default(),
    )?;
    std::fs::write(&script_path, RENDER_SCRIPT)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let started = std::time::Instant::now();
    // Rendered beside the payload and moved into place only once it exists. Writing
    // straight to `output` cannot tell a fresh image from the one a previous render left
    // at the same path, so a script that failed would hand back the old picture.
    let rendered = workdir.join("render.png");
    let child = tokio::process::Command::new(&blender)
        .arg("-b")
        .arg("-noaudio")
        // Without this Blender exits 0 when the script raises - measured on 5.2 - and a
        // traceback is indistinguishable from a render.
        .arg("--python-exit-code")
        .arg("1")
        .arg("-P")
        .arg(&script_path)
        .arg("--")
        .arg(&mesh_path)
        .arg(&settings_path)
        .arg(&rendered)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // The timeout below drops this future; without this the process it was waiting
        // on carries on rendering, holding the GPU, with nobody to collect the result.
        .kill_on_drop(true)
        .spawn()?;

    // Read as it arrives, line by line, so that the script's progress lines can be passed
    // on while Blender works; everything is also kept for the log.
    let run = async move {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt};
        let mut child = child;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let errors = tokio::spawn(async move {
            let mut text = Vec::new();
            if let Some(mut stderr) = stderr {
                let _ = stderr.read_to_end(&mut text).await;
            }
            text
        });
        let mut out = String::new();
        if let Some(stdout) = stdout {
            let mut lines = tokio::io::BufReader::new(stdout).split(b'\n');
            while let Ok(Some(line)) = lines.next_segment().await {
                let line = String::from_utf8_lossy(&line).into_owned();
                if let Some(progress) = read_progress(&line) {
                    report(progress);
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
        let status = child.wait().await?;
        let errors = errors.await.unwrap_or_default();
        Ok::<_, std::io::Error>((status, out, errors))
    };

    // Generous but bounded: a fine render of a large assembly is minutes, an infinite hang
    // is a bug. And stopped at once when it is cancelled: dropping the run drops Blender,
    // which was started to be killed on drop.
    let (status, stdout, stderr) = tokio::select! {
        finished = tokio::time::timeout(Duration::from_secs(1800), run) => match finished {
            Ok(result) => result?,
            Err(_) => {
                return Err(RenderError::Failed(
                    "Blender did not finish in 30 minutes".into(),
                ))
            }
        },
        () = CANCEL.notified() => {
            return Err(RenderError::Failed("the render was cancelled".into()))
        }
    };
    struct Finished {
        status: std::process::ExitStatus,
    }
    let finished = Finished { status };

    let log = format!("{}{}", stdout, String::from_utf8_lossy(&stderr));
    if !finished.status.success() {
        // Blender's failures are almost always in the last few lines; the whole log is
        // kept but the message quotes the useful end of it.
        let tail: Vec<&str> = log.lines().rev().take(6).collect();
        return Err(RenderError::Failed(format!(
            "Blender exited with {}: {}",
            finished.status,
            tail.into_iter().rev().collect::<Vec<_>>().join(" / ")
        )));
    }
    if !rendered.is_file() {
        let tail: Vec<&str> = log.lines().rev().take(6).collect();
        return Err(RenderError::NoOutput(
            output.to_path_buf(),
            tail.into_iter().rev().collect::<Vec<_>>().join(" / "),
        ));
    }
    // A rename when it can be one; a copy when the destination is on another volume.
    if std::fs::rename(&rendered, output).is_err() {
        std::fs::copy(&rendered, output)?;
    }

    Ok(RenderOutcome {
        image: output.to_path_buf(),
        triangles: mesh.triangle_count() as u32,
        seconds: started.elapsed().as_secs_f32(),
        engine: settings.engine.clone(),
        log,
    })
}

/// A light's direction in world space, which is what Blender places lamps with.
///
/// A light that follows the view has its direction written in the camera's own frame - x
/// to the right, y up, z towards the viewer - which is the frame the viewport shades in.
/// Turning it into world space is the same rotation the viewport skips, run backwards: the
/// camera basis from `eye`, `center` and `up`.
fn world_direction(light: &geometry::Light, settings: &RenderSettings) -> [f32; 3] {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let normalize = |v: [f32; 3]| {
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if length < 1e-6 {
            [0.0, 0.0, 1.0]
        } else {
            [v[0] / length, v[1] / length, v[2] / length]
        }
    };

    // Normalised here as well: a point or area lamp is placed at `direction * distance`,
    // so a short vector - or a zero one, typed into the panel - would put the lamp inside
    // the molecule. A light that follows the view is normalised on the way out below.
    if !light.follow_view {
        return normalize(light.direction);
    }

    let forward = normalize(sub(settings.center, settings.eye));
    // An up vector along the line of sight - a camera looking straight down with y up, or
    // a scripted view with no up at all - has no right to cross with. Any perpendicular
    // will do then; falling through to `normalize`'s fixed answer could hand back the
    // forward axis itself, and every camera-relative light then lost its x and y.
    let mut right = cross(forward, settings.up);
    if (right[0] * right[0] + right[1] * right[1] + right[2] * right[2]).sqrt() < 1e-6 {
        let helper = if forward[1].abs() < 0.9 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        right = cross(forward, helper);
    }
    let right = normalize(right);
    // Rebuilt rather than taken from the settings, which need not be perpendicular.
    let up = cross(right, forward);
    let [x, y, z] = light.direction;

    normalize([
        right[0] * x + up[0] * y - forward[0] * z,
        right[1] * x + up[1] * y - forward[1] * z,
        right[2] * x + up[2] * y - forward[2] * z,
    ])
}

/// Merges the camera settings and the scene style into what the render script reads.
///
/// One flat object rather than two: the script's job is to set up a scene, not to know
/// which half of it came from the camera panel and which from the light panel.
fn script_settings(settings: &RenderSettings, style: &geometry::SceneStyle) -> serde_json::Value {
    let lights: Vec<serde_json::Value> = style
        .enabled_lights()
        .map(|light| {
            serde_json::json!({
                "kind": light.kind,
                "color": light.color,
                "energy": light.energy,
                // Camera-relative lights are resolved here rather than in the script: the
                // camera basis is already known at this point, and a render whose lamps
                // sit somewhere else than the viewport's is not the picture on screen.
                "direction": world_direction(light, settings),
                "distance": light.distance,
                "softness": light.softness,
                "name": light.name,
            })
        })
        .collect();

    serde_json::json!({
        "eye": settings.eye,
        "center": settings.center,
        "up": settings.up,
        "camera_fov_y": settings.camera_fov_y(),
        "radius": settings.radius,
        "width": settings.width,
        "height": settings.height,
        "samples": settings.samples,
        "engine": settings.engine,
        "transparent": settings.transparent,
        "background": style.background,
        "ambient": style.ambient,
        "roughness": style.material.roughness,
        "specular": style.material.specular,
        "metallic": style.material.metallic,
        "lights": lights,
    })
}

impl RenderSettings {
    /**
     * The vertical field of view the render has to use to show everything the viewport
     * showed.
     *
     * The viewport's field of view is vertical, and a render is almost never the same
     * shape as the window it was framed in. Blender's `camera.angle` applies to the larger
     * render dimension by default, so a 40 degree vertical view became a 40 degree
     * *horizontal* one at 1600x1200 - a vertical field of 30.6 degrees, which cut the top
     * and bottom off every render.
     *
     * The rule here is "never crop": take whichever field of view contains both axes of
     * what was on screen. A render wider than the viewport then shows more at the sides, a
     * narrower one shows more above and below, and neither loses anything that was
     * visible. Framing generously is recoverable by cropping; framing tightly is not.
     */
    pub fn camera_fov_y(&self) -> f32 {
        // Kept to what a perspective camera can be: at 0 degrees nothing is seen, and at
        // 180 the tangent is infinite and Blender refuses the angle outright.
        const LOWEST: f32 = 1.0;
        const HIGHEST: f32 = 170.0;
        let fov_degrees = if self.fov_degrees.is_finite() {
            self.fov_degrees.clamp(LOWEST, HIGHEST)
        } else {
            default_fov()
        };
        let fov_y = fov_degrees.to_radians();
        let render_aspect = self.width.max(1) as f32 / self.height.max(1) as f32;
        let view_aspect = if self.view_aspect > 0.0 {
            self.view_aspect
        } else {
            render_aspect
        };
        let tan_y = (fov_y / 2.0).tan();
        // The horizontal half-extent the viewport showed, expressed as a vertical tangent
        // at the render's shape.
        let needed = tan_y * view_aspect / render_aspect;
        (2.0 * tan_y.max(needed).atan())
            .to_degrees()
            .clamp(LOWEST, HIGHEST)
    }
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    fn settings(view_aspect: f32, width: u32, height: u32) -> RenderSettings {
        RenderSettings {
            eye: [0.0, 0.0, 10.0],
            center: [0.0; 3],
            up: [0.0, 1.0, 0.0],
            fov_degrees: 40.0,
            view_aspect,
            radius: 5.0,
            width,
            height,
            samples: 16,
            engine: "cycles".into(),
            transparent: false,
            draft: true,
        }
    }

    /// The half-extents a field of view covers at unit distance, vertical and horizontal.
    fn extents(fov_y_degrees: f32, aspect: f32) -> (f32, f32) {
        let tan_y = (fov_y_degrees.to_radians() / 2.0).tan();
        (tan_y, tan_y * aspect)
    }

    #[test]
    fn nothing_visible_in_the_viewport_is_ever_cropped() {
        // The invariant, stated as an invariant: for any window shape and any render size,
        // the render covers at least what the viewport covered on both axes. Written this
        // way because the first version of this test asserted the field of view *grew* for
        // a case where it correctly did not, and passed for the wrong reason.
        let cases = [
            // The regression: a window taller than wide, a 4:3 render. Before the fix the
            // render's vertical field was 30.6 degrees against the viewport's 40.
            (0.936_f32, 1600_u32, 1200_u32),
            (4.0 / 3.0, 1600, 1200),
            (1.0, 1920, 1080),
            (16.0 / 9.0, 1200, 1600),
            (0.5, 4096, 512),
            (2.5, 512, 4096),
        ];
        for (view_aspect, width, height) in cases {
            let settings = settings(view_aspect, width, height);
            let (view_y, view_x) = extents(settings.fov_degrees, view_aspect);
            let (render_y, render_x) =
                extents(settings.camera_fov_y(), width as f32 / height as f32);
            assert!(
                render_y >= view_y - 1e-4,
                "{view_aspect} at {width}x{height}: vertical {render_y} < {view_y}"
            );
            assert!(
                render_x >= view_x - 1e-4,
                "{view_aspect} at {width}x{height}: horizontal {render_x} < {view_x}"
            );
        }
    }

    #[test]
    fn it_frames_no_wider_than_it_has_to() {
        // Never cropping is trivially satisfied by a fisheye, so the other half matters:
        // one of the two axes must fit exactly, or the subject is needlessly small.
        for (view_aspect, width, height) in [(0.936_f32, 1600_u32, 1200_u32), (1.78, 1200, 1600)] {
            let settings = settings(view_aspect, width, height);
            let (view_y, view_x) = extents(settings.fov_degrees, view_aspect);
            let (render_y, render_x) =
                extents(settings.camera_fov_y(), width as f32 / height as f32);
            let tight = (render_y - view_y).abs() < 1e-4 || (render_x - view_x).abs() < 1e-4;
            assert!(
                tight,
                "{view_aspect} at {width}x{height}: neither axis is tight"
            );
        }
    }

    #[test]
    fn the_same_shape_keeps_the_same_field_of_view() {
        let fov = settings(4.0 / 3.0, 1600, 1200).camera_fov_y();
        assert!((fov - 40.0).abs() < 1e-3, "{fov}");
    }

    #[test]
    fn a_portrait_render_of_a_landscape_view_opens_up_rather_than_cropping() {
        let fov = settings(16.0 / 9.0, 1200, 1600).camera_fov_y();
        assert!(fov > 40.0, "it must open up, not crop: {fov}");
    }

    #[test]
    fn a_missing_aspect_falls_back_to_the_render_shape() {
        // Older callers, and the scripted path, may not report one.
        let fov = settings(0.0, 800, 600).camera_fov_y();
        assert!((fov - 40.0).abs() < 1e-3, "{fov}");
    }

    #[test]
    fn a_field_of_view_out_of_range_is_kept_to_one_a_camera_can_have() {
        // Zero sees nothing, 180 is an infinite tangent, and Blender rejects both.
        for fov_degrees in [0.0_f32, -30.0, 180.0, 400.0] {
            let fov = RenderSettings {
                fov_degrees,
                ..settings(4.0 / 3.0, 1600, 1200)
            }
            .camera_fov_y();
            assert!(
                fov.is_finite() && fov > 0.0 && fov < 180.0,
                "{fov_degrees} gave {fov}"
            );
        }
        // A render with no width cannot be a fisheye either.
        let fov = settings(1.0, 0, 600).camera_fov_y();
        assert!(fov.is_finite() && fov > 0.0 && fov < 180.0, "{fov}");
    }

    #[test]
    fn a_zero_height_render_does_not_divide_by_zero() {
        let fov = settings(1.0, 800, 0).camera_fov_y();
        assert!(fov.is_finite() && fov > 0.0, "{fov}");
    }
}

/// Blender's version string, for the settings screen.
pub async fn info() -> Option<BlenderInfo> {
    let path = locate()?;
    let output = tokio::process::Command::new(&path)
        .arg("--version")
        .output()
        .await
        .ok()?;
    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    Some(BlenderInfo {
        path,
        version: if version.is_empty() {
            "unknown".into()
        } else {
            version
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_with_defaults() {
        let json = r#"{"eye":[0,0,50],"center":[0,0,0],"up":[0,1,0],"radius":20}"#;
        let settings: RenderSettings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.width, 1600);
        assert_eq!(settings.samples, 64);
        assert_eq!(settings.engine, "cycles");
        assert!(!settings.draft);
    }

    /// A camera looking down -z from +z, which is the viewport's own starting position.
    fn front_view() -> RenderSettings {
        serde_json::from_str(r#"{"eye":[0,0,50],"center":[0,0,0],"up":[0,1,0],"radius":20}"#)
            .unwrap()
    }

    fn light_pointing(direction: [f32; 3], follow_view: bool) -> geometry::Light {
        geometry::Light {
            id: "key".into(),
            name: "Light".into(),
            kind: geometry::LightKind::Sun,
            color: [1.0; 3],
            energy: 3.0,
            direction,
            distance: 4.0,
            softness: 10.0,
            follow_view,
            enabled: true,
        }
    }

    #[test]
    fn a_light_that_follows_the_view_is_placed_relative_to_the_camera() {
        // Straight over the viewer's shoulder: from the front view that is +z in world
        // space too, because the camera is on +z looking back at the origin.
        let front = world_direction(&light_pointing([0.0, 0.0, 1.0], true), &front_view());
        assert!((front[2] - 1.0).abs() < 1e-5, "{front:?}");

        // Turn the camera to look down the x axis and the same light follows it round.
        let side: RenderSettings =
            serde_json::from_str(r#"{"eye":[50,0,0],"center":[0,0,0],"up":[0,1,0],"radius":20}"#)
                .unwrap();
        let turned = world_direction(&light_pointing([0.0, 0.0, 1.0], true), &side);
        assert!((turned[0] - 1.0).abs() < 1e-5, "{turned:?}");

        // The camera's right and up are the light's x and y, so a key light over the
        // right shoulder stays over the right shoulder.
        let key = world_direction(&light_pointing([0.45, 0.55, 0.70], true), &side);
        assert!(key[0] > 0.6, "towards the eye: {key:?}");
        assert!(key[1] > 0.4, "and above it: {key:?}");
        assert!(key[2] < -0.3, "the camera's right is -z here: {key:?}");
    }

    #[test]
    fn a_light_fixed_to_the_scene_is_left_where_it_was_put() {
        // The other half of the switch: a light that belongs to the scene has to light two
        // renders from different angles the same way.
        let fixed = light_pointing([0.6, 0.0, 0.8], false);
        assert_eq!(world_direction(&fixed, &front_view()), [0.6, 0.0, 0.8]);
        let side: RenderSettings =
            serde_json::from_str(r#"{"eye":[50,0,0],"center":[0,0,0],"up":[0,1,0],"radius":20}"#)
                .unwrap();
        assert_eq!(world_direction(&fixed, &side), [0.6, 0.0, 0.8]);
    }

    fn unit(v: [f32; 3]) -> bool {
        v.iter().all(|value| value.is_finite())
            && ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() - 1.0).abs() < 1e-4
    }

    #[test]
    fn a_fixed_light_with_a_short_or_zero_direction_is_still_a_direction() {
        // A point lamp is placed at direction * distance * radius; a half-length vector
        // put it at half the distance the panel says, and a zero one inside the molecule.
        let short = world_direction(&light_pointing([0.0, 0.5, 0.0], false), &front_view());
        assert!(unit(short), "{short:?}");
        assert!((short[1] - 1.0).abs() < 1e-5, "{short:?}");
        let zero = world_direction(&light_pointing([0.0; 3], false), &front_view());
        assert!(unit(zero), "{zero:?}");
    }

    #[test]
    fn a_camera_looking_along_its_own_up_still_turns_lights_with_it() {
        // Looking straight down with y as up: the up vector is the line of sight, and the
        // camera basis has to be made from something else rather than collapsing.
        let down: RenderSettings =
            serde_json::from_str(r#"{"eye":[0,50,0],"center":[0,0,0],"up":[0,1,0],"radius":20}"#)
                .unwrap();
        let right = world_direction(&light_pointing([1.0, 0.0, 0.0], true), &down);
        let up = world_direction(&light_pointing([0.0, 1.0, 0.0], true), &down);
        let towards = world_direction(&light_pointing([0.0, 0.0, 1.0], true), &down);
        for direction in [right, up, towards] {
            assert!(unit(direction), "{direction:?}");
        }
        // Still a basis: three different directions, not one direction three times.
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        assert!(dot(right, up).abs() < 1e-4, "{right:?} {up:?}");
        assert!(dot(right, towards).abs() < 1e-4, "{right:?} {towards:?}");
        assert!(
            (towards[1] - 1.0).abs() < 1e-4,
            "towards the eye is +y: {towards:?}"
        );

        // The same along z, where the old fallback handed back the forward axis.
        let along_z: RenderSettings =
            serde_json::from_str(r#"{"eye":[0,0,50],"center":[0,0,0],"up":[0,0,1],"radius":20}"#)
                .unwrap();
        let right = world_direction(&light_pointing([1.0, 0.0, 0.0], true), &along_z);
        assert!(unit(right), "{right:?}");
        assert!(right[2].abs() < 1e-4, "right is across the view: {right:?}");

        // And an eye sitting on its target gives finite numbers, not NaN.
        let nowhere: RenderSettings =
            serde_json::from_str(r#"{"eye":[1,1,1],"center":[1,1,1],"up":[0,1,0],"radius":0}"#)
                .unwrap();
        assert!(unit(world_direction(
            &light_pointing([0.45, 0.55, 0.7], true),
            &nowhere
        )));
    }

    #[test]
    fn the_settings_written_for_blender_hold_no_nulls() {
        // serde_json writes a NaN or an infinity as null, and the script's float() of null
        // is a traceback rather than a render.
        let odd: RenderSettings = serde_json::from_str(
            r#"{"eye":[0,0,0],"center":[0,0,0],"up":[0,0,0],"radius":0,"fov_degrees":0,"width":0,"height":0}"#,
        )
        .unwrap();
        let mut style = geometry::SceneStyle::default();
        style.lights[0].direction = [0.0; 3];
        style.lights[1].follow_view = false;
        style.lights[1].direction = [0.0; 3];
        let json = script_settings(&odd, &style);
        let text = serde_json::to_string(&json).unwrap();
        assert!(!text.contains("null"), "{text}");
        let fov = json["camera_fov_y"].as_f64().unwrap();
        assert!(fov > 0.0 && fov < 180.0, "{fov}");
    }

    #[test]
    fn the_script_receives_the_style_the_viewport_is_using() {
        // The preview only predicts the render if both are driven by the same numbers.
        let settings: RenderSettings =
            serde_json::from_str(r#"{"eye":[0,0,50],"center":[0,0,0],"up":[0,1,0],"radius":20}"#)
                .unwrap();
        let mut style = geometry::SceneStyle {
            background: [1.0, 1.0, 1.0],
            ..Default::default()
        };
        style.lights[2].enabled = false;

        let json = script_settings(&settings, &style);
        assert_eq!(json["background"], serde_json::json!([1.0, 1.0, 1.0]));
        assert_eq!(
            json["lights"].as_array().unwrap().len(),
            2,
            "disabled lights are dropped"
        );
        assert_eq!(json["lights"][0]["kind"], "sun");
        assert!(json["roughness"].as_f64().unwrap() > 0.0);
        assert_eq!(json["width"], 1600);
    }

    #[test]
    fn the_script_is_embedded_and_looks_like_the_real_one() {
        // Catches the build having stopped shipping the script, which would fail only at
        // render time with a confusing Blender error.
        assert!(RENDER_SCRIPT.contains("def main()"));
        assert!(RENDER_SCRIPT.contains("MMSH"));
    }

    #[test]
    fn blender_s_status_line_is_read_as_a_fraction_and_a_stage() {
        let sampling = read_progress(
            "[progress] Remaining: 00:04.93 | Mem: 2921M | Sample 8/32 (Using optimized kernels)",
        )
        .unwrap();
        assert_eq!(sampling.fraction, Some(0.25));
        assert_eq!(sampling.stage, "Sample 8/32");
        let preparing = read_progress(
            "[progress] Loading render kernels (may take a few minutes the first time)",
        )
        .unwrap();
        assert_eq!(preparing.fraction, None);
        assert_eq!(preparing.stage, "Loading render kernels");
        assert!(read_progress("[render] wrote it").is_none());
    }
}
