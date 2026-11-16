//! Where the window was left.
//!
//! An app that opens at its factory size in the middle of the screen every time makes its
//! user arrange it every time. The frame is written as it changes and put back at launch -
//! but only where it still fits a screen that is actually attached, because a window
//! restored onto a monitor that has since been unplugged is a window nobody can reach.

use serde::{Deserialize, Serialize};

/// The setting the frame is kept under.
pub const KEY: &str = "window_frame";

/// The smallest window the interface is laid out for; see `tauri.conf.json`.
const MIN_WIDTH: u32 = 960;
const MIN_HEIGHT: u32 = 640;

/// Position and size in physical pixels, as the window system reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A screen's area in the same coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// Reads a saved frame. Anything unreadable is no frame at all, and the window keeps the
/// size it was configured with.
pub fn parse(saved: &str) -> Option<Frame> {
    let frame: Frame = serde_json::from_str(saved).ok()?;
    (frame.width > 0 && frame.height > 0).then_some(frame)
}

/// The frame to restore, or `None` if it should not be.
///
/// It is restored when enough of its title bar is on some screen to be grabbed: the top
/// edge on the screen, and at least a hand's width of it. The size is cut down to that
/// screen, and never below what the interface needs.
pub fn fit(frame: Frame, screens: &[Screen]) -> Option<Frame> {
    const GRAB: i64 = 120;
    let left = i64::from(frame.x);
    let right = left + i64::from(frame.width);
    let top = i64::from(frame.y);

    let screen = screens.iter().find(|screen| {
        let screen_left = i64::from(screen.x);
        let screen_right = screen_left + i64::from(screen.width);
        let screen_top = i64::from(screen.y);
        let screen_bottom = screen_top + i64::from(screen.height);
        let overlap = right.min(screen_right) - left.max(screen_left);
        overlap >= GRAB && top >= screen_top && top < screen_bottom - GRAB
    })?;

    let scale = if screen.scale.is_finite() && screen.scale > 0.0 {
        screen.scale
    } else {
        1.0
    };
    let least = |points: u32| (f64::from(points) * scale).round() as u32;
    Some(Frame {
        x: frame.x,
        y: frame.y,
        width: frame
            .width
            .min(screen.width)
            .max(least(MIN_WIDTH).min(screen.width)),
        height: frame
            .height
            .min(screen.height)
            .max(least(MIN_HEIGHT).min(screen.height)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: Screen = Screen {
        x: 0,
        y: 0,
        width: 3024,
        height: 1964,
        scale: 2.0,
    };

    #[test]
    fn a_frame_on_the_screen_comes_back_as_it_was() {
        let frame = Frame {
            x: 200,
            y: 100,
            width: 2400,
            height: 1500,
        };
        assert_eq!(fit(frame, &[LAPTOP]), Some(frame));
    }

    #[test]
    fn a_frame_on_an_unplugged_monitor_is_not_restored() {
        let frame = Frame {
            x: 4000,
            y: 100,
            width: 2400,
            height: 1500,
        };
        assert_eq!(fit(frame, &[LAPTOP]), None);
        // Above the screen, where its title bar could not be reached.
        let frame = Frame {
            x: 200,
            y: -900,
            width: 2400,
            height: 1500,
        };
        assert_eq!(fit(frame, &[LAPTOP]), None);
        assert_eq!(fit(frame, &[]), None);
    }

    #[test]
    fn a_frame_on_a_second_monitor_is_restored_there() {
        let external = Screen {
            x: -2560,
            y: 0,
            width: 2560,
            height: 1440,
            scale: 1.0,
        };
        let frame = Frame {
            x: -2000,
            y: 50,
            width: 1400,
            height: 900,
        };
        assert_eq!(fit(frame, &[LAPTOP, external]), Some(frame));
    }

    #[test]
    fn a_frame_larger_than_the_screen_is_cut_down_to_it() {
        let frame = Frame {
            x: 0,
            y: 0,
            width: 6000,
            height: 4000,
        };
        let fitted = fit(frame, &[LAPTOP]).unwrap();
        assert_eq!((fitted.width, fitted.height), (3024, 1964));
    }

    #[test]
    fn a_frame_is_never_smaller_than_the_interface_needs() {
        let frame = Frame {
            x: 100,
            y: 100,
            width: 300,
            height: 200,
        };
        let fitted = fit(frame, &[LAPTOP]).unwrap();
        assert_eq!((fitted.width, fitted.height), (1920, 1280));
    }

    #[test]
    fn rubbish_in_the_setting_is_no_frame() {
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"x":0,"y":0,"width":0,"height":900}"#), None);
        assert_eq!(
            parse(r#"{"x":1,"y":2,"width":3,"height":4}"#),
            Some(Frame {
                x: 1,
                y: 2,
                width: 3,
                height: 4
            })
        );
    }
}
