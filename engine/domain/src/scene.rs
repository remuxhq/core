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

/// How the camera and the screen share the picture.
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
pub enum Mode {
    /// The camera in its corner, over the screen.
    #[default]
    Overlay,
    /// The screen on the left, the camera in a column on the right.
    Columns,
    /// The camera drifting across the screen, bouncing off the edges.
    Bounce,
}

/// The camera's outline: as it comes, cut to a square, or a circle.
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
pub enum Shape {
    #[default]
    Rectangle,
    Square,
    Circle,
}

/// A look on the camera, applied in the compositor.
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
pub enum Filter {
    #[default]
    Plain,
    Sepia,
    Mono,
    Noir,
}

/// Where and how the camera sits in the picture: one value the engine holds,
/// every face reads, and a shell or a dragged window changes.
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Layout {
    /// The camera in a corner over the screen, beside it in a column of its
    /// own, or wandering the picture like the DVD logo.
    #[serde(default)]
    pub mode: Mode,
    pub corner: Corner,
    /// How wide the camera is, as a share of the picture's width.
    pub share: f64,
    pub margin: f64,
    pub shape: Shape,
    #[serde(default)]
    pub filter: Filter,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            corner: Corner::default(),
            share: CAMERA_SHARE,
            margin: CAMERA_MARGIN,
            shape: Shape::default(),
            filter: Filter::default(),
        }
    }
}

/// A change to some of the layout: what one verb or one drag moved.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
pub struct LayoutPatch {
    pub mode: Option<Mode>,
    pub corner: Option<Corner>,
    pub share: Option<f64>,
    pub margin: Option<f64>,
    pub shape: Option<Shape>,
    pub filter: Option<Filter>,
}

/// Where everything goes for one frame: the screen's rectangle and the
/// camera's slot with the part of it that is shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub screen: Rect,
    pub camera: Option<(Rect, Rect)>,
}

/// The camera on the move, for [`Mode::Bounce`]: where it is and where it
/// is going, in output pixels a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounce {
    pub x: f64,
    pub y: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Default for Bounce {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            dx: 3.0,
            dy: 2.0,
        }
    }
}

impl Bounce {
    /// One frame on: a slot of this size moves and turns at the edges.
    pub fn step(&mut self, output: (f64, f64), slot: (f64, f64)) {
        let (max_x, max_y) = ((output.0 - slot.0).max(0.0), (output.1 - slot.1).max(0.0));
        self.x += self.dx;
        self.y += self.dy;
        if self.x <= 0.0 || self.x >= max_x {
            self.dx = -self.dx;
            self.x = self.x.clamp(0.0, max_x);
        }
        if self.y <= 0.0 || self.y >= max_y {
            self.dy = -self.dy;
            self.y = self.y.clamp(0.0, max_y);
        }
    }
}

impl Layout {
    /// The whole frame placed. `screen` is the screen's own size, so a
    /// column keeps its shape; `bounced` is where the bounce has got to.
    pub fn placed(
        self,
        output: (f64, f64),
        screen: Option<(f64, f64)>,
        camera: Option<(f64, f64)>,
        bounced: Option<&Bounce>,
    ) -> Placed {
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: output.0,
            height: output.1,
        };
        match self.mode {
            Mode::Overlay => Placed {
                screen: full,
                camera: camera.and_then(|cam| self.slot(output, cam)),
            },
            Mode::Bounce => Placed {
                screen: full,
                camera: camera.and_then(|cam| {
                    let (slot, source) = self.slot(output, cam)?;
                    let at = bounced.copied().unwrap_or_default();
                    Some((
                        Rect {
                            x: at.x,
                            y: at.y,
                            ..slot
                        },
                        source,
                    ))
                }),
            },
            Mode::Columns => {
                let column = output.0 * self.share.clamp(0.2, 0.5);
                let left = Rect {
                    x: 0.0,
                    y: 0.0,
                    width: output.0 - column,
                    height: output.1,
                };
                let screen = screen.map_or(left, |from| fit(from, left));
                let camera = camera.and_then(|cam| {
                    let source = self.source(cam);
                    let width = column - 2.0 * self.margin;
                    let height = width * source.height / source.width;
                    (width > 0.0 && height <= output.1).then_some((
                        Rect {
                            x: left.width + self.margin,
                            y: (output.1 - height) / 2.0,
                            width,
                            height,
                        },
                        source,
                    ))
                });
                Placed { screen, camera }
            }
        }
    }

    /// The part of the camera the shape keeps.
    fn source(self, camera: (f64, f64)) -> Rect {
        match self.shape {
            Shape::Rectangle => Rect {
                x: 0.0,
                y: 0.0,
                width: camera.0,
                height: camera.1,
            },
            Shape::Square | Shape::Circle => {
                let side = camera.0.min(camera.1);
                Rect {
                    x: (camera.0 - side) / 2.0,
                    y: (camera.1 - side) / 2.0,
                    width: side,
                    height: side,
                }
            }
        }
    }

    pub fn patched(self, patch: LayoutPatch) -> Self {
        Self {
            mode: patch.mode.unwrap_or(self.mode),
            corner: patch.corner.unwrap_or(self.corner),
            share: patch.share.unwrap_or(self.share).clamp(0.05, 1.0),
            margin: patch.margin.unwrap_or(self.margin).max(0.0),
            shape: patch.shape.unwrap_or(self.shape),
            filter: patch.filter.unwrap_or(self.filter),
        }
    }

    /// The camera's slot for a camera of this size, and the part of the
    /// camera that fills it: the whole frame, or the centred square a square
    /// or a circle is cut from.
    pub fn slot(self, output: (f64, f64), camera: (f64, f64)) -> Option<(Rect, Rect)> {
        let source = self.source(camera);
        let slot = camera_slot(
            output,
            (source.width, source.height),
            self.corner,
            self.share,
            self.margin,
        )?;
        Some((slot, source))
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
    fn columns_put_the_screen_left_whole_and_the_camera_right_centred() {
        let layout = Layout {
            mode: Mode::Columns,
            share: 0.25,
            margin: 24.0,
            ..Layout::default()
        };
        let placed = layout.placed(HD, Some((1920.0, 1080.0)), Some((1280.0, 960.0)), None);
        assert_eq!((placed.screen.x, placed.screen.width), (0.0, 1440.0));
        assert_eq!(
            placed.screen.height, 810.0,
            "the screen keeps 16:9 in a 1440 column"
        );
        assert_eq!(placed.screen.y, 135.0, "centred");
        let (slot, _) = placed.camera.unwrap();
        assert_eq!((slot.x, slot.width), (1464.0, 432.0));
        assert_eq!(slot.height, 324.0, "4:3 kept");
        assert_eq!(slot.y, 378.0);
        assert_eq!(
            Layout::default().placed(HD, None, None, None).screen.width,
            1920.0
        );
    }

    #[test]
    fn the_bounce_turns_at_the_edges_and_places_the_camera_where_it_is() {
        let mut bounce = Bounce {
            x: 1438.0,
            y: 0.0,
            dx: 3.0,
            dy: -2.0,
        };
        bounce.step(HD, (480.0, 270.0));
        assert_eq!(
            (bounce.x, bounce.dx),
            (1440.0, -3.0),
            "hit the right edge and turned"
        );
        assert_eq!(
            (bounce.y, bounce.dy),
            (0.0, 2.0),
            "hit the bottom and turned"
        );
        bounce.step(HD, (480.0, 270.0));
        assert_eq!((bounce.x, bounce.y), (1437.0, 2.0));
        let layout = Layout {
            mode: Mode::Bounce,
            ..Layout::default()
        };
        let placed = layout.placed(HD, None, Some((1920.0, 1080.0)), Some(&bounce));
        let (slot, _) = placed.camera.unwrap();
        assert_eq!((slot.x, slot.y, slot.width), (1437.0, 2.0, 480.0));
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

#[cfg(test)]
mod layout_tests {
    use super::*;

    const HD: (f64, f64) = (1920.0, 1080.0);

    #[test]
    fn a_square_or_a_circle_is_cut_from_the_middle_of_the_camera_and_keeps_its_width() {
        let layout = Layout {
            shape: Shape::Circle,
            corner: Corner::TopLeft,
            ..Layout::default()
        };
        let (slot, source) = layout.slot(HD, (1280.0, 720.0)).expect("it fits");
        assert_eq!(
            (source.x, source.y, source.width, source.height),
            (280.0, 0.0, 720.0, 720.0)
        );
        assert_eq!(
            (slot.width, slot.height),
            (480.0, 480.0),
            "square in, square out"
        );
        assert_eq!((slot.x, slot.y), (24.0, 1080.0 - 24.0 - 480.0), "top left");
    }

    #[test]
    fn a_patch_moves_only_what_it_names_and_keeps_the_share_sane() {
        let moved = Layout::default().patched(LayoutPatch {
            share: Some(0.5),
            ..LayoutPatch::default()
        });
        assert_eq!(
            (moved.corner, moved.share, moved.shape),
            (Corner::BottomRight, 0.5, Shape::Rectangle)
        );
        assert_eq!(
            Layout::default()
                .patched(LayoutPatch {
                    share: Some(9.0),
                    ..LayoutPatch::default()
                })
                .share,
            1.0
        );
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
