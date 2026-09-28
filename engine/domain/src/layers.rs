//! Ephemeral CLI-controlled overlays. Order in the list is back to front.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Camera,
    Window,
    Screen,
}

/// A selected device or window: `handle` is its stable device ID or the
/// window server's decimal window ID; `name` is what a person reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Source {
    pub kind: Kind,
    pub handle: String,
    pub name: String,
    /// Native pixels captured from this source, before any crop or transform.
    pub width: u32,
    pub height: u32,
    /// The monitor's own identity, for a display on a platform that has one:
    /// what a saved layer is opened by, since `handle` is the display's
    /// number now and the numbers move when a monitor comes or goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable: Option<String>,
}

impl Source {
    /// Whether two sources are one physical capture: by the monitor's own
    /// identity when both have it, by kind and handle otherwise.
    pub fn same_capture(&self, other: &Source) -> bool {
        self.kind == other.kind
            && match (&self.stable, &other.stable) {
                (Some(a), Some(b)) => a == b,
                _ => self.handle == other.handle,
            }
    }
}

/// A rectangle in the source's native pixels, measured from its top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Crop {
    /// Core Image has a bottom-left origin; the CLI names the top-left.
    /// A resized source that no longer contains the crop is hidden, never
    /// replaced with the full uncropped picture.
    pub fn rect(self, size: (u32, u32)) -> Option<crate::scene::Rect> {
        self.validate(size).ok()?;
        Some(crate::scene::Rect {
            x: f64::from(self.x),
            y: f64::from(size.1 - self.y - self.height),
            width: f64::from(self.width),
            height: f64::from(self.height),
        })
    }

    pub fn validate(self, size: (u32, u32)) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self
                .x
                .checked_add(self.width)
                .is_none_or(|right| right > size.0)
            || self
                .y
                .checked_add(self.height)
                .is_none_or(|bottom| bottom > size.1)
        {
            return Err(format!(
                "crop must be a non-empty rectangle inside the {}x{} source",
                size.0, size.1
            ));
        }
        Ok(())
    }
}

/// Scene pixels, measured from the top left. Rotation is clockwise about the centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Transform {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub degrees: i32,
}

impl Transform {
    pub fn validate(self) -> Result<(), String> {
        if self.width == 0 || self.height == 0 || self.width > 8192 || self.height > 8192 {
            return Err("layer size must be between 1x1 and 8192x8192".into());
        }
        if self.x >= 1920
            || self.y >= 1080
            || i64::from(self.x) + i64::from(self.width) <= 0
            || i64::from(self.y) + i64::from(self.height) <= 0
        {
            return Err("layer viewport must intersect the 1920x1080 scene".into());
        }
        Ok(())
    }
}

impl Transform {
    /// Initially one scene pixel per captured source pixel; the scene clips
    /// anything outside 1920x1080 instead of silently stretching the source.
    pub fn native(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
            degrees: 0,
        }
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::native((480, 270))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Layer {
    pub id: String,
    pub source: Source,
    pub transform: Transform,
    /// Video visibility only. Capture keeps running so showing it again is instant.
    /// A hidden display's requested sound is gated out of the mix until shown.
    #[serde(default = "visible_by_default")]
    pub visible: bool,
    /// None means the entire source, not a stored crop of its former size.
    #[serde(default)]
    pub crop: Option<Crop>,
    /// Camera-only mask; other sources have no shape. Existing camera layers
    /// are rectangular until the operator asks for a circle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<crate::scene::CameraShape>,
    /// Flip only this camera, independently of other cameras.
    #[serde(default)]
    pub mirrored: bool,
    /// GLSL fragment shader applied to native captured pixels before layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<String>,
}

fn visible_by_default() -> bool {
    true
}

/// Resolve a camera without guessing among multiple independent captures.
/// Old single-camera verbs can use `None`; layer verbs supply an explicit ID.
pub fn camera<'a>(layers: &'a [Layer], id: Option<&str>) -> Result<&'a Layer, String> {
    if let Some(id) = id {
        let layer = layers
            .iter()
            .find(|layer| layer.id == id)
            .ok_or_else(|| format!("no layer {id:?}"))?;
        return (layer.source.kind == Kind::Camera)
            .then_some(layer)
            .ok_or_else(|| format!("layer {id:?} is not a camera"));
    }
    let mut cameras = layers
        .iter()
        .filter(|layer| layer.source.kind == Kind::Camera);
    let first = cameras
        .next()
        .ok_or("no camera layer; add one or specify its ID")?;
    if cameras.next().is_some() {
        return Err("more than one camera layer; specify its ID".into());
    }
    Ok(first)
}

/// A source's bottom-left Core Image affine matrix, preserving its aspect.
pub fn affine(from: (f64, f64), t: Transform) -> [f64; 6] {
    let scale = (t.width as f64 / from.0).min(t.height as f64 / from.1);
    let cx = t.x as f64 + t.width as f64 / 2.0;
    let cy = 1080.0 - t.y as f64 - t.height as f64 / 2.0;
    let radians = -(t.degrees as f64).to_radians();
    let (sin, cos) = radians.sin_cos();
    let a = scale * cos;
    let b = scale * sin;
    let c = -scale * sin;
    let d = scale * cos;
    // Rotate about the viewport centre, not the origin of the source.
    [
        a,
        b,
        c,
        d,
        cx - (a * from.0 + c * from.1) / 2.0,
        cy - (b * from.0 + d * from.1) / 2.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_keeps_centre_and_fit_preserves_aspect() {
        let t = Transform {
            x: 100,
            y: 200,
            width: 400,
            height: 300,
            degrees: 90,
        };
        let [a, b, c, d, tx, ty] = affine((800.0, 400.0), t);
        assert!((a * 400.0 + c * 200.0 + tx - 300.0).abs() < 0.001);
        assert!((b * 400.0 + d * 200.0 + ty - 730.0).abs() < 0.001);
        assert!((a * a + b * b - 0.25).abs() < 0.001);
    }

    #[test]
    fn crop_is_bounded_in_source_pixels_including_overflow() {
        let valid = Crop {
            x: 100,
            y: 50,
            width: 300,
            height: 200,
        };
        assert!(valid.validate((640, 480)).is_ok());
        assert_eq!(
            valid.rect((640, 480)),
            Some(crate::scene::Rect {
                x: 100.0,
                y: 230.0,
                width: 300.0,
                height: 200.0
            })
        );
        assert!(
            valid.rect((200, 200)).is_none(),
            "a smaller window must not expose its uncropped pixels"
        );
        for crop in [
            Crop {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
            Crop {
                x: 600,
                y: 0,
                width: 50,
                height: 20,
            },
            Crop {
                x: u32::MAX,
                y: 0,
                width: 20,
                height: 20,
            },
        ] {
            assert!(crop.validate((640, 480)).is_err());
        }
    }

    #[test]
    fn a_monitor_is_the_same_capture_by_its_identity_whatever_its_number() {
        let desk = |handle: &str, stable: Option<&str>| Source {
            kind: Kind::Screen,
            handle: handle.into(),
            name: "Display".into(),
            width: 0,
            height: 0,
            stable: stable.map(String::from),
        };
        assert!(desk("2", Some("A")).same_capture(&desk("1", Some("A"))));
        assert!(!desk("1", Some("A")).same_capture(&desk("1", Some("B"))));
        assert!(desk("1", None).same_capture(&desk("1", Some("A"))));
        assert!(!desk("1", None).same_capture(&desk("2", None)));
    }

    #[test]
    fn native_size_is_not_a_1920x1080_preset() {
        assert_eq!(
            Transform::native((853, 479)),
            Transform {
                x: 0,
                y: 0,
                width: 853,
                height: 479,
                degrees: 0
            }
        );
        assert!(Transform::native((3840, 2160)).validate().is_ok());
    }

    #[test]
    fn existing_saved_layers_without_visibility_still_show() {
        let old = serde_json::json!({
            "id": "desk",
            "source": { "kind": "screen", "handle": "1", "name": "Display", "width": 1920, "height": 1080 },
            "transform": { "x": 0, "y": 0, "width": 1920, "height": 1080, "degrees": 0 },
            "crop": null
        });
        let layer: Layer = serde_json::from_value(old).unwrap();
        assert!(layer.visible);
        assert_eq!(serde_json::to_value(&layer).unwrap()["visible"], true);
    }

    #[test]
    fn invalid_sizes_and_origins_are_refused() {
        for t in [
            Transform {
                width: 0,
                ..Transform::default()
            },
            Transform {
                x: 1920,
                ..Transform::default()
            },
        ] {
            assert!(t.validate().is_err());
        }
        assert!(Transform::default().validate().is_ok());
    }
}
