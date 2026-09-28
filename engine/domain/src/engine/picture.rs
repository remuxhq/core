//! The picture: what is behind it, what is drawn over it, and the self-view.

use super::*;
use base64::Engine as _;

/// A failed source change may also lose the original device (for example,
/// when its window closed during the swap). Never report it still active.
#[derive(Debug)]
pub struct LayerSwapError {
    pub reason: String,
    pub restored: bool,
}

/// The picture's half of the media path: what is captured and what is drawn
/// over it. See [`crate::engine::Pipeline`] for why it is a port.
pub trait Picture: Send {
    /// Render visual elements above the capture layers. Deadlines are transient.
    fn show(
        &mut self,
        elements: &[Element],
        timers: &[(String, Duration)],
        order: &[String],
    ) -> Result<(), String>;
    fn stop(&mut self);
    /// One frame of what is going out, already small and already JPEG.
    ///
    /// `None` when there is no picture yet, which a panel draws as an empty
    /// frame rather than as an error: an engine that has not been pointed at
    /// anything is not broken.
    fn shot(&mut self, of: Framed) -> Option<(Vec<u8>, u32, u32)>;
    fn layer_shot(&mut self, _id: &str) -> Option<(Vec<u8>, u32, u32)> {
        None
    }
    fn element_shot(
        &mut self,
        _element: &Element,
        _time: Option<Duration>,
    ) -> Option<(Vec<u8>, u32, u32)> {
        None
    }
    /// Legacy panel self-view flip. Broadcast cameras use their individual
    /// layer mirror properties instead.
    fn mirror(&mut self, on: bool);
    /// An overlay's capture is started before the engine announces it in Status.
    fn layer_add(&mut self, _layer: &crate::layers::Layer) -> Result<(u32, u32), String> {
        Err("this pipeline cannot capture layers".into())
    }
    fn layer_remove(&mut self, _id: &str) {}
    /// Stop the old capture before starting the new one under the same ID.
    /// On failure restore the old capture or explicitly report its loss.
    fn layer_replace(
        &mut self,
        old: &crate::layers::Layer,
        new: &crate::layers::Layer,
    ) -> Result<(u32, u32), LayerSwapError>;
    /// Prepare every new physical capture before changing the compositor or
    /// releasing the old scene. On error the old scene must remain intact.
    fn scene_transition(
        &mut self,
        _from: &[crate::layers::Layer],
        _to: &[crate::layers::Layer],
        _elements: &[Element],
        _shader: Option<&str>,
    ) -> Result<(), String> {
        Err("this pipeline cannot switch scenes".into())
    }
    fn layers_changed(&mut self, _layers: &[crate::layers::Layer]) {}
    /// Route system audio from exactly one display layer, or disconnect it.
    fn screen_audio(&mut self, _id: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    /// Compile a scene shader before switching the running compositor.
    fn shader(&mut self, path: Option<&str>) -> Result<(), String>;
    /// Compile before replacing this layer's shader. `None` removes it.
    fn layer_shader(
        &mut self,
        _layer: &crate::layers::Layer,
        path: Option<&str>,
    ) -> Result<(), String> {
        if path.is_none() {
            Ok(())
        } else {
            Err("this pipeline has no layer shaders".into())
        }
    }
    fn element_shader(&mut self, _element: &Element, path: Option<&str>) -> Result<(), String> {
        if path.is_none() {
            Ok(())
        } else {
            Err("this pipeline has no element shaders".into())
        }
    }
    fn layer_shader_active(&self, _id: &str) -> bool {
        true
    }
    /// A GPU failure may disable a shader after selection; status must not lie.
    fn shader_active(&self) -> bool {
        true
    }
    fn flowing(&self) -> Flowing;
    /// Counters for the exact capture named by ID, never an arbitrary camera or display.
    fn layer_flowing(&self, _id: &str) -> Flowing {
        Flowing::default()
    }
    /// Where a panel can map the preview, when there is one. Read every time
    /// rather than kept: an engine that lost its region should stop claiming
    /// to have one.
    fn preview(&self) -> Option<crate::preview::Preview> {
        None
    }
    /// Whether anybody is drawing the preview, so the pipeline can stop making
    /// one for nobody. See [`Command::Watching`].
    fn previewing(&mut self, _on: bool) {}
}

impl Engine {
    /// A lease a face renews while it draws, once a second. "Off" is
    /// accepted and does nothing: a face that stopped drawing stops
    /// renewing, and the lease runs out by itself, so two windows and
    /// one closing never blinks the other. It used to be a count, and
    /// a count is state this engine holds for a face: an engine
    /// restarted under a live panel started at zero and never
    /// published again, which read as the picture freezing.
    pub(super) fn watch(&mut self, on: bool) -> Reply {
        if on {
            self.watch_lease = WATCH_LEASE_TICKS;
            self.pipeline.previewing(true);
        }
        Reply::Ok
    }

    /// Choosing what is in the picture. Each answers with the name of
    /// what it chose rather than a bare ok, because "screen 3" and
    /// "the one called VG2791R" are different amounts of confidence
    /// and a person about to go live wants the second.
    pub(super) fn choose_screen(&mut self, display: u32) -> Reply {
        if let Err(message) =
            self.single_layer(&[crate::layers::Kind::Screen, crate::layers::Kind::Window])
        {
            return Reply::Error { message };
        }
        match self.screen_source(display) {
            Ok(source) => self.select_single_source(source),
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn choose_window(&mut self, query: String) -> Reply {
        if let Err(message) =
            self.single_layer(&[crate::layers::Kind::Screen, crate::layers::Kind::Window])
        {
            return Reply::Error { message };
        }
        match self.window_source(&query) {
            Ok(source) => self.select_single_source(source),
            Err(message) => Reply::Error { message },
        }
    }

    /// The camera and the microphone. `None` turns one off, which is a
    /// different thing from never having chosen one: the camera is a
    /// slot in the picture and the mic is a channel in the mix, and
    /// both are allowed to be empty.
    pub(super) fn choose_camera(&mut self, device: Option<String>) -> Reply {
        match device {
            Some(query) => {
                if let Err(message) = self.single_layer(&[crate::layers::Kind::Camera]) {
                    return Reply::Error { message };
                }
                match self.camera_source(&query) {
                    Ok(source) => self.select_single_source(source),
                    Err(message) => Reply::Error { message },
                }
            }
            None => match self.single_layer(&[crate::layers::Kind::Camera]) {
                Ok(Some(id)) => self.layer_remove(id),
                Ok(None) => Reply::Status(Box::new(self.reported())),
                Err(message) => Reply::Error { message },
            },
        }
    }

    fn single_layer(&self, kinds: &[crate::layers::Kind]) -> Result<Option<String>, String> {
        let mut found = self
            .status
            .layers
            .iter()
            .filter(|l| kinds.contains(&l.source.kind));
        let first = found.next().map(|l| l.id.clone());
        if found.next().is_some() {
            return Err("more than one matching layer; specify a layer ID".into());
        }
        Ok(first)
    }

    /// Generated IDs are ordinary IDs, not a hidden slot or a reserved name.
    /// Commands always resolve by kind and cardinality, never this spelling.
    fn available_layer_id(&self, prefix: &str) -> String {
        (1..)
            .map(|n| format!("{prefix}-{n}"))
            .find(|id| self.status.layers.iter().all(|layer| &layer.id != id))
            .expect("there is always a free layer ID")
    }

    fn select_single_source(&mut self, source: crate::layers::Source) -> Reply {
        let visual = [crate::layers::Kind::Screen, crate::layers::Kind::Window];
        let camera = [crate::layers::Kind::Camera];
        let (kinds, prefix): (&[_], &str) = if source.kind == crate::layers::Kind::Camera {
            (&camera, "camera")
        } else {
            (&visual, "source")
        };
        match self.single_layer(kinds) {
            Ok(Some(id)) => self.replace_layer_source(id, source),
            Ok(None) => self.add_layer(Self::layer_with_source(
                self.available_layer_id(prefix),
                source,
            )),
            Err(message) => Reply::Error { message },
        }
    }

    /// Stop sharing the one selected visual source without changing scenes.
    pub(super) fn share(&mut self, on: bool) -> Reply {
        if on {
            return Reply::Error {
                message: "choose a screen or a window to share".into(),
            };
        }
        match self.single_layer(&[crate::layers::Kind::Screen, crate::layers::Kind::Window]) {
            Ok(Some(id)) => self.layer_remove(id),
            Ok(None) => Reply::Status(Box::new(self.reported())),
            Err(message) => Reply::Error { message },
        }
    }

    fn active_scene_order(&self) -> Vec<String> {
        self.status
            .scenes
            .iter()
            .find(|s| s.name == self.status.active_scene)
            .map(|s| s.ordered_ids())
            .unwrap_or_default()
    }

    fn sync_scene_layers(&mut self) {
        if let Some(scene) = self
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == self.status.active_scene)
        {
            scene.layers = self.status.layers.clone();
            scene.normalize_order();
        }
        self.pipeline.layers_changed(&self.status.layers);
        self.render_scene();
    }

    fn active_elements(&self) -> Vec<Element> {
        self.status
            .scenes
            .iter()
            .find(|s| s.name == self.status.active_scene)
            .map(|s| s.elements.clone())
            .unwrap_or_default()
    }

    pub(super) fn scene_element_add(&mut self, element: Element) -> Reply {
        if !element.valid() {
            return Reply::Error {
                message: "element viewport must fit inside 1920x1080 and ID must be printable"
                    .into(),
            };
        }
        let scene = self
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == self.status.active_scene)
            .expect("active scene");
        if scene.ordered_ids().contains(&element.id) {
            return Reply::Error {
                message: format!("element {:?} already exists", element.id),
            };
        }
        if let Some(path) = element.shader.as_deref() {
            if let Err(message) = self.pipeline.element_shader(&element, Some(path)) {
                return Reply::Error { message };
            }
        }
        scene.order.push(element.id.clone());
        scene.elements.push(element);
        self.render_scene();
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn scene_element_set(&mut self, element: Element) -> Reply {
        if !element.valid() {
            return Reply::Error {
                message: "element viewport must fit inside 1920x1080 and ID must be printable"
                    .into(),
            };
        }
        let scene = self
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == self.status.active_scene)
            .expect("active scene");
        let Some(old) = scene.elements.iter_mut().find(|e| e.id == element.id) else {
            return Reply::Error {
                message: format!("no element {:?}", element.id),
            };
        };
        if old.content != element.content {
            self.counting.remove(&element.id);
        }
        let visible = old.visible;
        let shader = old.shader.clone();
        let next = Element {
            visible,
            shader: shader.clone(),
            ..element
        };
        if (old.width, old.height) != (next.width, next.height) && shader.is_some() {
            if let Err(message) = self.pipeline.element_shader(&next, shader.as_deref()) {
                return Reply::Error { message };
            }
        }
        *old = next;
        self.render_scene();
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn scene_element_remove(&mut self, id: String) -> Reply {
        let scene = self
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == self.status.active_scene)
            .expect("active scene");
        let before = scene.elements.len();
        let removed = scene.elements.iter().find(|e| e.id == id).cloned();
        scene.elements.retain(|e| e.id != id);
        if before == scene.elements.len() {
            return Reply::Error {
                message: format!("no element {id:?}"),
            };
        }
        scene.normalize_order();
        if let Some(element) = removed {
            let _ = self.pipeline.element_shader(&element, None);
        }
        self.counting.remove(&id);
        self.render_scene();
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn scene_timer(&mut self, id: String, start: bool) -> Reply {
        let Some(seconds) =
            self.active_elements()
                .iter()
                .find(|e| e.id == id)
                .and_then(|e| match e.content {
                    ElementContent::Timer { seconds } => Some(seconds),
                    _ => None,
                })
        else {
            return Reply::Error {
                message: format!("no timer {id:?} in active scene"),
            };
        };
        if start {
            self.counting
                .insert(id, Instant::now() + Duration::from_secs(seconds as u64));
        } else {
            self.counting.remove(&id);
        }
        self.render_scene();
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn render_scene(&mut self) {
        let elements = self.active_elements();
        let timers = self
            .counting
            .iter()
            .map(|(id, deadline)| {
                (
                    id.clone(),
                    deadline.saturating_duration_since(Instant::now()),
                )
            })
            .collect::<Vec<_>>();
        let order = self.active_scene_order();
        let _ = self.pipeline.show(&elements, &timers, &order);
    }

    /// A picture of what is going out, for a panel to draw. It asks;
    /// the engine does not push, and at one a second that is the same
    /// thing with less to go wrong.
    pub(super) fn shot(&mut self, of: Framed) -> Reply {
        let kind = match of {
            Framed::Camera => Some(crate::layers::Kind::Camera),
            Framed::Screen => Some(crate::layers::Kind::Screen),
            Framed::Scene => None,
        };
        if let Some(kind) = kind {
            let mut matches = self
                .status
                .layers
                .iter()
                .filter(|layer| layer.source.kind == kind);
            if matches.next().is_some() && matches.next().is_some() {
                return Reply::Error {
                    message: "more than one source layer; use layer-shot with its ID".into(),
                };
            }
        }
        match self.pipeline.shot(of) {
            Some((jpeg, width, height)) => Reply::Shot {
                jpeg: base64::engine::general_purpose::STANDARD.encode(&jpeg),
                width,
                height,
            },
            None => Reply::Error {
                message: match of {
                    Framed::Camera => "no camera is open".into(),
                    Framed::Screen => "no screen is being shared".into(),
                    Framed::Scene => "there is no picture yet".into(),
                },
            },
        }
    }

    pub(super) fn layer_shot(&mut self, id: String) -> Reply {
        if !self.status.layers.iter().any(|layer| layer.id == id) {
            if let Some(element) = self
                .active_elements()
                .into_iter()
                .find(|element| element.id == id)
            {
                let left = self
                    .counting
                    .get(&id)
                    .map(|deadline| deadline.saturating_duration_since(Instant::now()));
                return match self.pipeline.element_shot(&element, left) {
                    Some((jpeg, width, height)) => Reply::Shot {
                        jpeg: base64::engine::general_purpose::STANDARD.encode(jpeg),
                        width,
                        height,
                    },
                    None => Reply::Error {
                        message: format!("layer {id:?} has no picture yet"),
                    },
                };
            }
            return Reply::Error {
                message: format!("no layer {id:?}"),
            };
        }
        match self.pipeline.layer_shot(&id) {
            Some((jpeg, width, height)) => Reply::Shot {
                jpeg: base64::engine::general_purpose::STANDARD.encode(jpeg),
                width,
                height,
            },
            None => Reply::Error {
                message: format!("layer {id:?} has no picture yet"),
            },
        }
    }

    pub(super) fn mirror(&mut self, on: bool) -> Reply {
        self.status.mirrored = on;
        self.pipeline.mirror(on);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn camera_position(&mut self, at: Option<crate::scene::CameraPosition>) -> Reply {
        match crate::layers::camera(&self.status.layers, None) {
            Ok(layer) => self.layer_position(layer.id.clone(), at),
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn camera_shape(&mut self, shape: crate::scene::CameraShape) -> Reply {
        match crate::layers::camera(&self.status.layers, None) {
            Ok(layer) => self.layer_shape(layer.id.clone(), shape),
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn layer_camera(&mut self, id: String, query: String) -> Reply {
        if let Err(message) = self.validate_layer_id(&id) {
            return Reply::Error { message };
        }
        match self.camera_source(&query) {
            Ok(source) => self.add_layer(Self::layer_with_source(id, source)),
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn layer_replace_camera(&mut self, id: String, device: String) -> Reply {
        if let Err(message) = self.validate_replacement(&id, &[crate::layers::Kind::Camera]) {
            return Reply::Error { message };
        }
        match self.camera_source(&device) {
            Ok(source) => self.replace_layer_source(id, source),
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn layer_screen(&mut self, id: String, display: u32) -> Reply {
        if let Err(message) = self.validate_layer_id(&id) {
            return Reply::Error { message };
        }
        match self.screen_source(display) {
            Ok(source) => self.add_layer(Self::layer_with_source(id, source)),
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn layer_replace_screen(&mut self, id: String, display: u32) -> Reply {
        if let Err(message) = self.validate_replacement(
            &id,
            &[crate::layers::Kind::Screen, crate::layers::Kind::Window],
        ) {
            return Reply::Error { message };
        }
        match self.screen_source(display) {
            Ok(source) => self.replace_layer_source(id, source),
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn layer_window(&mut self, id: String, query: String) -> Reply {
        if let Err(message) = self.validate_layer_id(&id) {
            return Reply::Error { message };
        }
        match self.window_source(&query) {
            Ok(source) => self.add_layer(Self::layer_with_source(id, source)),
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn layer_replace_window(&mut self, id: String, query: String) -> Reply {
        if let Err(message) = self.validate_replacement(
            &id,
            &[crate::layers::Kind::Screen, crate::layers::Kind::Window],
        ) {
            return Reply::Error { message };
        }
        match self.window_source(&query) {
            Ok(source) => self.replace_layer_source(id, source),
            Err(message) => Reply::Error { message },
        }
    }

    fn screen_source(&self, display: u32) -> Result<crate::layers::Source, String> {
        let available = self.sources.available()?;
        let screen = available
            .screens
            .iter()
            .find(|s| s.id.0 == display)
            .ok_or_else(|| {
                format!(
                    "no display {display}. There is {}",
                    list_of(available.screens.iter().map(|s| s.name.clone()))
                )
            })?;
        Ok(crate::layers::Source {
            kind: crate::layers::Kind::Screen,
            handle: display.to_string(),
            name: screen.name.clone(),
            width: 0,
            height: 0,
            stable: screen.stable.clone(),
        })
    }

    fn window_source(&self, query: &str) -> Result<crate::layers::Source, String> {
        let available = self.sources.available()?;
        let window = available
            .windows
            .iter()
            .find(|w| window_label(w) == query)
            .or_else(|| {
                query
                    .parse::<u32>()
                    .ok()
                    .and_then(|id| available.windows.iter().find(|w| w.id.0 == id))
            })
            .or_else(|| pick(query, &available.windows))
            .ok_or_else(|| format!("no window matches {query:?}"))?;
        Ok(crate::layers::Source {
            kind: crate::layers::Kind::Window,
            handle: window.id.0.to_string(),
            name: window_label(window),
            width: 0,
            height: 0,
            stable: None,
        })
    }

    fn camera_source(&self, query: &str) -> Result<crate::layers::Source, String> {
        let available = self.sources.available()?;
        let camera = pick_device(query, &available.cameras).ok_or_else(|| {
            format!(
                "no camera matches {query:?}. There is {}",
                list_of(available.cameras.iter().map(|c| c.name.clone()))
            )
        })?;
        Ok(crate::layers::Source {
            kind: crate::layers::Kind::Camera,
            handle: camera.id.clone(),
            name: camera.name.clone(),
            width: 0,
            height: 0,
            stable: None,
        })
    }

    fn layer_with_source(id: String, source: crate::layers::Source) -> crate::layers::Layer {
        crate::layers::Layer {
            id,
            shape: (source.kind == crate::layers::Kind::Camera)
                .then_some(crate::scene::CameraShape::Rectangle),
            source,
            transform: crate::layers::Transform::default(),
            visible: true,
            crop: None,
            shader: None,
            mirrored: false,
        }
    }

    fn validate_replacement(&self, id: &str, kinds: &[crate::layers::Kind]) -> Result<(), String> {
        let layer = self
            .status
            .layers
            .iter()
            .find(|layer| layer.id == id)
            .ok_or_else(|| format!("no layer {id:?}"))?;
        if !kinds.contains(&layer.source.kind) {
            return Err(format!(
                "layer {id:?} is not a {}",
                if kinds.len() == 1 {
                    "camera"
                } else {
                    "display or window"
                }
            ));
        }
        Ok(())
    }

    fn replace_layer_source(&mut self, id: String, source: crate::layers::Source) -> Reply {
        let Some(index) = self.status.layers.iter().position(|layer| layer.id == id) else {
            return Reply::Error {
                message: format!("no layer {id:?}"),
            };
        };
        let old = self.status.layers[index].clone();
        let visual = [crate::layers::Kind::Screen, crate::layers::Kind::Window];
        if (old.source.kind == crate::layers::Kind::Camera)
            != (source.kind == crate::layers::Kind::Camera)
        {
            return Reply::Error {
                message: format!("layer {id:?} cannot change between camera and display/window"),
            };
        }
        if source.kind != crate::layers::Kind::Camera && !visual.contains(&source.kind) {
            return Reply::Error {
                message: "unsupported source".into(),
            };
        }
        // Selecting the same physical source is a no-op: never blink the live
        // or restart audio just because a client sent the same choice twice.
        if old.source.same_capture(&source) {
            let kept = &mut self.status.layers[index].source;
            kept.name = source.name;
            kept.handle = source.handle;
            kept.stable = source.stable;
            return Reply::Status(Box::new(self.reported()));
        }
        let mut next = old.clone();
        next.source = source;
        next.shape = if next.source.kind == crate::layers::Kind::Camera {
            old.shape.or(Some(crate::scene::CameraShape::Rectangle))
        } else {
            None
        };
        let sound_was_here = self.status.screen_sound_layer.as_deref() == Some(&id);
        match self.pipeline.layer_replace(&old, &next) {
            Ok((width, height)) if width > 0 && height > 0 && width <= 8192 && height <= 8192 => {
                next.source.width = width;
                next.source.height = height;
                if next
                    .crop
                    .is_some_and(|crop| crop.validate((width, height)).is_err())
                {
                    next.crop = None;
                }
                self.status.layers[index] = next;
                self.sync_scene_layers();
                if sound_was_here
                    && self.status.layers[index].source.kind != crate::layers::Kind::Screen
                {
                    self.status.screen_sound_layer = None;
                    self.status.screen_sound = false;
                    return self.sound();
                }
                Reply::Status(Box::new(self.reported()))
            }
            Ok(_) => {
                // Ports must reject an unusable native size; if one violates
                // that contract, never keep an incorrect capture in Status.
                self.pipeline.layer_remove(&id);
                self.drop_failed_layer(index, sound_was_here);
                Reply::Error {
                    message: "the replacement has no usable native size; original source lost"
                        .into(),
                }
            }
            Err(error) => {
                if error.restored {
                    // The restored capture has a new frame slot under the same
                    // ID. Rewire the compositor even though Status did not
                    // change, or the live stays on the cleared old slot.
                    self.pipeline.layers_changed(&self.status.layers);
                } else {
                    self.drop_failed_layer(index, sound_was_here);
                }
                Reply::Error {
                    message: error.reason,
                }
            }
        }
    }

    fn drop_failed_layer(&mut self, index: usize, sound_was_here: bool) {
        self.status.layers.remove(index);
        self.pipeline.layers_changed(&self.status.layers);
        if sound_was_here {
            self.status.screen_sound_layer = None;
            self.status.screen_sound = false;
            let _ = self.sound();
        }
    }

    fn validate_layer_id(&self, id: &str) -> Result<(), String> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("layer id must contain only letters, digits, - or _".into());
        }
        if self
            .active_scene_order()
            .iter()
            .any(|existing| existing == id)
        {
            return Err(format!("layer {id:?} already exists"));
        }
        Ok(())
    }

    fn add_layer(&mut self, mut layer: crate::layers::Layer) -> Reply {
        let size = match self.pipeline.layer_add(&layer) {
            Ok(size) if size.0 > 0 && size.1 > 0 && size.0 <= 8192 && size.1 <= 8192 => size,
            Ok(_) => {
                self.pipeline.layer_remove(&layer.id);
                return Reply::Error {
                    message: "the source has no usable native size".into(),
                };
            }
            Err(message) => return Reply::Error { message },
        };
        layer.source.width = size.0;
        layer.source.height = size.1;
        layer.transform = crate::layers::Transform::native(size);
        self.status.layers.push(layer);
        self.sync_scene_layers();
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layer_visible(&mut self, id: String, on: bool) -> Reply {
        let Some(index) = self.status.layers.iter().position(|layer| layer.id == id) else {
            if let Some(element) = self
                .status
                .scenes
                .iter_mut()
                .find(|s| s.name == self.status.active_scene)
                .and_then(|s| s.elements.iter_mut().find(|e| e.id == id))
            {
                element.visible = on;
                self.render_scene();
                return Reply::Status(Box::new(self.reported()));
            }
            return Reply::Error {
                message: format!("no layer {id:?}"),
            };
        };
        if self.status.layers[index].visible == on {
            return Reply::Status(Box::new(self.reported()));
        }
        self.status.layers[index].visible = on;
        self.pipeline.layers_changed(&self.status.layers);
        if self.status.screen_sound_layer.as_deref() == Some(&id) {
            return self.sound();
        }
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layer_remove(&mut self, id: String) -> Reply {
        let Some(index) = self.status.layers.iter().position(|layer| layer.id == id) else {
            return self.scene_element_remove(id);
        };
        if self.status.screen_sound_layer.as_deref() == Some(&id) {
            if let Err(message) = self.pipeline.screen_audio(None) {
                return Reply::Error { message };
            }
            self.status.screen_sound = false;
            self.status.screen_sound_layer = None;
            let _ = self.sound();
        }
        self.pipeline.layer_remove(&id);
        self.status.layers.remove(index);
        self.sync_scene_layers();
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layer_move(&mut self, id: String, index: usize) -> Reply {
        if !self.active_scene_order().contains(&id) {
            return Reply::Error {
                message: format!("no layer {id:?}"),
            };
        }
        let mut order = self.active_scene_order();
        if index >= order.len() {
            return Reply::Error {
                message: "layer index is out of range".into(),
            };
        }
        order.retain(|item| item != &id);
        order.insert(index, id);
        if let Some(scene) = self
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == self.status.active_scene)
        {
            scene.order = order.clone();
        }
        self.status.layers.sort_by_key(|layer| {
            order
                .iter()
                .position(|id| id == &layer.id)
                .unwrap_or(usize::MAX)
        });
        self.sync_scene_layers();
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layer_crop(&mut self, id: String, crop: Option<crate::layers::Crop>) -> Reply {
        let Some(layer) = self.status.layers.iter_mut().find(|layer| layer.id == id) else {
            return Reply::Error {
                message: format!("no layer {id:?}"),
            };
        };
        if let Some(rect) = crop {
            if let Err(message) = rect.validate((layer.source.width, layer.source.height)) {
                return Reply::Error { message };
            }
        }
        layer.crop = crop;
        self.pipeline.layers_changed(&self.status.layers);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layer_mirror(&mut self, id: String, on: bool) -> Reply {
        if let Err(message) = crate::layers::camera(&self.status.layers, Some(&id)) {
            return Reply::Error { message };
        }
        let layer = self
            .status
            .layers
            .iter_mut()
            .find(|l| l.id == id)
            .expect("camera");
        layer.mirrored = on;
        self.pipeline.layers_changed(&self.status.layers);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layer_shape(&mut self, id: String, shape: crate::scene::CameraShape) -> Reply {
        if let Err(message) = crate::layers::camera(&self.status.layers, Some(&id)) {
            return Reply::Error { message };
        }
        let layer = self
            .status
            .layers
            .iter_mut()
            .find(|layer| layer.id == id)
            .expect("validated camera");
        layer.shape = Some(shape);
        self.pipeline.layers_changed(&self.status.layers);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layer_position(
        &mut self,
        id: String,
        at: Option<crate::scene::CameraPosition>,
    ) -> Reply {
        if let Err(message) = crate::layers::camera(&self.status.layers, Some(&id)) {
            return Reply::Error { message };
        }
        let layer = self
            .status
            .layers
            .iter_mut()
            .find(|layer| layer.id == id)
            .expect("validated camera");
        let (wide, tall) = crate::scene::CAMERA_OUTPUT;
        if let Some(at) = at {
            if at.x >= wide || at.y >= tall {
                return Reply::Error {
                    message: format!("camera position must be inside the {wide}x{tall} scene"),
                };
            }
        }
        layer.transform.x = at.map_or(0, |at| {
            at.x.min(wide.saturating_sub(layer.transform.width)) as i32
        });
        layer.transform.y = at.map_or(0, |at| {
            at.y.min(tall.saturating_sub(layer.transform.height)) as i32
        });
        self.pipeline.layers_changed(&self.status.layers);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layer_transform(
        &mut self,
        id: String,
        transform: crate::layers::Transform,
    ) -> Reply {
        if let Err(message) = transform.validate() {
            return Reply::Error { message };
        }
        let Some(layer) = self.status.layers.iter_mut().find(|layer| layer.id == id) else {
            if transform.degrees != 0 {
                return Reply::Error {
                    message: "text and timer rotation is not supported".into(),
                };
            }
            if let Some(element) = self
                .status
                .scenes
                .iter_mut()
                .find(|s| s.name == self.status.active_scene)
                .and_then(|s| s.elements.iter_mut().find(|e| e.id == id))
            {
                let mut next = element.clone();
                next.x = transform.x;
                next.y = transform.y;
                next.width = transform.width;
                next.height = transform.height;
                if !next.valid() {
                    return Reply::Error {
                        message: "element must fit inside 1920x1080".into(),
                    };
                }
                if (element.width, element.height) != (next.width, next.height)
                    && next.shader.is_some()
                {
                    if let Err(message) =
                        self.pipeline.element_shader(&next, next.shader.as_deref())
                    {
                        return Reply::Error { message };
                    }
                }
                *element = next;
                self.render_scene();
                return Reply::Status(Box::new(self.reported()));
            }
            return Reply::Error {
                message: format!("no layer {id:?}"),
            };
        };
        layer.transform = transform;
        self.pipeline.layers_changed(&self.status.layers);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layer_shader(&mut self, id: String, path: Option<String>) -> Reply {
        let Some(index) = self.status.layers.iter().position(|layer| layer.id == id) else {
            let element = self
                .active_elements()
                .into_iter()
                .find(|element| element.id == id);
            let Some(element) = element else {
                return Reply::Error {
                    message: format!("no layer {id:?}"),
                };
            };
            return match self.pipeline.element_shader(&element, path.as_deref()) {
                Ok(()) => {
                    let scene = self
                        .status
                        .scenes
                        .iter_mut()
                        .find(|s| s.name == self.status.active_scene)
                        .expect("active scene");
                    scene
                        .elements
                        .iter_mut()
                        .find(|e| e.id == id)
                        .expect("element")
                        .shader = path;
                    Reply::Status(Box::new(self.reported()))
                }
                Err(message) => Reply::Error { message },
            };
        };
        match self
            .pipeline
            .layer_shader(&self.status.layers[index], path.as_deref())
        {
            Ok(()) => {
                self.status.layers[index].shader = path;
                Reply::Status(Box::new(self.reported()))
            }
            Err(message) => Reply::Error { message },
        }
    }

    pub(super) fn shader(&mut self, path: Option<String>) -> Reply {
        match self.pipeline.shader(path.as_deref()) {
            Ok(()) => {
                self.status.shader = path;
                Reply::Status(Box::new(self.reported()))
            }
            Err(message) => Reply::Error { message },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::*;
    use crate::layers::Kind;
    use crate::sources::{DisplayId, WindowId};

    fn text(id: &str, line: &str) -> Element {
        Element {
            id: id.into(),
            x: 120,
            y: 60,
            width: 640,
            height: 160,
            visible: true,
            shader: None,
            content: ElementContent::Text { text: line.into() },
        }
    }

    #[test]
    fn elements_are_ordered_persist_and_render_without_capture() {
        let fake = Wrote::default();
        let shown = fake.shown.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        assert_eq!(engine.status().scenes.len(), 1);
        let a = text("first", "Welcome");
        let b = text("second", "Hello");
        assert!(matches!(
            engine.handle(Command::SceneElementAdd { element: a.clone() }),
            Reply::Status(_)
        ));
        assert!(matches!(
            engine.handle(Command::SceneElementAdd { element: b.clone() }),
            Reply::Status(_)
        ));
        assert_eq!(engine.status().scenes[0].elements, [a.clone(), b.clone()]);
        assert_eq!(shown.lock().unwrap().last().unwrap().0, [a, b]);
        let saved =
            crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
        let mut restored = Engine::new();
        restored.restore(&saved);
        assert_eq!(restored.status().scenes[0].elements.len(), 2);
        assert!(matches!(
            engine.handle(Command::SceneElementRemove { id: "first".into() }),
            Reply::Status(_)
        ));
        assert_eq!(engine.status().scenes[0].elements[0].id, "second");
    }

    #[test]
    fn captures_and_generated_layers_share_one_order_and_identity() {
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        engine.handle(Command::LayerScreen {
            id: "desk".into(),
            display: 1,
        });
        engine.handle(Command::SceneElementAdd {
            element: text("label", "Hi"),
        });
        engine.handle(Command::LayerCamera {
            id: "face".into(),
            device: "MacBook Pro Camera".into(),
        });
        engine.handle(Command::Mirror { on: true });
        engine.handle(Command::LayerMirror {
            id: "face".into(),
            on: false,
        });
        assert!(
            !engine
                .status()
                .layers
                .iter()
                .find(|layer| layer.id == "face")
                .unwrap()
                .mirrored
        );
        engine.handle(Command::LayerMirror {
            id: "face".into(),
            on: true,
        });
        assert!(
            engine
                .status()
                .layers
                .iter()
                .find(|layer| layer.id == "face")
                .unwrap()
                .mirrored
        );
        assert_eq!(
            engine.status().scenes[0].ordered_ids(),
            ["desk", "label", "face"]
        );
        assert!(matches!(
            engine.handle(Command::SceneElementAdd {
                element: text("desk", "duplicate")
            }),
            Reply::Error { .. }
        ));
        engine.handle(Command::LayerMove {
            id: "label".into(),
            index: 2,
        });
        assert_eq!(
            engine.status().scenes[0].ordered_ids(),
            ["desk", "face", "label"]
        );
        engine.handle(Command::LayerVisible {
            id: "label".into(),
            on: false,
        });
        assert!(!engine.status().scenes[0].elements[0].visible);
        let saved =
            crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
        let mut restored =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        restored.restore(&saved);
        assert_eq!(
            restored.status().scenes[0].ordered_ids(),
            ["desk", "face", "label"]
        );
        assert!(
            restored
                .status()
                .layers
                .iter()
                .find(|layer| layer.id == "face")
                .unwrap()
                .mirrored
        );
        restored.handle(Command::LayerRemove { id: "label".into() });
        assert_eq!(restored.status().scenes[0].ordered_ids(), ["desk", "face"]);
    }

    #[test]
    fn generated_shader_is_atomic_persistent_and_validated_on_switch() {
        let mut engine = Engine::new().with_pipeline(Box::new(Wrote::default()));
        engine.handle(Command::SceneElementAdd {
            element: text("title", "Before"),
        });
        assert!(matches!(
            engine.handle(Command::LayerShader {
                id: "title".into(),
                path: Some("good.wgsl".into())
            }),
            Reply::Status(_)
        ));
        assert_eq!(
            engine.status().scenes[0].elements[0].shader.as_deref(),
            Some("good.wgsl")
        );
        assert!(matches!(
            engine.handle(Command::LayerShader {
                id: "title".into(),
                path: Some("bad.wgsl".into())
            }),
            Reply::Error { .. }
        ));
        assert_eq!(
            engine.status().scenes[0].elements[0].shader.as_deref(),
            Some("good.wgsl")
        );
        let mut invalid = text("invalid", "No");
        invalid.shader = Some("bad.wgsl".into());
        assert!(matches!(
            engine.handle(Command::SceneElementAdd { element: invalid }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status().scenes[0].elements.len(), 1);
        engine.handle(Command::SceneDuplicate {
            name: "next".into(),
        });
        assert!(matches!(
            engine.handle(Command::LayerShader {
                id: "title".into(),
                path: None
            }),
            Reply::Status(_)
        ));
        engine.handle(Command::SceneSwitch {
            name: "default".into(),
        });
        assert_eq!(
            engine
                .status()
                .scenes
                .iter()
                .find(|s| s.name == "default")
                .unwrap()
                .elements[0]
                .shader
                .as_deref(),
            Some("good.wgsl")
        );
        let saved =
            crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
        assert_eq!(
            saved.scenes[0].elements[0].shader.as_deref(),
            Some("good.wgsl")
        );

        let mut restored = Engine::new().with_pipeline(Box::new(Wrote {
            refuse: Some("shader:invalid".into()),
            ..Default::default()
        }));
        restored.restore(&saved);
        assert_eq!(
            restored.status().scenes[0].elements[0].shader.as_deref(),
            Some("good.wgsl")
        );
        restored.handle(Command::LayerShader {
            id: "title".into(),
            path: None,
        });
        restored.handle(Command::SceneDuplicate { name: "bad".into() });
        restored
            .status
            .scenes
            .iter_mut()
            .find(|s| s.name == "bad")
            .unwrap()
            .elements[0]
            .shader = Some("bad.wgsl".into());
        restored.handle(Command::SceneSwitch {
            name: "default".into(),
        });
        let previous = restored.status.active_scene.clone();
        assert!(matches!(
            restored.handle(Command::SceneSwitch { name: "bad".into() }),
            Reply::Error { .. }
        ));
        assert_eq!(restored.status.active_scene, previous);
    }

    #[test]
    fn timers_are_transient_per_scene_and_never_switch_automatically() {
        let fake = Wrote::default();
        let shown = fake.shown.clone();
        let mut engine = Engine::new().with_pipeline(Box::new(fake));
        let timer = Element {
            id: "clock".into(),
            x: 600,
            y: 400,
            width: 700,
            height: 180,
            visible: true,
            shader: None,
            content: ElementContent::Timer { seconds: 0 },
        };
        engine.handle(Command::SceneElementAdd {
            element: timer.clone(),
        });
        engine.handle(Command::SceneTimerStart { id: "clock".into() });
        assert_eq!(shown.lock().unwrap().last().unwrap().1[0].1, Duration::ZERO);
        assert_eq!(engine.status().active_scene, "default");
        engine.handle(Command::SceneElementSet {
            element: Element {
                content: ElementContent::Timer { seconds: 90 },
                ..timer.clone()
            },
        });
        engine.handle(Command::SceneTimerStart { id: "clock".into() });
        assert!(shown.lock().unwrap().last().unwrap().1[0].1 > Duration::from_secs(89));
        engine.handle(Command::SceneTimerStop { id: "clock".into() });
        assert!(engine.counting.is_empty());
        engine.handle(Command::SceneElementSet {
            element: timer.clone(),
        });
        engine.handle(Command::SceneTimerStart { id: "clock".into() });
        let saved = engine.remembered();
        assert!(crate::remembered::write(&saved)
            .unwrap()
            .contains("\"seconds\": 0"));
        assert!(!crate::remembered::write(&saved)
            .unwrap()
            .contains("deadline"));
        engine.handle(Command::SceneDuplicate {
            name: "other".into(),
        });
        assert!(engine.counting.is_empty());
        engine.handle(Command::SceneSwitch {
            name: "default".into(),
        });
        assert!(engine.counting.is_empty());
        let mut restarted = Engine::new();
        restarted.restore(&saved);
        assert_eq!(restarted.status().scenes[0].elements, [timer]);
        assert!(restarted.counting.is_empty());
    }

    #[test]
    fn layer_and_scene_shaders_belong_to_the_scene_they_were_set_in() {
        let mut engine = Engine::new().with_pipeline(Box::new(Wrote::default()));
        engine.status.layers = vec![layer("face", Kind::Camera, "cam", 0)];
        let set = engine.handle(Command::LayerShader {
            id: "face".into(),
            path: Some("layer.wgsl".into()),
        });
        let Reply::Status(status) = set else {
            panic!("expected status")
        };
        assert_eq!(status.layers[0].shader.as_deref(), Some("layer.wgsl"));
        engine.handle(Command::Shader {
            path: Some("global.wgsl".into()),
        });
        engine.handle(Command::SceneDuplicate {
            name: "second".into(),
        });
        engine.handle(Command::LayerShader {
            id: "face".into(),
            path: None,
        });
        engine.handle(Command::Shader { path: None });
        engine.handle(Command::SceneSwitch {
            name: "default".into(),
        });
        let status = engine.status();
        assert_eq!(status.shader.as_deref(), Some("global.wgsl"));
        assert_eq!(status.layers[0].shader.as_deref(), Some("layer.wgsl"));
        let saved =
            crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
        assert_eq!(saved.scenes[0].shader.as_deref(), Some("global.wgsl"));
        assert_eq!(
            saved.scenes[0].layers[0].shader.as_deref(),
            Some("layer.wgsl")
        );
        let second = saved
            .scenes
            .iter()
            .find(|scene| scene.name == "second")
            .unwrap();
        assert_eq!(second.shader, None);
        assert_eq!(second.layers[0].shader, None);
    }

    #[test]
    fn camera_shape_and_position_belong_to_one_named_layer() {
        let (mut engine, _, _) = machine_with_music();
        assert!(crate::layers::camera(&engine.status().layers, None).is_err());
        engine.handle(Command::LayerCamera {
            id: "left".into(),
            device: "HP".into(),
        });
        assert_eq!(
            crate::layers::camera(&engine.status().layers, None)
                .unwrap()
                .id,
            "left"
        );
        assert!(matches!(
            engine.handle(Command::CameraShape {
                shape: crate::scene::CameraShape::Circle
            }),
            Reply::Status(_)
        ));
        assert_eq!(
            engine.status().layers[0].shape,
            Some(crate::scene::CameraShape::Circle)
        );
        engine.handle(Command::LayerCamera {
            id: "right".into(),
            device: "MacBook".into(),
        });
        assert!(crate::layers::camera(&engine.status().layers, None).is_err());
        assert!(matches!(
            engine.handle(Command::CameraShape {
                shape: crate::scene::CameraShape::Rectangle
            }),
            Reply::Error { .. }
        ));
        assert!(matches!(
            engine.handle(Command::CameraPosition { at: None }),
            Reply::Error { .. }
        ));
        let before = engine.status().layers[1].transform;
        assert!(matches!(
            engine.handle(Command::LayerShape {
                id: "missing".into(),
                shape: crate::scene::CameraShape::Circle
            }),
            Reply::Error { .. }
        ));
        assert!(matches!(
            engine.handle(Command::LayerPosition {
                id: "missing".into(),
                at: None
            }),
            Reply::Error { .. }
        ));
        assert!(matches!(
            engine.handle(Command::LayerShape {
                id: "right".into(),
                shape: crate::scene::CameraShape::Circle
            }),
            Reply::Status(_)
        ));
        assert_eq!(
            engine.status().layers[0].shape,
            Some(crate::scene::CameraShape::Circle)
        );
        assert_eq!(
            engine.status().layers[1].shape,
            Some(crate::scene::CameraShape::Circle)
        );
        engine.handle(Command::LayerPosition {
            id: "right".into(),
            at: Some(crate::scene::CameraPosition { x: 1900, y: 1000 }),
        });
        assert_eq!(
            engine.status().layers[1].transform.x,
            (1920 - before.width) as i32
        );
        assert_eq!(
            engine.status().layers[1].transform.y,
            (1080 - before.height) as i32
        );
        assert_eq!(engine.status().layers[1].transform.width, before.width);
        assert_eq!(engine.status().layers[0].transform.x, 0);
        engine.handle(Command::LayerPosition {
            id: "right".into(),
            at: None,
        });
        assert_eq!(engine.status().layers[1].transform, before);
        engine.handle(Command::LayerWindow {
            id: "app".into(),
            query: "tmux".into(),
        });
        assert!(matches!(
            engine.handle(Command::LayerShape {
                id: "app".into(),
                shape: crate::scene::CameraShape::Circle
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status().layers[2].shape, None);
    }

    #[test]
    fn display_layers_have_no_reserved_id_or_order() {
        let (mut engine, _, _) = machine_with_music();
        assert!(matches!(
            engine.handle(Command::LayerScreen {
                id: "left".into(),
                display: 99
            }),
            Reply::Error { .. }
        ));
        assert!(engine.status().layers.is_empty());
        assert!(matches!(
            engine.handle(Command::LayerScreen {
                id: "left".into(),
                display: 3
            }),
            Reply::Status(_)
        ));
        assert!(matches!(
            engine.handle(Command::LayerScreen {
                id: "right".into(),
                display: 1
            }),
            Reply::Status(_)
        ));
        assert_eq!(
            engine.status().layers[0].source.kind,
            crate::layers::Kind::Screen
        );
        assert_eq!(engine.status().layers[0].source.handle, "3");
        assert_eq!(engine.status().layers[0].source.name, "VG2791R");
        engine.handle(Command::LayerMove {
            id: "right".into(),
            index: 0,
        });
        assert_eq!(engine.status().layers[0].id, "right");
        engine.handle(Command::LayerRemove { id: "left".into() });
        assert_eq!(engine.status().layers[0].id, "right");
    }

    #[test]
    fn layers_are_ordered_ephemeral_and_failed_capture_does_not_appear() {
        use crate::layers::{Kind, Transform};
        let (mut engine, _, _) = machine_with_music();
        engine.handle(Command::LayerCamera {
            id: "face".into(),
            device: "HP 430".into(),
        });
        engine.handle(Command::LayerWindow {
            id: "code".into(),
            query: "tmux".into(),
        });
        assert_eq!(engine.status().layers.len(), 2);
        assert_eq!(engine.status().layers[1].source.kind, Kind::Window);
        assert_eq!(engine.status().layers[1].source.handle, "10");
        assert_eq!(
            (
                engine.status().layers[0].source.width,
                engine.status().layers[0].source.height
            ),
            (1280, 720)
        );
        assert_eq!(
            engine.status().layers[1].transform,
            Transform::native((853, 479))
        );
        let transform = Transform {
            x: 100,
            y: 200,
            width: 800,
            height: 450,
            degrees: 90,
        };
        engine.handle(Command::LayerTransform {
            id: "face".into(),
            transform,
        });
        engine.handle(Command::LayerMove {
            id: "face".into(),
            index: 1,
        });
        assert_eq!(engine.status().layers[1].transform, transform);
        assert_eq!(engine.status().layers[1].id, "face");
        let crop = crate::layers::Crop {
            x: 10,
            y: 20,
            width: 600,
            height: 400,
        };
        engine.handle(Command::LayerCrop {
            id: "face".into(),
            crop: Some(crop),
        });
        assert_eq!(engine.status().layers[1].crop, Some(crop));
        assert!(matches!(
            engine.handle(Command::LayerCrop {
                id: "face".into(),
                crop: Some(crate::layers::Crop { x: 1000, ..crop })
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status().layers[1].crop, Some(crop));
        engine.handle(Command::LayerCrop {
            id: "face".into(),
            crop: None,
        });
        assert_eq!(engine.status().layers[1].crop, None);
        assert!(matches!(
            engine.handle(Command::LayerCamera {
                id: "face".into(),
                device: "HP".into()
            }),
            Reply::Error { .. }
        ));
        assert!(matches!(
            engine.handle(Command::LayerMove {
                id: "face".into(),
                index: 5
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status().layers.len(), 2);
        assert!(crate::remembered::write(&engine.remembered())
            .unwrap()
            .contains("face"));
        engine.handle(Command::LayerRemove { id: "code".into() });
        assert_eq!(engine.status().layers.len(), 1);
        engine.handle(Command::HideEverything);
        assert!(engine.status().layers.is_empty());
    }

    fn unique_name(status: &Status, kinds: &[crate::layers::Kind]) -> Option<String> {
        let mut found = status
            .layers
            .iter()
            .filter(|l| kinds.contains(&l.source.kind));
        let first = found.next()?;
        found.next().is_none().then(|| first.source.name.clone())
    }

    fn screen_name(status: &Status) -> Option<String> {
        unique_name(
            status,
            &[crate::layers::Kind::Screen, crate::layers::Kind::Window],
        )
    }

    fn camera_name(status: &Status) -> Option<String> {
        unique_name(status, &[crate::layers::Kind::Camera])
    }

    // A screen is chosen by its display id, never by its position: the
    // capturer and the display list order the same hardware differently, and
    // a real machine shows it (the built-in is display 1, the monitor is 3).
    #[test]
    fn a_screen_is_chosen_by_its_display_id_and_answers_with_its_name() {
        let mut engine = machine();
        let Reply::Status(status) = engine.handle(Command::Screen { display: 3 }) else {
            panic!("choosing a screen answers with the new status")
        };
        assert_eq!(screen_name(&status), Some("VG2791R".into()));
    }

    // "no display 9" alone sends a person hunting. Saying what there is
    // instead turns a dead end into the answer.
    #[test]
    fn asking_for_a_display_that_is_not_there_says_what_is() {
        let mut engine = machine();
        let Reply::Error { message } = engine.handle(Command::Screen { display: 9 }) else {
            panic!("an absent display is an error")
        };
        assert!(message.contains("no display 9"), "{message}");
        assert!(
            message.contains("Built-in Retina Display and VG2791R"),
            "it should say what there is: {message}"
        );
        assert!(engine.status().layers.is_empty(), "and change nothing");
    }

    #[test]
    fn a_window_is_chosen_by_part_of_its_title() {
        let mut engine = machine();
        let Reply::Status(status) = engine.handle(Command::Window {
            query: "tmux".into(),
        }) else {
            panic!("choosing a window answers with the new status")
        };
        assert_eq!(screen_name(&status), Some("Ghostty — tmux a".into()));
    }

    // The CLI has always taken part of a name, and a browser window is titled
    // after the page it is showing, so the application has to match too.
    #[test]
    fn a_window_is_found_by_the_application_that_owns_it() {
        let mut engine = machine();
        engine.handle(Command::Window {
            query: "brave".into(),
        });
        assert_eq!(
            screen_name(engine.status()),
            Some("Brave Browser — remux".into())
        );
    }

    #[test]
    fn asking_for_a_window_that_is_not_there_changes_nothing() {
        let mut engine = machine();
        engine.handle(Command::Screen { display: 3 });
        let Reply::Error { message } = engine.handle(Command::Window {
            query: "photoshop".into(),
        }) else {
            panic!("an absent window is an error")
        };
        assert!(message.contains("photoshop"), "{message}");
        assert_eq!(
            screen_name(engine.status()),
            Some("VG2791R".into()),
            "a miss must not drop what was already chosen"
        );
    }

    // A window replaces a screen and a screen replaces a window: there is one
    // picture, and this is what is behind it.
    #[test]
    fn choosing_one_source_replaces_the_other() {
        let mut engine = machine();
        engine.handle(Command::Screen { display: 1 });
        assert_eq!(
            screen_name(engine.status()),
            Some("Built-in Retina Display".into())
        );
        engine.handle(Command::Window {
            query: "notes".into(),
        });
        assert_eq!(
            screen_name(engine.status()),
            Some("TextEdit — notes".into())
        );
        engine.handle(Command::Screen { display: 3 });
        assert_eq!(screen_name(engine.status()), Some("VG2791R".into()));
    }

    #[test]
    fn a_refused_grant_is_a_sentence_when_choosing_too() {
        let mut engine = Engine::with_sources(Box::new(Refused));
        assert!(matches!(
            engine.handle(Command::Screen { display: 1 }),
            Reply::Error { .. }
        ));
        assert!(matches!(
            engine.handle(Command::Window { query: "x".into() }),
            Reply::Error { .. }
        ));
    }

    #[test]
    fn a_camera_is_found_by_part_of_its_name_and_by_its_id() {
        let mut engine = machine();
        engine.handle(Command::Camera {
            device: Some("webcam".into()),
        });
        assert_eq!(
            camera_name(engine.status()),
            Some("HP 430/435 FHD Webcam".into())
        );

        engine.handle(Command::Camera {
            device: Some("6C707041".into()),
        });
        assert_eq!(
            camera_name(engine.status()),
            Some("MacBook Pro Camera".into()),
            "an id is the handle a saved preference holds"
        );
    }

    // Turning the camera off is a different thing from never having chosen
    // one, and both are allowed: the camera is a slot in the picture.
    #[test]
    fn a_camera_can_be_turned_off_again() {
        let mut engine = machine();
        engine.handle(Command::Camera {
            device: Some("webcam".into()),
        });
        assert!(camera_name(engine.status()).is_some());
        engine.handle(Command::Camera { device: None });
        assert!(camera_name(engine.status()).is_none());
    }

    #[test]
    fn camera_position_changes_the_pipeline_mid_live_and_can_be_restored() {
        use crate::scene::CameraPosition;
        let pipeline = Wrote::default();
        let mut engine = Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_destination(Some(DESTINATION.into()));
        engine.handle(Command::Window {
            query: "remux".into(),
        });
        engine.handle(Command::Camera {
            device: Some("HP".into()),
        });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        let at = CameraPosition { x: 300, y: 200 };
        let Reply::Status(after) = engine.handle(Command::CameraPosition { at: Some(at) }) else {
            panic!("status")
        };
        let face = after
            .layers
            .iter()
            .find(|l| l.source.kind == crate::layers::Kind::Camera)
            .unwrap();
        assert_eq!((face.transform.x, face.transform.y), (300, 200));
        assert!(after.on_air, "moving the camera must not stop the stream");
        assert_eq!(
            engine
                .remembered()
                .layers
                .iter()
                .find(|l| l.id == face.id)
                .unwrap()
                .transform,
            face.transform
        );
        for bad in [
            CameraPosition { x: 1920, y: 0 },
            CameraPosition { x: 0, y: 1080 },
        ] {
            assert!(matches!(
                engine.handle(Command::CameraPosition { at: Some(bad) }),
                Reply::Error { .. }
            ));
            let face = engine
                .status()
                .layers
                .iter()
                .find(|l| l.source.kind == crate::layers::Kind::Camera)
                .unwrap();
            assert_eq!(
                (face.transform.x, face.transform.y),
                (300, 200),
                "invalid wire command must not change the picture"
            );
        }
        engine.handle(Command::CameraPosition { at: None });
        let face = engine
            .status()
            .layers
            .iter()
            .find(|l| l.source.kind == crate::layers::Kind::Camera)
            .unwrap();
        assert_eq!((face.transform.x, face.transform.y), (0, 0));
        assert!(engine.status().on_air);
    }

    #[test]
    fn changing_camera_shape_on_air_keeps_position_and_publication() {
        use crate::scene::{CameraPosition, CameraShape};
        let published: Published = Default::default();
        let pipeline = Wrote {
            published: published.clone(),
            ..Default::default()
        };
        let mut engine = Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_destination(Some(DESTINATION.into()));
        engine.handle(Command::Window {
            query: "remux".into(),
        });
        engine.handle(Command::Camera {
            device: Some("HP".into()),
        });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        let at = CameraPosition { x: 300, y: 200 };
        engine.handle(Command::CameraPosition { at: Some(at) });
        let Reply::Status(rectangle) = engine.handle(Command::CameraShape {
            shape: CameraShape::Rectangle,
        }) else {
            panic!("status")
        };
        let face = rectangle
            .layers
            .iter()
            .find(|l| l.source.kind == crate::layers::Kind::Camera)
            .unwrap();
        assert_eq!(face.shape, Some(CameraShape::Rectangle));
        assert_eq!((face.transform.x, face.transform.y), (300, 200));
        assert!(rectangle.on_air);
        assert_eq!(
            engine
                .remembered()
                .layers
                .iter()
                .find(|l| l.id == face.id)
                .unwrap()
                .shape,
            Some(CameraShape::Rectangle)
        );
        let Reply::Status(circle) = engine.handle(Command::CameraShape {
            shape: CameraShape::Circle,
        }) else {
            panic!("status")
        };
        let face = circle
            .layers
            .iter()
            .find(|l| l.source.kind == crate::layers::Kind::Camera)
            .unwrap();
        assert_eq!(face.shape, Some(CameraShape::Circle));
        assert_eq!((face.transform.x, face.transform.y), (300, 200));
        assert!(circle.on_air);
        assert_eq!(
            published.lock().unwrap().len(),
            1,
            "shape changes do not restart publication"
        );
    }

    #[test]
    fn a_shader_changes_the_scene_without_restarting_live_and_refusal_keeps_the_last_one() {
        let (mut engine, published) = publishing_engine(None);
        engine.handle(Command::Window {
            query: "remux".into(),
        });
        assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
        let path = "/tmp/invert.wgsl".to_string();
        let Reply::Status(status) = engine.handle(Command::Shader {
            path: Some(path.clone()),
        }) else {
            panic!("shader selection answers with a status");
        };
        assert_eq!(status.shader.as_deref(), Some(path.as_str()));
        assert!(status.on_air);
        assert_eq!(published.lock().unwrap().len(), 1);
        let Reply::Status(status) = engine.handle(Command::Shader { path: None }) else {
            panic!("shader off answers with a status");
        };
        assert_eq!(status.shader, None);
        assert!(status.on_air);
        assert_eq!(published.lock().unwrap().len(), 1);
        engine.handle(Command::Shader {
            path: Some("/tmp/unsafe.wgsl".into()),
        });
        engine.handle(Command::HideEverything);
        assert_eq!(
            engine.status().shader,
            None,
            "panic removes even a shader that obscures the emergency card"
        );

        let (mut refusing, _) = publishing_engine(Some("compiler refused".into()));
        let Reply::Error { message } = refusing.handle(Command::Shader { path: Some(path) }) else {
            panic!("a broken shader must not be accepted");
        };
        assert_eq!(message, "compiler refused");
        assert_eq!(refusing.status().shader, None);
    }

    #[test]
    fn the_self_view_flips_and_the_status_can_be_read_back() {
        let (mut engine, _) = publishing_engine(None);
        assert!(!engine.status().mirrored, "a camera starts unflipped");
        let Reply::Status(after) = engine.handle(Command::Mirror { on: true }) else {
            panic!("mirror answers with a status, so a panel redraws from one line")
        };
        assert!(after.mirrored);
        engine.handle(Command::Mirror { on: false });
        assert!(!engine.status().mirrored);
    }

    #[test]
    fn flipping_reaches_the_picture_and_not_only_the_status() {
        let mirrored: std::sync::Arc<std::sync::Mutex<bool>> = Default::default();
        let pipeline = Wrote {
            told: Default::default(),
            cameras: Default::default(),
            mics: Default::default(),
            played: Default::default(),
            levels: Default::default(),
            shown: Default::default(),
            counting: Default::default(),
            published: Default::default(),
            recorded: Default::default(),
            mirrored: mirrored.clone(),
            gated: Default::default(),
            heard_here: Default::default(),
            speaker_calls: Default::default(),
            previewed: Default::default(),
            ran_out: Default::default(),
            refuse: None,
            scene_events: Default::default(),
        };
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
        engine.handle(Command::Mirror { on: true });
        assert!(
            *mirrored.lock().expect("mirrored"),
            "the compositor has to be told, not just the status"
        );
    }

    #[test]
    fn a_panel_can_read_back_scene_elements() {
        let (mut engine, _) = publishing_engine(None);
        let element = Element {
            id: "label".into(),
            x: 20,
            y: 30,
            width: 300,
            height: 90,
            visible: true,
            shader: None,
            content: ElementContent::Text {
                text: "Chegando já".into(),
            },
        };
        engine.handle(Command::SceneElementAdd {
            element: element.clone(),
        });
        assert_eq!(engine.status().scenes[0].elements, [element]);
    }

    #[test]
    fn a_panel_can_ask_for_a_picture_of_what_is_going_out() {
        let (mut engine, _) = publishing_engine(None);
        let Reply::Shot {
            jpeg,
            width,
            height,
        } = engine.handle(Command::Shot { of: Framed::Scene })
        else {
            panic!("a shot answers with a shot")
        };
        assert_eq!((width, height), (480, 270));
        assert_eq!(
            jpeg, "/9j/",
            "the bytes come back as base64, not as a number"
        );
    }

    #[test]
    fn an_engine_with_no_picture_says_so_rather_than_sending_an_empty_one() {
        let mut engine = engine();
        assert!(
            matches!(
                engine.handle(Command::Shot { of: Framed::Scene }),
                Reply::Error { .. }
            ),
            "nothing is plugged in, so there is nothing to show"
        );
    }

    // A restart is the case. The count that said somebody was watching lived
    // in the engine's memory, so an engine restarted under a live panel never
    // published a preview frame again and the panel showed the last one it had
    // for ever: screen, camera and scene, all still, and a card that never
    // appeared. Watching is a lease: a face renews it while it draws, and an
    // engine that has not heard from one in a few seconds stops publishing.
    #[test]
    fn watching_is_a_lease_a_face_keeps_renewing() {
        let previewed: std::sync::Arc<std::sync::Mutex<Option<bool>>> = Default::default();
        let pipeline = Wrote {
            previewed: previewed.clone(),
            ..Default::default()
        };
        let mut engine = machine().with_pipeline(Box::new(pipeline));
        let told = || *previewed.lock().expect("previewed");

        engine.handle(Command::Watching { on: true });
        assert_eq!(told(), Some(true), "a face drawing turns the preview on");
        for _ in 0..WATCH_LEASE_TICKS - 1 {
            engine.tick();
        }
        assert_eq!(told(), Some(true), "still within the lease");
        engine.handle(Command::Watching { on: true });
        for _ in 0..WATCH_LEASE_TICKS - 1 {
            engine.tick();
        }
        assert_eq!(told(), Some(true), "renewed in time, so still on");
        engine.tick();
        assert_eq!(told(), Some(false), "nobody renewed: the preview stops");
        engine.handle(Command::Watching { on: true });
        assert_eq!(
            told(),
            Some(true),
            "and a face coming back turns it on again"
        );
    }

    fn machine_with_pipeline() -> (Engine, std::sync::Arc<std::sync::Mutex<Vec<Behind>>>) {
        let told = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let pipeline = Wrote {
            told: told.clone(),
            scene_events: Default::default(),
            cameras: Default::default(),
            mics: Default::default(),
            played: Default::default(),
            levels: Default::default(),
            shown: Default::default(),
            counting: Default::default(),
            published: Default::default(),
            recorded: Default::default(),
            mirrored: Default::default(),
            gated: Default::default(),
            heard_here: Default::default(),
            speaker_calls: Default::default(),
            previewed: Default::default(),
            ran_out: Default::default(),
            refuse: None,
        };
        (
            Engine::with_sources(Box::new(ThisMachine))
                .with_pipeline(Box::new(pipeline))
                .with_destination(Some(DESTINATION.into())),
            told,
        )
    }

    #[test]
    fn choosing_a_screen_points_the_capture_at_that_display() {
        let (mut engine, told) = machine_with_pipeline();
        engine.handle(Command::Screen { display: 3 });
        assert_eq!(
            *told.lock().expect("told"),
            vec![Behind::Screen(DisplayId(3))]
        );
    }

    #[test]
    fn choosing_a_window_points_the_capture_at_that_window() {
        let (mut engine, told) = machine_with_pipeline();
        engine.handle(Command::Window {
            query: "tmux".into(),
        });
        assert_eq!(
            *told.lock().expect("told"),
            vec![Behind::Window(WindowId(10))]
        );
    }

    // Every change goes through, on air or not. Swapping the monitor mid-live
    // is a thing people do and it is where a pipeline is most likely to break,
    // so it must not be a path that only runs off air.
    #[test]
    fn a_swap_while_on_air_still_reaches_the_capture() {
        let (mut engine, told) = machine_with_pipeline();
        engine.handle(Command::Screen { display: 1 });
        engine.handle(Command::GoLive);
        engine.handle(Command::Screen { display: 3 });
        engine.handle(Command::Window {
            query: "notes".into(),
        });
        assert_eq!(
            told.lock()
                .expect("told")
                .iter()
                .copied()
                .filter(|source| *source != Behind::Nothing)
                .collect::<Vec<_>>(),
            vec![
                Behind::Screen(DisplayId(1)),
                Behind::Screen(DisplayId(3)),
                Behind::Window(WindowId(12)),
            ]
        );
        assert!(engine.status().on_air, "and it never left the air");
    }

    // The switch that takes the screen off the live. Nothing is a real state
    // with a picture of its own, not an absence: the frames have to keep
    // flowing or a viewer cannot tell a deliberate blank from a dead stream.
    #[test]
    fn sharing_nothing_points_the_capture_at_nothing() {
        let (mut engine, told) = machine_with_pipeline();
        engine.handle(Command::Screen { display: 3 });
        let Reply::Status(status) = engine.handle(Command::Share { on: false }) else {
            panic!("the share switch answers a status")
        };
        assert!(status.layers.is_empty());
        assert_eq!(
            told.lock().expect("told").last(),
            Some(&Behind::Nothing),
            "the capture is told, not merely forgotten"
        );
    }

    // The status must never claim a capture that did not start. This is the
    // whole reason `behind` asks the pipeline before it writes the status.
    #[test]
    fn a_capture_that_refuses_leaves_the_status_alone() {
        let refusing = Wrote {
            told: Default::default(),
            cameras: Default::default(),
            mics: Default::default(),
            played: Default::default(),
            levels: Default::default(),
            shown: Default::default(),
            counting: Default::default(),
            published: Default::default(),
            recorded: Default::default(),
            mirrored: Default::default(),
            gated: Default::default(),
            heard_here: Default::default(),
            speaker_calls: Default::default(),
            previewed: Default::default(),
            ran_out: Default::default(),
            refuse: Some("macOS refused: screen recording".into()),
            scene_events: Default::default(),
        };
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(refusing));
        let Reply::Error { message } = engine.handle(Command::Screen { display: 3 }) else {
            panic!("a refused capture is an error")
        };
        assert!(message.contains("refused"), "{message}");
        let Reply::Error { message } = engine.handle(Command::LayerWindow {
            id: "editor".into(),
            query: "tmux".into(),
        }) else {
            panic!("a refused overlay is an error")
        };
        assert!(message.contains("refused"));
        assert!(
            engine.status().layers.is_empty(),
            "a failed capture is not in Status"
        );
        assert!(
            engine.status().layers.is_empty(),
            "the status must not claim a capture that never started"
        );
    }

    #[test]
    fn what_is_flowing_is_measured_not_assumed() {
        let (mut engine, _) = machine_with_pipeline();
        engine.handle(Command::Screen { display: 1 });
        assert_eq!(
            engine.flowing(),
            Flowing {
                captured: 7,
                frames: 42,
                width: 1920,
                height: 1080,
                held: None
            }
        );
    }

    #[test]
    fn hiding_keeps_capture_preview_layout_and_id_and_pauses_screen_sound() {
        let pipeline = Wrote::default();
        let captured = pipeline.told.clone();
        let levels = pipeline.levels.clone();
        let shown = pipeline.shown.clone();
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
        engine.handle(Command::LayerScreen {
            id: "desk".into(),
            display: 1,
        });
        let original = engine.status().layers[0].clone();
        engine.handle(Command::LayerScreenSound {
            id: "desk".into(),
            on: true,
        });
        assert!(levels.lock().unwrap().unwrap().5);

        let Reply::Status(hidden) = engine.handle(Command::LayerVisible {
            id: "desk".into(),
            on: false,
        }) else {
            panic!("hide")
        };
        assert_eq!(hidden.layers[0].id, original.id);
        assert_eq!(hidden.layers[0].source, original.source);
        assert_eq!(hidden.layers[0].transform, original.transform);
        assert!(!hidden.layers[0].visible);
        assert_eq!(
            hidden.layer_flowing["desk"].captured, 3,
            "capture continues while hidden"
        );
        assert!(matches!(
            engine.handle(Command::LayerShot { id: "desk".into() }),
            Reply::Shot { .. }
        ));
        assert!(hidden.screen_sound && hidden.screen_sound_layer.as_deref() == Some("desk"));
        assert!(
            !levels.lock().unwrap().unwrap().5,
            "hidden display audio must leave the mix"
        );
        assert_eq!(engine.status().active_scene, "default");
        assert_eq!(
            captured.lock().unwrap().len(),
            1,
            "hide must not stop or reopen capture"
        );
        let saved =
            crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
        assert!(!saved.layers[0].visible);
        let mut restored =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        restored.restore(&saved);
        assert_eq!(restored.status().layers[0].id, "desk");
        assert!(!restored.status().layers[0].visible);

        engine.handle(Command::LayerVisible {
            id: "desk".into(),
            on: true,
        });
        assert!(engine.status().layers[0].visible);
        assert!(
            levels.lock().unwrap().unwrap().5,
            "show resumes requested audio"
        );
        assert_eq!(engine.status().active_scene, "default");
        assert_eq!(captured.lock().unwrap().len(), 1);
        let before = elements_shown(&shown).len();
        engine.handle(Command::LayerVisible {
            id: "desk".into(),
            on: true,
        });
        assert_eq!(
            elements_shown(&shown).len(),
            before,
            "showing an already visible layer does nothing"
        );
    }

    #[test]
    fn ordinary_visual_verbs_reuse_one_layer_without_a_reserved_id() {
        let pipeline = Wrote::default();
        let told = pipeline.told.clone();
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
        let Reply::Status(first) = engine.handle(Command::Window {
            query: "tmux".into(),
        }) else {
            panic!("window")
        };
        let id = first.layers[0].id.clone();
        assert!(id.starts_with("source-") && !id.contains("legacy"));
        engine.handle(Command::LayerCamera {
            id: "face".into(),
            device: "HP".into(),
        });
        let transform = crate::layers::Transform {
            x: 80,
            y: 50,
            width: 600,
            height: 300,
            degrees: 15,
        };
        engine.handle(Command::LayerTransform {
            id: id.clone(),
            transform,
        });
        let crop = crate::layers::Crop {
            x: 20,
            y: 30,
            width: 400,
            height: 200,
        };
        engine.handle(Command::LayerCrop {
            id: id.clone(),
            crop: Some(crop),
        });
        let Reply::Status(switched) = engine.handle(Command::Screen { display: 3 }) else {
            panic!("screen")
        };
        assert_eq!(
            switched
                .layers
                .iter()
                .map(|layer| layer.id.as_str())
                .collect::<Vec<_>>(),
            vec![id.as_str(), "face"]
        );
        assert_eq!(switched.layers[0].transform, transform);
        assert_eq!(switched.layers[0].crop, Some(crop));
        assert_eq!(switched.layers[0].source.name, "VG2791R");
        assert_eq!(switched.layers[0].source.kind, crate::layers::Kind::Screen);
        // The fake records the stop before the new source opens, not two captures
        // with different generated IDs overlapping even briefly.
        assert_eq!(
            *told.lock().unwrap(),
            vec![
                Behind::Window(WindowId(10)),
                Behind::Nothing,
                Behind::Screen(DisplayId(3))
            ]
        );
        let Reply::Status(same) = engine.handle(Command::Screen { display: 3 }) else {
            panic!("same source")
        };
        assert_eq!(same.layers[0].id, id);
        assert_eq!(
            told.lock().unwrap().len(),
            3,
            "same source must not restart capture"
        );
    }

    #[test]
    fn a_swap_resets_an_invalid_crop_and_only_disables_audio_for_a_window() {
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        engine.handle(Command::LayerScreen {
            id: "desk".into(),
            display: 1,
        });
        engine.handle(Command::LayerCrop {
            id: "desk".into(),
            crop: Some(crate::layers::Crop {
                x: 1500,
                y: 0,
                width: 300,
                height: 200,
            }),
        });
        engine.handle(Command::LayerScreenSound {
            id: "desk".into(),
            on: true,
        });
        let Reply::Status(display) = engine.handle(Command::LayerReplaceScreen {
            id: "desk".into(),
            display: 3,
        }) else {
            panic!("display swap")
        };
        assert!(display.screen_sound);
        assert_eq!(display.screen_sound_layer.as_deref(), Some("desk"));
        let Reply::Status(window) = engine.handle(Command::Window {
            query: "notes".into(),
        }) else {
            panic!("window swap")
        };
        assert_eq!(window.layers[0].id, "desk");
        assert_eq!(window.layers[0].source.kind, crate::layers::Kind::Window);
        assert_eq!(window.layers[0].crop, None);
        assert!(!window.screen_sound);
        assert_eq!(window.screen_sound_layer, None);
        assert!(matches!(
            engine.handle(Command::LayerReplaceCamera {
                id: "desk".into(),
                device: "HP".into()
            }),
            Reply::Error { .. }
        ));
    }

    #[test]
    fn failed_swap_keeps_the_id_or_explicitly_drops_an_unrecoverable_capture() {
        let mut engine = Engine::with_sources(Box::new(ThisMachine));
        engine.handle(Command::Screen { display: 1 });
        let original = engine.status().layers[0].clone();
        let mut engine = engine.with_pipeline(Box::new(Wrote {
            refuse: Some("new source refused".into()),
            ..Default::default()
        }));
        assert!(matches!(
            engine.handle(Command::Window {
                query: "notes".into()
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status().layers, vec![original.clone()]);
        // A missing target is validated before the pipeline sees any swap.
        assert!(matches!(
            engine.handle(Command::Window {
                query: "missing-window".into()
            }),
            Reply::Error { .. }
        ));
        assert_eq!(engine.status().layers, vec![original.clone()]);

        let mut engine = engine.with_pipeline(Box::new(Wrote {
            refuse: Some("rollback lost".into()),
            ..Default::default()
        }));
        assert!(matches!(
            engine.handle(Command::Window {
                query: "notes".into()
            }),
            Reply::Error { .. }
        ));
        assert!(
            engine.status().layers.is_empty(),
            "a lost capture must not remain in Status"
        );
    }

    #[test]
    fn a_camera_alias_reuses_the_unique_camera_but_refuses_two() {
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        engine.handle(Command::LayerCamera {
            id: "host".into(),
            device: "HP".into(),
        });
        let Reply::Status(switched) = engine.handle(Command::Camera {
            device: Some("MacBook".into()),
        }) else {
            panic!("camera swap")
        };
        assert_eq!(switched.layers[0].id, "host");
        assert_eq!(switched.layers[0].source.name, "MacBook Pro Camera");
        engine.handle(Command::LayerCamera {
            id: "guest".into(),
            device: "HP".into(),
        });
        assert!(
            matches!(engine.handle(Command::Camera { device: Some("HP".into()) }), Reply::Error { message } if message.contains("layer ID"))
        );
    }

    #[test]
    fn old_source_verbs_refuse_ambiguous_layers() {
        let mut engine =
            Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
        for id in ["left", "right"] {
            assert!(matches!(
                engine.handle(Command::LayerScreen {
                    id: id.into(),
                    display: 1
                }),
                Reply::Status(_)
            ));
        }
        assert!(
            matches!(engine.handle(Command::Screen { display: 3 }), Reply::Error { message } if message.contains("layer ID"))
        );
        assert!(
            matches!(engine.handle(Command::Window { query: "missing".into() }), Reply::Error { message } if message.contains("layer ID"))
        );
        assert!(
            matches!(engine.handle(Command::Share { on: false }), Reply::Error { message } if message.contains("layer ID"))
        );
        assert_eq!(engine.status().layers.len(), 2);
        assert!(
            matches!(engine.handle(Command::Shot { of: Framed::Screen }), Reply::Error { message } if message.contains("layer-shot"))
        );
        assert!(matches!(
            engine.handle(Command::LayerShot { id: "right".into() }),
            Reply::Shot { .. }
        ));
        assert!(matches!(
            engine.handle(Command::LayerShot {
                id: "missing".into()
            }),
            Reply::Error { .. }
        ));
        let Reply::Status(status) = engine.handle(Command::Status) else {
            panic!("status")
        };
        assert_eq!(
            status
                .layer_flowing
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["left", "right"]
        );
        for id in ["first", "second"] {
            engine.handle(Command::LayerCamera {
                id: id.into(),
                device: "HP".into(),
            });
        }
        assert!(
            matches!(engine.handle(Command::Camera { device: None }), Reply::Error { message } if message.contains("layer ID"))
        );
    }
}
