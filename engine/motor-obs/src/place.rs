//! Where a layer's item sits in the libobs scene: arithmetic over the
//! domain's layer and the source's size, so the tests cover it without a
//! libobs, and the pipeline only hands the numbers over.
//!
//! The domain speaks scene pixels from the top left, which is how libobs
//! counts too. A layer's viewport is where its source is fitted, keeping the
//! source's aspect, and turned clockwise about the viewport's middle: an item
//! aligned on its centre, bounded to the viewport, rotated.

use remuxd_domain::layers::{Kind, Layer};
use remuxd_domain::scene::CameraShape;

/// A rectangle in a source's own pixels, from its top left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The viewport's middle, in scene pixels.
    pub centre: (f32, f32),
    /// The viewport's size: the source is fitted inside it.
    pub bounds: (f32, f32),
    pub degrees: f32,
    /// Off each edge of the source: left, top, right, bottom.
    pub crop: (i32, i32, i32, i32),
    pub mirrored: bool,
    pub visible: bool,
    /// A camera cut to a circle: the square of the source it is cut from,
    /// which is where the mask's circle goes.
    pub circle: Option<Region>,
}

/// The part of the source the layer shows: its crop, or all of it, and for a
/// circle the centred square of that. `None` when the source has not said
/// its size or no longer contains the crop: hidden, never the uncropped
/// picture in its place.
pub fn shown(layer: &Layer, size: (u32, u32)) -> Option<Region> {
    if size.0 == 0 || size.1 == 0 {
        return None;
    }
    let region = match layer.crop {
        Some(crop) => {
            crop.validate(size).ok()?;
            Region {
                x: crop.x,
                y: crop.y,
                width: crop.width,
                height: crop.height,
            }
        }
        None => Region {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        },
    };
    Some(if circle(layer) {
        let side = region.width.min(region.height);
        Region {
            x: region.x + (region.width - side) / 2,
            y: region.y + (region.height - side) / 2,
            width: side,
            height: side,
        }
    } else {
        region
    })
}

fn circle(layer: &Layer) -> bool {
    layer.source.kind == Kind::Camera && layer.shape == Some(CameraShape::Circle)
}

pub fn placement(layer: &Layer, size: (u32, u32)) -> Placement {
    let t = layer.transform;
    let region = shown(layer, size);
    Placement {
        centre: (
            t.x as f32 + t.width as f32 / 2.0,
            t.y as f32 + t.height as f32 / 2.0,
        ),
        bounds: (t.width as f32, t.height as f32),
        degrees: t.degrees as f32,
        crop: region.map_or((0, 0, 0, 0), |r| {
            (
                r.x as i32,
                r.y as i32,
                (size.0 - r.x - r.width) as i32,
                (size.1 - r.y - r.height) as i32,
            )
        }),
        mirrored: layer.source.kind == Kind::Camera && layer.mirrored,
        visible: layer.visible && region.is_some(),
        circle: region.filter(|_| circle(layer)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remuxd_domain::layers::{Crop, Source, Transform};

    fn layer(kind: Kind) -> Layer {
        Layer {
            id: "face".into(),
            source: Source {
                kind,
                handle: "cam".into(),
                name: "Cam".into(),
                width: 1280,
                height: 720,
            },
            transform: Transform {
                x: 100,
                y: 200,
                width: 480,
                height: 270,
                degrees: 90,
            },
            visible: true,
            crop: None,
            shape: None,
            mirrored: false,
            shader: None,
        }
    }

    #[test]
    fn the_viewport_s_middle_is_where_the_item_turns() {
        let placed = placement(&layer(Kind::Screen), (1280, 720));
        assert_eq!(placed.centre, (340.0, 335.0));
        assert_eq!(placed.bounds, (480.0, 270.0));
        assert_eq!(placed.degrees, 90.0);
        assert_eq!(placed.crop, (0, 0, 0, 0));
        assert!(placed.visible && !placed.mirrored && placed.circle.is_none());
    }

    #[test]
    fn a_crop_is_said_as_what_comes_off_each_edge() {
        let mut cropped = layer(Kind::Window);
        cropped.crop = Some(Crop {
            x: 100,
            y: 50,
            width: 300,
            height: 200,
        });
        assert_eq!(placement(&cropped, (1280, 720)).crop, (100, 50, 880, 470));
        let shrunk = placement(&cropped, (200, 200));
        assert!(!shrunk.visible, "a window that shrank past its crop hides");
    }

    #[test]
    fn a_circle_is_cut_from_the_middle_of_what_is_shown_and_only_on_a_camera() {
        let mut face = layer(Kind::Camera);
        face.shape = Some(CameraShape::Circle);
        face.mirrored = true;
        let placed = placement(&face, (1280, 720));
        assert_eq!(placed.crop, (280, 0, 280, 0));
        assert_eq!(
            placed.circle,
            Some(Region {
                x: 280,
                y: 0,
                width: 720,
                height: 720
            })
        );
        assert!(placed.mirrored);
        face.crop = Some(Crop {
            x: 0,
            y: 0,
            width: 640,
            height: 400,
        });
        assert_eq!(placement(&face, (1280, 720)).crop, (120, 0, 760, 320));
        let mut screen = layer(Kind::Screen);
        screen.shape = Some(CameraShape::Circle);
        screen.mirrored = true;
        let placed = placement(&screen, (1280, 720));
        assert!(placed.circle.is_none() && !placed.mirrored);
    }

    #[test]
    fn nothing_is_shown_before_the_source_has_a_size() {
        assert!(!placement(&layer(Kind::Camera), (0, 0)).visible);
        let mut hidden = layer(Kind::Camera);
        hidden.visible = false;
        assert!(!placement(&hidden, (1280, 720)).visible);
    }
}
