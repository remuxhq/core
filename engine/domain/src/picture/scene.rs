//! Where things sit in the picture.
//!
//! Arithmetic over rectangles, so `cargo test` covers all of it and the
//! compositor is left with nothing to decide. That split matters more here
//! than anywhere else: a camera in the wrong place is visible to every viewer,
//! and a self-view that stretches with its window is the bug this guards.

/// A rectangle in the output picture, in pixels, origin at the bottom left the
/// way Core Image and Core Graphics count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn area(&self) -> f64 {
        self.width * self.height
    }
}

/// Which corner the self-view lives in.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    TopLeft,
    TopRight,
    #[default]
    BottomRight,
    BottomLeft,
}

/// The two shapes the camera can take on the broadcast. The circular crop
/// is the current default; rectangle restores the camera's native aspect.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CameraShape {
    #[default]
    Circle,
    Rectangle,
}

/// The native scene's dimensions; the CLI positions its camera in these pixels.
pub const CAMERA_OUTPUT: (u32, u32) = (1920, 1080);

/// Requested upper-left corner of the composited camera in output pixels.
/// The output is 1920×1080; Core Image uses a bottom-left origin, so the
/// conversion belongs in this domain rather than the compositor.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct CameraPosition {
    pub x: u32,
    pub y: u32,
}

/// Place a camera viewport at the requested top-left pixel, keeping the whole
/// viewport visible even when the requested point lies close to an edge.
#[must_use]
pub fn positioned_camera(
    slot: Rect,
    output: (f64, f64),
    at: Option<CameraPosition>,
    shape: CameraShape,
) -> Rect {
    let (width, height) = match shape {
        CameraShape::Circle => {
            let side = slot.width.min(slot.height);
            (side, side)
        }
        CameraShape::Rectangle => (slot.width, slot.height),
    };
    match at {
        Some(at) => Rect {
            x: (at.x as f64).clamp(0.0, (output.0 - width).max(0.0)),
            y: (output.1 - at.y as f64 - height).clamp(0.0, (output.1 - height).max(0.0)),
            width,
            height,
        },
        None => Rect {
            x: slot.x + (slot.width - width) / 2.0,
            y: slot.y + (slot.height - height) / 2.0,
            width,
            height,
        },
    }
}

/// How wide the camera is, as a share of the picture. A quarter is what the
/// panel has always used and what a face needs to be readable at 1080p.
pub const CAMERA_SHARE: f64 = 0.25;

/// How far off the edges. Enough that the corner does not look like a mistake.
pub const CAMERA_MARGIN: f64 = 24.0;

/// Where the camera goes.
///
/// The rule that matters: **the camera's own aspect decides its height**, and
/// nothing else ever does. A 4:3 webcam and a 16:9 webcam are different shapes
/// and forcing either into the other's box is the stretched face this project
/// has shipped twice. If the aspect is unknown the slot is empty, because
/// drawing a guess is worse than drawing nothing.
pub fn camera_slot(
    output: (f64, f64),
    camera: (f64, f64),
    corner: Corner,
    share: f64,
    margin: f64,
) -> Option<Rect> {
    let (out_w, out_h) = output;
    let (cam_w, cam_h) = camera;
    if out_w <= 0.0 || out_h <= 0.0 || cam_w <= 0.0 || cam_h <= 0.0 {
        return None;
    }
    let width = (out_w * share.clamp(0.05, 1.0)).min(out_w - 2.0 * margin);
    let height = width * (cam_h / cam_w);
    if width <= 0.0 || height <= 0.0 || height > out_h - 2.0 * margin {
        // A camera so tall it would not fit is not placed rather than squashed.
        return None;
    }
    let (x, y) = match corner {
        Corner::TopLeft => (margin, out_h - margin - height),
        Corner::TopRight => (out_w - margin - width, out_h - margin - height),
        Corner::BottomLeft => (margin, margin),
        Corner::BottomRight => (out_w - margin - width, margin),
    };
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}

/// `from` fitted inside `into`, its shape kept, centred.
pub fn fit(from: (f64, f64), into: Rect) -> Rect {
    if from.0 <= 0.0 || from.1 <= 0.0 {
        return into;
    }
    let scale = (into.width / from.0).min(into.height / from.1);
    let (width, height) = (from.0 * scale, from.1 * scale);
    Rect {
        x: into.x + (into.width - width) / 2.0,
        y: into.y + (into.height - height) / 2.0,
        width,
        height,
    }
}

/// The transform that puts a source of `from` pixels into `into`, and mirrors
/// it if asked. Returned as the six numbers of an affine transform, in the
/// order Core Graphics takes them: `[a, b, c, d, tx, ty]`.
///
/// Mirroring is a negative horizontal scale plus a shift back into place, not
/// a separate step: doing it in one transform is one resample instead of two,
/// and the self-view is the one thing on screen a person stares at.
pub fn place(from: (f64, f64), into: Rect, mirror: bool) -> [f64; 6] {
    let (src_w, src_h) = from;
    let scale_x = into.width / src_w;
    let scale_y = into.height / src_h;
    if mirror {
        [-scale_x, 0.0, 0.0, scale_y, into.x + into.width, into.y]
    } else {
        [scale_x, 0.0, 0.0, scale_y, into.x, into.y]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD: (f64, f64) = (1920.0, 1080.0);

    #[test]
    fn a_camera_moves_from_the_top_left_and_stays_wholly_in_frame() {
        let slot = camera_slot(
            HD,
            (1280.0, 720.0),
            Corner::BottomRight,
            CAMERA_SHARE,
            CAMERA_MARGIN,
        )
        .unwrap();
        let at =
            |x, y| positioned_camera(slot, HD, Some(CameraPosition { x, y }), CameraShape::Circle);
        assert_eq!((at(300, 200).x, at(300, 200).y), (300.0, 610.0));
        assert_eq!((at(0, 0).x, at(0, 0).y), (0.0, 810.0));
        assert_eq!((at(1919, 1079).x, at(1919, 1079).y), (1650.0, 0.0));
        assert_eq!(
            positioned_camera(slot, HD, None, CameraShape::Circle),
            Rect {
                x: slot.x + (slot.width - 270.0) / 2.0,
                y: slot.y,
                width: 270.0,
                height: 270.0
            },
            "the original centered square stays in its corner"
        );
    }

    #[test]
    fn rectangle_preserves_the_camera_aspect_and_its_position() {
        let slot = camera_slot(
            HD,
            (1280.0, 720.0),
            Corner::BottomRight,
            CAMERA_SHARE,
            CAMERA_MARGIN,
        )
        .unwrap();
        assert_eq!(
            positioned_camera(slot, HD, None, CameraShape::Rectangle),
            slot
        );
        let at = positioned_camera(
            slot,
            HD,
            Some(CameraPosition { x: 1650, y: 810 }),
            CameraShape::Rectangle,
        );
        assert_eq!(
            at,
            Rect {
                x: 1440.0,
                y: 0.0,
                width: 480.0,
                height: 270.0
            }
        );
        let at = positioned_camera(
            slot,
            HD,
            Some(CameraPosition { x: 300, y: 200 }),
            CameraShape::Rectangle,
        );
        assert_eq!(
            at,
            Rect {
                x: 300.0,
                y: 610.0,
                width: 480.0,
                height: 270.0
            }
        );
    }

    #[test]
    fn a_sixteen_by_nine_camera_keeps_its_shape() {
        let slot =
            camera_slot(HD, (1920.0, 1080.0), Corner::BottomRight, 0.25, 24.0).expect("it fits");
        assert_eq!(slot.width, 480.0);
        assert_eq!(slot.height, 270.0, "16:9 in, 16:9 out");
    }

    // A 1280x960 webcam is 4:3. Forcing it into a 16:9 box is a stretched
    // face, and the reason the height is computed and never given.
    #[test]
    fn a_four_by_three_camera_also_keeps_its_shape() {
        let slot =
            camera_slot(HD, (1280.0, 960.0), Corner::BottomRight, 0.25, 24.0).expect("it fits");
        assert_eq!(slot.width, 480.0);
        assert_eq!(slot.height, 360.0, "4:3 in, 4:3 out, never 270");
    }

    #[test]
    fn each_corner_is_the_corner_it_says() {
        let cam = (1920.0, 1080.0);
        let at = |corner| camera_slot(HD, cam, corner, 0.25, 24.0).expect("it fits");

        assert_eq!(
            (at(Corner::BottomLeft).x, at(Corner::BottomLeft).y),
            (24.0, 24.0)
        );
        assert_eq!(at(Corner::BottomRight).x, 1920.0 - 24.0 - 480.0);
        assert_eq!(at(Corner::BottomRight).y, 24.0);
        assert_eq!(at(Corner::TopLeft).x, 24.0);
        assert_eq!(at(Corner::TopLeft).y, 1080.0 - 24.0 - 270.0);
        assert_eq!(at(Corner::TopRight).x, 1920.0 - 24.0 - 480.0);
    }

    #[test]
    fn every_corner_stays_inside_the_picture() {
        for corner in [
            Corner::TopLeft,
            Corner::TopRight,
            Corner::BottomLeft,
            Corner::BottomRight,
        ] {
            for camera in [(1920.0, 1080.0), (1280.0, 960.0), (1920.0, 1440.0)] {
                let slot = camera_slot(HD, camera, corner, 0.25, 24.0).expect("it fits");
                assert!(slot.x >= 0.0, "{corner:?} {camera:?} ran off the left");
                assert!(slot.y >= 0.0, "{corner:?} {camera:?} ran off the bottom");
                assert!(
                    slot.x + slot.width <= 1920.0,
                    "{corner:?} ran off the right"
                );
                assert!(slot.y + slot.height <= 1080.0, "{corner:?} ran off the top");
            }
        }
    }

    // Drawing a guess is worse than drawing nothing: a camera that has not
    // reported a size yet would otherwise be placed at whatever the last one
    // was, which is a face in the wrong shape for a second or two.
    #[test]
    fn a_camera_with_no_size_yet_is_not_placed() {
        assert_eq!(
            camera_slot(HD, (0.0, 0.0), Corner::BottomRight, 0.25, 24.0),
            None
        );
        assert_eq!(
            camera_slot(
                (0.0, 0.0),
                (1920.0, 1080.0),
                Corner::BottomRight,
                0.25,
                24.0
            ),
            None
        );
    }

    #[test]
    fn a_camera_too_tall_for_the_picture_is_not_squashed_into_it() {
        // a very tall source, asked to be nearly the full width
        assert_eq!(
            camera_slot(HD, (100.0, 900.0), Corner::BottomRight, 0.9, 24.0),
            None
        );
    }

    #[test]
    fn placing_scales_the_source_into_the_slot() {
        let slot = Rect {
            x: 100.0,
            y: 50.0,
            width: 480.0,
            height: 270.0,
        };
        let [a, b, c, d, tx, ty] = place((1920.0, 1080.0), slot, false);
        assert_eq!((a, d), (0.25, 0.25));
        assert_eq!((b, c), (0.0, 0.0));
        assert_eq!((tx, ty), (100.0, 50.0));
    }

    // Mirroring is one transform, not a flip followed by a move: the self-view
    // is the thing a person stares at, and two resamples show.
    #[test]
    fn mirroring_flips_horizontally_and_lands_in_the_same_slot() {
        let slot = Rect {
            x: 100.0,
            y: 50.0,
            width: 480.0,
            height: 270.0,
        };
        let [a, _, _, d, tx, ty] = place((1920.0, 1080.0), slot, true);
        assert_eq!(a, -0.25, "negative horizontal scale is the mirror");
        assert_eq!(d, 0.25, "and the vertical is untouched");
        assert_eq!(
            tx, 580.0,
            "shifted by its own width so it lands where it should"
        );
        assert_eq!(ty, 50.0);

        // the mirrored image occupies exactly the same rectangle
        let left = tx + 1920.0 * a;
        assert_eq!(left, 100.0, "its left edge is the slot's left edge");
    }
}

/// How wide a thumbnail is, and what shape it comes back in.
///
/// 960 across, which is half the output. It was 480, and 480 is what a Retina
/// display makes look soft: a panel drawing this 400 points wide is drawing it
/// at 800 pixels, so 480 is stretched and a person cannot read their own
/// screen in their own preview. Half the output is a 2x panel
/// showing it at up to 480 points with no upscale at all.
///
/// Sent as JPEG rather than PNG because this goes down a socket once a second:
/// measured on the real picture, a PNG of the same frame is over two hundred
/// kilobytes and a JPEG at this size and quality is around ninety, on a unix
/// socket that moves hundreds of megabytes a second.
pub const THUMBNAIL_WIDTH: u32 = 960;

/// The quality a thumbnail is written at.
///
/// 0.7, up from 0.5. At 480 the artefacts were smaller than the softness and
/// nobody could see them; at 960 they are what is left, and text on a shared
/// screen is exactly the thing JPEG at 0.5 destroys.
pub const THUMBNAIL_QUALITY: f64 = 0.7;

/// The height that keeps a picture of this shape, at that width.
pub fn thumbnail(of: (f64, f64)) -> (u32, u32) {
    if of.0 <= 0.0 || of.1 <= 0.0 {
        return (0, 0);
    }
    let scale = f64::from(THUMBNAIL_WIDTH) / of.0;
    (THUMBNAIL_WIDTH, (of.1 * scale).round().max(1.0) as u32)
}

#[cfg(test)]
mod thumbnail_tests {
    use super::*;

    #[test]
    fn a_thumbnail_keeps_the_shape_of_what_it_is_of() {
        assert_eq!(thumbnail((1920.0, 1080.0)), (960, 540));
        assert_eq!(thumbnail((1280.0, 960.0)), (960, 720));
    }

    #[test]
    fn nothing_has_no_thumbnail_rather_than_a_division_by_zero() {
        assert_eq!(thumbnail((0.0, 0.0)), (0, 0));
        assert_eq!(thumbnail((1920.0, 0.0)), (0, 0));
    }
}

/// Whether the camera is drawn into the picture. Not over a shared display
/// while a face draws its self-view window on it (the panel's floating
/// window): that window is in the capture already, where the operator
/// placed it, and drawing the camera too puts the same face on the picture
/// twice. With no such window (the CLI alone), over a shared window or over
/// a card, it is drawn.
#[must_use]
pub fn camera_is_composited(sharing_a_whole_display: bool, a_face_draws_a_self_view: bool) -> bool {
    !(sharing_a_whole_display && a_face_draws_a_self_view)
}

#[cfg(test)]
mod camera_tests {
    use super::*;

    #[test]
    fn a_shared_display_carries_the_self_view_only_while_a_face_draws_one() {
        assert!(!camera_is_composited(true, true));
        assert!(
            camera_is_composited(true, false),
            "the CLI alone: no window on the display"
        );
        assert!(camera_is_composited(false, true));
        assert!(camera_is_composited(false, false));
    }
}
