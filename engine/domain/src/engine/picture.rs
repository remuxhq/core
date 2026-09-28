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
    use super::super::tests::{layer, ThisMachine, Wrote};
    use super::*;
    use crate::layers::Kind;

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

        let mut restored = Engine::new().with_pipeline(Box::new(Wrote::refusing("shader:invalid")));
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
}
