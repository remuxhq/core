//! The picture over libobs: every layer a source and an item in the one
//! scene, every element a `remux_element` of its own size, one back-to-front
//! order for both, and the operator's filters on whichever of them asked.
//!
//! What the tick needs (the items, the sizes they were placed at, the
//! running clocks) lives in [`Drawn`], behind a mutex the tick only tries:
//! a frame never waits on the engine.

use std::ffi::c_void;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use remuxd_domain::engine::{LayerSwapError, Picture};
use remuxd_domain::picture::layers::{Kind, Layer, Source};
use remuxd_domain::picture::scenes::{Element, ElementContent};
use remuxd_domain::protocol::{Flowing, Framed};

use crate::pipeline::ObsPipeline;
use crate::place::{placement, Region};
use crate::{c, effect};
use libobs as sys;

/// How long a capture has to say its size before it counts as not there.
/// A camera's first frame on a cold start was measured at under a second.
const FIRST_FRAME: Duration = Duration::from_secs(4);

#[derive(Default)]
pub struct Drawn {
    pub layers: Vec<Drawing>,
    pub elements: Vec<Written>,
    /// Back to front, layers and elements together, as the scene says.
    pub order: Vec<String>,
}

// SAFETY: libobs handles, used under the mutex from the engine's thread and
// the tick; libobs is thread-safe about them.
unsafe impl Send for Drawn {}

/// One layer on the picture.
pub struct Drawing {
    pub layer: Layer,
    pub source: *mut sys::obs_source_t,
    pub item: *mut sys::obs_sceneitem_t,
    /// The source's size the item was last placed for.
    pub size: (u32, u32),
    pub filter: Option<(String, *mut sys::obs_source_t)>,
    pub mask: *mut sys::obs_source_t,
    pub masked: Option<((u32, u32), Region)>,
}

/// One element on the picture.
pub struct Written {
    pub element: Element,
    pub source: *mut sys::obs_source_t,
    pub item: *mut sys::obs_sceneitem_t,
    pub filter: Option<(String, *mut sys::obs_source_t)>,
    /// When a running timer reaches zero.
    pub deadline: Option<Instant>,
    pub words: String,
}

/// What a capture is, as a physical thing: two layers of the same display
/// are one capture as far as a scene switch is concerned.
fn size_of(source: *mut sys::obs_source_t) -> (u32, u32) {
    // SAFETY: pure reads on a live source.
    unsafe {
        (
            sys::obs_source_get_width(source),
            sys::obs_source_get_height(source),
        )
    }
}

fn clock(left: Duration) -> String {
    remuxd_domain::picture::timer::clock(left.as_secs_f64().ceil() as i64)
}

/// A timer's words now, and when it runs out if it is running.
fn timer_words(element: &Element, left: Option<Duration>) -> (String, Option<Instant>) {
    match (&element.content, left) {
        (ElementContent::Text { text }, _) => (text.clone(), None),
        (ElementContent::Timer { .. }, Some(left)) => (clock(left), Some(Instant::now() + left)),
        (ElementContent::Timer { seconds }, None) => (
            remuxd_domain::picture::timer::clock(i64::from(*seconds)),
            None,
        ),
    }
}

impl Drawing {
    /// Where the layer says, for the size its source has now; the circle's
    /// mask redrawn when what it is cut from moved.
    fn place(&mut self) {
        self.size = size_of(self.source);
        let placed = placement(&self.layer, self.size);
        // SAFETY: the item, the source and the mask are this drawing's own.
        unsafe {
            sys::obs_sceneitem_set_alignment(self.item, sys::OBS_ALIGN_CENTER);
            sys::obs_sceneitem_set_bounds_alignment(self.item, sys::OBS_ALIGN_CENTER);
            sys::obs_sceneitem_set_bounds_type(
                self.item,
                sys::obs_bounds_type_OBS_BOUNDS_SCALE_INNER,
            );
            sys::obs_sceneitem_set_bounds(
                self.item,
                &crate::vec2(placed.bounds.0, placed.bounds.1),
            );
            // Mirroring is a negative scale; the bounds decide the size.
            sys::obs_sceneitem_set_scale(
                self.item,
                &crate::vec2(if placed.mirrored { -1.0 } else { 1.0 }, 1.0),
            );
            sys::obs_sceneitem_set_pos(self.item, &crate::vec2(placed.centre.0, placed.centre.1));
            sys::obs_sceneitem_set_rot(self.item, placed.degrees);
            sys::obs_sceneitem_set_crop(
                self.item,
                &sys::obs_sceneitem_crop {
                    left: placed.crop.0,
                    top: placed.crop.1,
                    right: placed.crop.2,
                    bottom: placed.crop.3,
                },
            );
            sys::obs_sceneitem_set_visible(self.item, placed.visible);
        }
        let wanted = placed.circle.map(|region| (self.size, region));
        if wanted != self.masked {
            self.remask(wanted);
        }
    }

    fn remask(&mut self, wanted: Option<((u32, u32), Region)>) {
        // SAFETY: the mask is this drawing's own, on its source.
        unsafe {
            if !self.mask.is_null() {
                sys::obs_source_filter_remove(self.source, self.mask);
                sys::obs_source_release(self.mask);
                self.mask = std::ptr::null_mut();
            }
        }
        self.masked = None;
        let Some((size, region)) = wanted else {
            return;
        };
        let file = crate::text::folder().join(format!(
            "circle-{}x{}-{}-{}-{}.png",
            size.0, size.1, region.x, region.y, region.width
        ));
        let Ok(file) = crate::text::circle_mask(&file, size, region) else {
            return;
        };
        // SAFETY: settings released after the create; the filter is ours.
        unsafe {
            let settings = sys::obs_data_create();
            sys::obs_data_set_string(
                settings,
                c"type".as_ptr(),
                c"mask_alpha_filter.effect".as_ptr(),
            );
            sys::obs_data_set_string(
                settings,
                c"image_path".as_ptr(),
                c(&file.display().to_string()).as_ptr(),
            );
            sys::obs_data_set_bool(settings, c"stretch".as_ptr(), true);
            let mask = sys::obs_source_create(
                c"mask_filter_v2".as_ptr(),
                c"shape".as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            if !mask.is_null() {
                sys::obs_source_filter_add(self.source, mask);
                self.mask = mask;
                self.masked = wanted;
            }
        }
    }

    /// The operator's filter on this layer, or none; the mask stays last.
    fn refilter(&mut self, path: Option<&str>) -> Result<(), String> {
        if self.filter.as_ref().map(|(p, _)| p.as_str()) == path {
            return Ok(());
        }
        let made = path.map(effect::filter).transpose()?;
        let masked = self.masked;
        self.remask(None);
        set_filter(self.source, &mut self.filter, path, made);
        self.remask(masked);
        Ok(())
    }

    fn release(mut self) {
        self.remask(None);
        set_filter(self.source, &mut self.filter, None, None);
        // SAFETY: the item goes before its source.
        unsafe {
            sys::obs_sceneitem_remove(self.item);
            sys::obs_source_release(self.source);
        }
    }
}

/// Swap the filter in `slot` on `source` for `made` (built from `path`).
fn set_filter(
    source: *mut sys::obs_source_t,
    slot: &mut Option<(String, *mut sys::obs_source_t)>,
    path: Option<&str>,
    made: Option<*mut sys::obs_source_t>,
) {
    // SAFETY: the old filter is ours, on this source; the new one is handed
    // over to the slot.
    unsafe {
        if let Some((_, old)) = slot.take() {
            sys::obs_source_filter_remove(source, old);
            sys::obs_source_release(old);
        }
        if let (Some(path), Some(made)) = (path, made) {
            sys::obs_source_filter_add(source, made);
            *slot = Some((path.to_string(), made));
        }
    }
}

impl Written {
    fn release(mut self) {
        set_filter(self.source, &mut self.filter, None, None);
        // SAFETY: the item goes before its source.
        unsafe {
            sys::obs_sceneitem_remove(self.item);
            sys::obs_source_release(self.source);
        }
    }
}

impl Drawn {
    /// Back to front, the scene's order; what it does not name keeps its place.
    fn reorder(&self) {
        for id in &self.order {
            let item = self
                .layers
                .iter()
                .find(|d| &d.layer.id == id)
                .map(|d| d.item)
                .or_else(|| {
                    self.elements
                        .iter()
                        .find(|w| &w.element.id == id)
                        .map(|w| w.item)
                });
            if let Some(item) = item {
                // SAFETY: an item of this scene.
                unsafe {
                    sys::obs_sceneitem_set_order(item, sys::obs_order_movement_OBS_ORDER_MOVE_TOP)
                };
            }
        }
    }

    /// Every frame: a source whose size changed is placed again (a camera's
    /// first frame, a window resized), and a running clock rewritten when
    /// its second turns.
    fn tick(&mut self) {
        for drawing in &mut self.layers {
            if size_of(drawing.source) != drawing.size {
                drawing.place();
            }
        }
        for written in &mut self.elements {
            let Some(deadline) = written.deadline else {
                continue;
            };
            let words = clock(deadline.saturating_duration_since(Instant::now()));
            if words != written.words {
                effect::reword(
                    written.source,
                    written.element.width,
                    written.element.height,
                    &words,
                );
                written.words = words;
            }
        }
    }

    fn first(&self, kinds: &[Kind]) -> *mut sys::obs_source_t {
        self.layers
            .iter()
            .find(|d| kinds.contains(&d.layer.source.kind))
            .map_or(std::ptr::null_mut(), |d| d.source)
    }
}

pub unsafe extern "C" fn tick(param: *mut c_void, _seconds: f32) {
    // SAFETY: `param` is the pipeline's boxed `Mutex<Drawn>`, registered
    // with the scene and removed before the box goes.
    let drawn = unsafe { &*(param as *const Mutex<Drawn>) };
    if let Ok(mut drawn) = drawn.try_lock() {
        drawn.tick();
    }
}

impl ObsPipeline {
    /// A capture of this source, opened: a display, a window, a camera or a
    /// picture file.
    fn open(&self, source: &Source) -> Result<*mut sys::obs_source_t, String> {
        // SAFETY: settings created and released around the create; the
        // source is the caller's.
        unsafe {
            let (kind, settings) = match source.kind {
                Kind::Screen => {
                    let table = crate::platform::screen();
                    let known = self
                        .known
                        .lock()
                        .map_err(|_| "the display list is poisoned")?;
                    // The monitor itself when the layer knows it, whatever
                    // its number now; the number otherwise.
                    let uuid = match source.stable.as_ref().filter(|_| table.stable_displays) {
                        Some(stable) => known
                            .displays
                            .values()
                            .find(|uuid| *uuid == stable)
                            .cloned()
                            .ok_or_else(|| format!("{} is not connected", source.name))?,
                        None => {
                            let number: u32 = source.handle.parse().map_err(|_| {
                                format!("display {:?} is not a number", source.handle)
                            })?;
                            known.displays.get(&number).cloned().ok_or_else(|| {
                                format!("no display {number}: remux devices lists them")
                            })?
                        }
                    };
                    drop(known);
                    let settings = sys::obs_data_create();
                    if let Some(kind) = table.kind_key {
                        sys::obs_data_set_int(settings, c(kind).as_ptr(), 0);
                    }
                    if table.portal {
                        // The portal picks; a token from last time skips its dialog.
                        if let (Some(key), Ok(token)) = (
                            table.token_key,
                            std::fs::read_to_string(crate::platform::portal_token_path()),
                        ) {
                            if !token.trim().is_empty() {
                                sys::obs_data_set_string(
                                    settings,
                                    c(key).as_ptr(),
                                    c(token.trim()).as_ptr(),
                                );
                            }
                        }
                    } else {
                        // One OS names a display by uuid, another by number.
                        match uuid.parse::<i64>() {
                            Ok(n) if table.kind_key.is_none() => {
                                sys::obs_data_set_int(settings, c(table.display_key).as_ptr(), n)
                            }
                            _ => sys::obs_data_set_string(
                                settings,
                                c(table.display_key).as_ptr(),
                                c(&uuid).as_ptr(),
                            ),
                        }
                    }
                    (table.source, settings)
                }
                Kind::Window => {
                    let table = crate::platform::screen();
                    if table.portal {
                        return Err("under Wayland the portal picks a window: add a screen layer, then choose it in the dialog".into());
                    }
                    let id: u32 = source
                        .handle
                        .parse()
                        .map_err(|_| format!("window {:?} is not a number", source.handle))?;
                    let settings = sys::obs_data_create();
                    if let Some(kind) = table.kind_key {
                        sys::obs_data_set_int(settings, c(kind).as_ptr(), 1);
                        sys::obs_data_set_int(
                            settings,
                            c(table.window_key).as_ptr(),
                            i64::from(id),
                        );
                    } else {
                        // xcomposite names a window "id\r\nname\r\nclass": the
                        // id alone matches the window.
                        sys::obs_data_set_string(
                            settings,
                            c(table.window_key).as_ptr(),
                            c(&format!("{id}\r\n\r\n")).as_ptr(),
                        );
                    }
                    (table.window_source, settings)
                }
                Kind::Camera => {
                    let table = &crate::platform::TABLE.camera;
                    let settings = sys::obs_data_create();
                    sys::obs_data_set_string(
                        settings,
                        c(table.device_key).as_ptr(),
                        c(&source.handle).as_ptr(),
                    );
                    if let Some((key, preset)) = table.preset {
                        sys::obs_data_set_string(settings, c(key).as_ptr(), c(preset).as_ptr());
                    }
                    // Held at the picture's 30.
                    sys::obs_data_set_frames_per_second(
                        settings,
                        c"frame_rate".as_ptr(),
                        sys::media_frames_per_second {
                            numerator: 30,
                            denominator: 1,
                        },
                        std::ptr::null(),
                    );
                    (table.source, settings)
                }
                Kind::Image => {
                    let settings = sys::obs_data_create();
                    sys::obs_data_set_string(
                        settings,
                        c"file".as_ptr(),
                        c(&source.handle).as_ptr(),
                    );
                    // Loaded once and kept, whether shown or not.
                    sys::obs_data_set_bool(settings, c"unload".as_ptr(), false);
                    ("image_source", settings)
                }
            };
            let made = sys::obs_source_create(
                c(kind).as_ptr(),
                c(&source.name).as_ptr(),
                settings,
                std::ptr::null_mut(),
            );
            sys::obs_data_release(settings);
            if made.is_null() {
                return Err(format!("libobs could not open {}", source.name));
            }
            Ok(made)
        }
    }

    /// The source's size once it has one; `(0, 0)` if it never says.
    fn first_size(source: *mut sys::obs_source_t) -> (u32, u32) {
        let until = Instant::now() + FIRST_FRAME;
        loop {
            let size = size_of(source);
            if (size.0 > 0 && size.1 > 0) || Instant::now() >= until {
                return size;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// An item for a source, hidden until it is placed.
    fn item(&mut self, source: *mut sys::obs_source_t) -> *mut sys::obs_sceneitem_t {
        let scene = self.scene();
        // SAFETY: the scene takes its own reference to the source.
        unsafe {
            let item = sys::obs_scene_add(scene, source);
            sys::obs_sceneitem_set_visible(item, false);
            item
        }
    }

    fn drawn(&self) -> std::sync::MutexGuard<'_, Drawn> {
        self.picture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The rings follow the layers of now: the first camera, and the first
    /// display or window.
    fn point_rings(&mut self) {
        let (camera, screen) = {
            let drawn = self.drawn();
            (
                drawn.first(&[Kind::Camera]),
                drawn.first(&[Kind::Screen, Kind::Window]),
            )
        };
        if let Some(ring) = self.preview.as_deref() {
            ring.alone(camera, screen);
        }
    }

    /// The first display layer's source, for what reads a display's settings.
    pub(crate) fn first_screen(&self) -> *mut sys::obs_source_t {
        self.drawn().first(&[Kind::Screen])
    }

    /// Every layer and element off the picture.
    pub(crate) fn clear_picture(&mut self) {
        let (layers, elements) = {
            let mut drawn = self.drawn();
            (
                std::mem::take(&mut drawn.layers),
                std::mem::take(&mut drawn.elements),
            )
        };
        if let Some(ring) = self.preview.as_deref() {
            ring.alone(std::ptr::null_mut(), std::ptr::null_mut());
        }
        layers.into_iter().for_each(Drawing::release);
        elements.into_iter().for_each(Written::release);
        let _ = self.shader(None);
    }

    /// One capture's counters. libobs does not count a source's own frames,
    /// so a source with a picture counts the scene's while it has one: a rate
    /// read off two of these is the picture's, and zero is no picture.
    fn flowing_of(source: *mut sys::obs_source_t) -> Flowing {
        let (width, height) = size_of(source);
        // SAFETY: a pure read.
        let frames = unsafe { sys::obs_get_total_frames() } as u64;
        Flowing {
            captured: if width > 0 { frames } else { 0 },
            frames,
            width,
            height,
            held: (width > 0).then_some(30),
        }
    }

    /// Whether any camera layer has delivered a frame: the camera's grant.
    pub(crate) fn a_camera_delivers(&self) -> bool {
        self.drawn()
            .layers
            .iter()
            .any(|d| d.layer.source.kind == Kind::Camera && size_of(d.source).0 > 0)
    }

    /// One source rendered alone, for a shot of one layer or one element.
    fn snap(
        &mut self,
        source: *mut sys::obs_source_t,
        mirrored: bool,
    ) -> Option<(Vec<u8>, u32, u32)> {
        self.ring()?.snap(source, mirrored)
    }
}

impl Picture for ObsPipeline {
    fn show(
        &mut self,
        elements: &[Element],
        timers: &[(String, Duration)],
        order: &[String],
    ) -> Result<(), String> {
        let gone: Vec<Written> = {
            let mut drawn = self.drawn();
            let (kept, gone) = std::mem::take(&mut drawn.elements)
                .into_iter()
                .partition(|w| elements.iter().any(|e| e.id == w.element.id));
            drawn.elements = kept;
            gone
        };
        gone.into_iter().for_each(Written::release);
        for element in elements {
            let left = timers
                .iter()
                .find(|(id, _)| id == &element.id)
                .map(|(_, left)| *left);
            let (words, deadline) = timer_words(element, left);
            let wanted = self
                .element_filters
                .get(&element.id)
                .cloned()
                .or_else(|| element.shader.clone());
            let there = self
                .drawn()
                .elements
                .iter()
                .any(|w| w.element.id == element.id);
            if !there {
                let source = effect::element(element.width, element.height, &words)?;
                let item = self.item(source);
                // SAFETY: the item is new and this scene's.
                unsafe {
                    sys::obs_sceneitem_set_alignment(item, sys::OBS_ALIGN_LEFT | sys::OBS_ALIGN_TOP)
                };
                self.drawn().elements.push(Written {
                    element: element.clone(),
                    source,
                    item,
                    filter: None,
                    deadline,
                    words: words.clone(),
                });
            }
            let mut drawn = self.drawn();
            let written = drawn
                .elements
                .iter_mut()
                .find(|w| w.element.id == element.id)
                .expect("drawn above");
            if written.words != words
                || (written.element.width, written.element.height)
                    != (element.width, element.height)
            {
                effect::reword(written.source, element.width, element.height, &words);
            }
            written.words = words;
            written.deadline = deadline;
            written.element = element.clone();
            // SAFETY: the item is this element's.
            unsafe {
                sys::obs_sceneitem_set_pos(
                    written.item,
                    &crate::vec2(element.x as f32, element.y as f32),
                );
                sys::obs_sceneitem_set_visible(written.item, element.visible);
            }
            if written.filter.as_ref().map(|(p, _)| p) != wanted.as_ref() {
                let made = wanted.as_deref().map(effect::filter).transpose();
                match made {
                    Ok(made) => {
                        set_filter(written.source, &mut written.filter, wanted.as_deref(), made)
                    }
                    Err(why) => remuxd_domain::log::note(&format!(
                        "element {}: its filter did not build: {why}",
                        element.id
                    )),
                }
            }
        }
        let mut drawn = self.drawn();
        drawn.order = order.to_vec();
        drawn.reorder();
        Ok(())
    }

    fn stop(&mut self) {
        let gone = std::mem::take(&mut self.drawn().elements);
        gone.into_iter().for_each(Written::release);
    }

    fn shot(&mut self, of: Framed) -> Option<(Vec<u8>, u32, u32)> {
        // The ring first: one made here is pointed at the camera and the
        // screen too, or it renders neither until the next ask.
        self.ring()?;
        self.point_rings();
        let ring = self.preview.as_deref_mut()?;
        let taken = |ring: &crate::preview::Ring| match of {
            Framed::Scene => ring.shot(),
            Framed::Camera => ring.shot_alone(true),
            Framed::Screen => ring.shot_alone(false),
        };
        if let Some(shot) = taken(ring) {
            return Some(shot);
        }
        // A ring just woken takes a few renders to hold a picture: a camera
        // added a moment ago measured about half a second here.
        ring.watch(true);
        ring.render(true);
        let until = Instant::now() + Duration::from_secs(1);
        let shot = loop {
            std::thread::sleep(Duration::from_millis(20));
            let shot = self.preview.as_deref().and_then(taken);
            if shot.is_some() || Instant::now() >= until {
                break shot;
            }
        };
        // Back to sleep unless a face is watching: left awake, libobs scaled
        // every frame into the ring on the CPU for nobody, 45% of a core of
        // an idle engine at 1080p30 (`sample`, all of it in swscale).
        let watched = self.previewing;
        if let Some(ring) = self.preview.as_deref_mut() {
            ring.watch(watched);
            ring.render(watched);
        }
        shot
    }

    fn layer_shot(&mut self, id: &str) -> Option<(Vec<u8>, u32, u32)> {
        let (source, mirrored) =
            self.drawn()
                .layers
                .iter()
                .find(|d| d.layer.id == id)
                .map(|d| {
                    (
                        d.source,
                        d.layer.source.kind == Kind::Camera && d.layer.mirrored,
                    )
                })?;
        self.snap(source, mirrored)
    }

    fn element_shot(
        &mut self,
        element: &Element,
        time: Option<Duration>,
    ) -> Option<(Vec<u8>, u32, u32)> {
        let (words, _) = timer_words(element, time);
        let source = effect::element(element.width, element.height, &words).ok()?;
        let mut filter = None;
        if let Some(path) = self
            .element_filters
            .get(&element.id)
            .cloned()
            .or_else(|| element.shader.clone())
        {
            if let Ok(made) = effect::filter(&path) {
                set_filter(source, &mut filter, Some(&path), Some(made));
            }
        }
        let shot = self.snap(source, false);
        set_filter(source, &mut filter, None, None);
        // SAFETY: ours, made above.
        unsafe { sys::obs_source_release(source) };
        shot
    }

    /// The panel's self-view flip. A broadcast camera is mirrored by its own
    /// layer's switch, so this changes nothing that goes out.
    fn mirror(&mut self, on: bool) {
        self.mirror = on;
    }

    fn layer_add(&mut self, layer: &Layer) -> Result<(u32, u32), String> {
        let source = self.open(&layer.source)?;
        let size = Self::first_size(source);
        if size.0 == 0 || size.1 == 0 {
            // SAFETY: ours, never put anywhere.
            unsafe { sys::obs_source_release(source) };
            let named = if layer.source.name.trim().is_empty() {
                format!("{:?} {}", layer.source.kind, layer.source.handle).to_lowercase()
            } else {
                layer.source.name.clone()
            };
            return Err(format!(
                "{named} gave no picture in {} s",
                FIRST_FRAME.as_secs()
            ));
        }
        let item = self.item(source);
        let mut drawing = Drawing {
            layer: layer.clone(),
            source,
            item,
            size,
            filter: None,
            mask: std::ptr::null_mut(),
            masked: None,
        };
        if let Err(why) = drawing.refilter(layer.shader.as_deref()) {
            remuxd_domain::log::note(&format!("layer {}: {why}", layer.id));
        }
        drawing.place();
        {
            let mut drawn = self.drawn();
            drawn.layers.push(drawing);
            drawn.reorder();
        }
        self.point_rings();
        Ok(size)
    }

    fn layer_remove(&mut self, id: &str) {
        let gone = {
            let mut drawn = self.drawn();
            drawn
                .layers
                .iter()
                .position(|d| d.layer.id == id)
                .map(|at| drawn.layers.remove(at))
        };
        if let Some(drawing) = gone {
            if let Some(ring) = self.preview.as_deref() {
                ring.alone(std::ptr::null_mut(), std::ptr::null_mut());
            }
            drawing.release();
        }
        self.point_rings();
    }

    /// The old capture closes before the new one opens, as the port asks;
    /// when the new one will not, the old one is opened again.
    fn layer_replace(&mut self, old: &Layer, new: &Layer) -> Result<(u32, u32), LayerSwapError> {
        self.layer_remove(&old.id);
        match self.layer_add(new) {
            Ok(size) => Ok(size),
            Err(reason) => Err(LayerSwapError {
                reason,
                restored: self.layer_add(old).is_ok(),
            }),
        }
    }

    /// Everything the next scene needs is opened and every filter built
    /// before anything of this one changes; a capture both scenes show is
    /// kept, item and all, so it never blinks.
    fn scene_transition(
        &mut self,
        _from: &[Layer],
        to: &[Layer],
        elements: &[Element],
        shader: Option<&str>,
    ) -> Result<(), String> {
        if let Some(path) = shader {
            effect::check(path)?;
        }
        for layer in to {
            if let Some(path) = &layer.shader {
                effect::check(path).map_err(|why| format!("layer {}: {why}", layer.id))?;
            }
        }
        for element in elements {
            if let Some(path) = &element.shader {
                effect::check(path).map_err(|why| format!("element {}: {why}", element.id))?;
            }
        }
        let mut pool: Vec<Option<Drawing>> = std::mem::take(&mut self.drawn().layers)
            .into_iter()
            .map(Some)
            .collect();
        let mut kept: Vec<Option<usize>> = Vec::new();
        for layer in to {
            let reuse = pool.iter().enumerate().position(|(at, d)| {
                d.as_ref()
                    .is_some_and(|d| d.layer.source.same_capture(&layer.source))
                    && !kept.contains(&Some(at))
            });
            kept.push(reuse);
        }
        let mut opened: Vec<*mut sys::obs_source_t> = Vec::new();
        for (layer, reuse) in to.iter().zip(&kept) {
            if reuse.is_some() {
                continue;
            }
            match self.open(&layer.source) {
                Ok(source) => opened.push(source),
                Err(why) => {
                    // SAFETY: opened here and never put anywhere.
                    unsafe { opened.into_iter().for_each(|s| sys::obs_source_release(s)) };
                    self.drawn().layers = pool.into_iter().flatten().collect();
                    return Err(why);
                }
            }
        }
        let mut opened = opened.into_iter();
        let mut next = Vec::new();
        for (layer, reuse) in to.iter().zip(&kept) {
            let mut drawing = match reuse {
                Some(at) => pool[*at].take().expect("kept once"),
                None => {
                    let source = opened.next().expect("opened above");
                    let item = self.item(source);
                    Drawing {
                        layer: layer.clone(),
                        source,
                        item,
                        size: (0, 0),
                        filter: None,
                        mask: std::ptr::null_mut(),
                        masked: None,
                    }
                }
            };
            drawing.layer = layer.clone();
            let _ = drawing.refilter(layer.shader.as_deref());
            drawing.place();
            next.push(drawing);
        }
        if let Some(ring) = self.preview.as_deref() {
            ring.alone(std::ptr::null_mut(), std::ptr::null_mut());
        }
        pool.into_iter().flatten().for_each(Drawing::release);
        {
            let mut drawn = self.drawn();
            drawn.layers = next;
            drawn.reorder();
        }
        self.point_rings();
        self.shader(shader)
    }

    fn layers_changed(&mut self, layers: &[Layer]) {
        let mut drawn = self.drawn();
        for drawing in &mut drawn.layers {
            let Some(layer) = layers.iter().find(|l| l.id == drawing.layer.id) else {
                continue;
            };
            drawing.layer = layer.clone();
            if let Err(why) = drawing.refilter(layer.shader.as_deref()) {
                remuxd_domain::log::note(&format!("layer {}: {why}", layer.id));
            }
            drawing.place();
        }
        if drawn.order.is_empty() {
            drawn.order = layers.iter().map(|l| l.id.clone()).collect();
        }
        drawn.reorder();
    }

    /// Screen sound is the system's, whichever display asked for it: the
    /// capture hears every app but this one, not one display's.
    fn screen_audio(&mut self, id: Option<&str>) -> Result<(), String> {
        self.set_screen_sound(id.is_some())
    }

    fn shader(&mut self, path: Option<&str>) -> Result<(), String> {
        if self.scene_filter.as_ref().map(|(p, _)| p.as_str()) == path {
            return Ok(());
        }
        let made = path.map(effect::filter).transpose()?;
        let scene = self.scene();
        // SAFETY: the scene's own source; the old filter is ours on it.
        let source = unsafe { sys::obs_scene_get_source(scene) };
        let mut slot = self.scene_filter.take();
        set_filter(source, &mut slot, path, made);
        self.scene_filter = slot;
        Ok(())
    }

    fn layer_shader(&mut self, layer: &Layer, path: Option<&str>) -> Result<(), String> {
        let mut drawn = self.drawn();
        match drawn.layers.iter_mut().find(|d| d.layer.id == layer.id) {
            Some(drawing) => {
                drawing.refilter(path)?;
                drawing.layer.shader = path.map(String::from);
                Ok(())
            }
            None => path.map_or(Ok(()), effect::check),
        }
    }

    fn element_shader(&mut self, element: &Element, path: Option<&str>) -> Result<(), String> {
        match path {
            Some(path) => {
                effect::check(path)?;
                self.element_filters
                    .insert(element.id.clone(), path.to_string());
            }
            None => {
                self.element_filters.remove(&element.id);
            }
        }
        let mut drawn = self.drawn();
        if let Some(written) = drawn
            .elements
            .iter_mut()
            .find(|w| w.element.id == element.id)
        {
            if written.filter.as_ref().map(|(p, _)| p.as_str()) != path {
                let made = path.map(effect::filter).transpose()?;
                set_filter(written.source, &mut written.filter, path, made);
            }
        }
        Ok(())
    }

    fn flowing(&self) -> Flowing {
        self.keep_portal_token();
        let captured = self.drawn().layers.iter().any(|d| size_of(d.source).0 > 0);
        // libobs renders an empty scene as steadily as a full one, as the
        // native motor does: the plan keeps an empty scene off the air.
        // SAFETY: a pure read of libobs's counter.
        let frames = unsafe { sys::obs_get_total_frames() as u64 };
        Flowing {
            captured: if captured { frames } else { 0 },
            frames,
            width: self.width(),
            height: self.height(),
            held: None,
        }
    }

    fn layer_flowing(&self, id: &str) -> Flowing {
        self.drawn()
            .layers
            .iter()
            .find(|d| d.layer.id == id)
            .map_or_else(Flowing::default, |d| Self::flowing_of(d.source))
    }

    fn preview(&self) -> Option<remuxd_domain::picture::preview::Preview> {
        self.preview.as_ref().map(|ring| ring.said().clone())
    }

    fn previewing(&mut self, on: bool) {
        self.previewing = on;
        self.point_rings();
        if let Some(ring) = self.ring() {
            ring.watch(on);
            ring.render(on);
        }
    }
}
