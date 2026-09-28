//! Ordered scene content and physical capture reuse.
use crate::layers::{Kind, Layer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ElementContent {
    Text { text: String },
    Timer { seconds: u32 },
}

/// A visual element in output pixels, with a stable identity within its scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Element {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default = "visible_by_default")]
    pub visible: bool,
    /// Shader over this generated layer's own pixels, before scene composition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<String>,
    #[serde(flatten)]
    pub content: ElementContent,
}

fn visible_by_default() -> bool {
    true
}

impl Element {
    pub fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 80
            && !self.id.chars().any(char::is_control)
            && self.x >= 0
            && self.y >= 0
            && self.width > 0
            && self.height > 0
            && (self.x as u32).saturating_add(self.width) <= 1920
            && (self.y as u32).saturating_add(self.height) <= 1080
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visual_content_has_stable_flat_wire_shape_and_valid_viewport() {
        let element = Element {
            id: "time".into(),
            x: 10,
            y: 20,
            width: 300,
            height: 90,
            visible: true,
            shader: None,
            content: ElementContent::Timer { seconds: 120 },
        };
        assert!(element.valid());
        let value = serde_json::to_value(&element).unwrap();
        assert_eq!(value["kind"], "timer");
        assert_eq!(value["seconds"], 120);
        assert_eq!(serde_json::from_value::<Element>(value).unwrap(), element);
        assert!(!Element {
            x: 1900,
            ..element.clone()
        }
        .valid());
        assert!(!Element {
            width: 0,
            ..element
        }
        .valid());
    }
}

pub fn defaults() -> Vec<Scene> {
    vec![Scene {
        name: crate::protocol::default_scene_name(),
        layers: vec![],
        elements: vec![],
        order: vec![],
        shader: None,
    }]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Scene {
    pub name: String,
    /// Back-to-front order of capture layers.
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// Generated content and captures share this single back-to-front order.
    /// Old saved scenes have no order; captures then elements is their original appearance.
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub elements: Vec<Element>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<String>,
}

impl Scene {
    pub fn ordered_ids(&self) -> Vec<String> {
        let ids: Vec<_> = self
            .layers
            .iter()
            .map(|l| &l.id)
            .chain(self.elements.iter().map(|e| &e.id))
            .collect();
        let mut order = Vec::new();
        for id in self.order.iter().chain(ids.iter().copied()) {
            if ids.contains(&id) && !order.contains(id) {
                order.push(id.clone());
            }
        }
        order
    }

    pub fn normalize_order(&mut self) {
        self.order = self.ordered_ids();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaptureKey {
    pub kind: Kind,
    pub handle: String,
}
impl CaptureKey {
    pub fn of(layer: &Layer) -> Self {
        Self {
            kind: layer.source.kind,
            handle: layer.source.handle.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub keep: Vec<CaptureKey>,
    pub open: Vec<CaptureKey>,
    pub close: Vec<CaptureKey>,
}
fn captures(layers: &[Layer]) -> Vec<CaptureKey> {
    let mut keys = Vec::new();
    for layer in layers {
        let key = CaptureKey::of(layer);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}
pub fn transition(from: &[Layer], to: &[Layer]) -> Transition {
    let old = captures(from);
    let next = captures(to);
    Transition {
        keep: next
            .iter()
            .filter(|key| old.contains(key))
            .cloned()
            .collect(),
        open: next
            .iter()
            .filter(|key| !old.contains(key))
            .cloned()
            .collect(),
        close: old
            .iter()
            .filter(|key| !next.contains(key))
            .cloned()
            .collect(),
    }
}
