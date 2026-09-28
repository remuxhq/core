//! The preview, written where a face reads it: the shared-memory ring
//! (`remuxd_domain::preview`), filled here from
//! libobs's raw video callback, scaled to 960x540 BGRA by libobs itself.
//! The last frame is kept for `remux shot`.

use std::ffi::{c_void, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use remuxd_domain::preview::{
    fills, Preview, CAMERA_SEQUENCE_AT, MAGIC, SCREEN_SEQUENCE_AT, SEQUENCE_AT, SLOTS,
};

use libobs as sys;

pub const WIDE: u32 = 960;
pub const TALL: u32 = 540;

pub struct Ring {
    said: Preview,
    base: *mut u8,
    sequence: AtomicU64,
    /// The last frame, whole, for a shot.
    last: Mutex<Vec<u8>>,
    callback_on: bool,
    /// The camera and the screen on their own: each rendered into a texture
    /// on libobs's render thread, staged, and copied into its ring.
    camera: Mutex<Alone>,
    screen: Mutex<Alone>,
    render_on: bool,
    /// One source asked for once, alone, at its own shape.
    snap: Mutex<Option<Snap>>,
}

/// A source rendered alone once, fitted into 960x540, for a shot of one
/// layer or one element.
struct Snap {
    source: *mut sys::obs_source_t,
    width: u32,
    height: u32,
    taken: Option<Vec<u8>>,
}

// SAFETY: as `Alone`.
unsafe impl Send for Snap {}

/// One source rendered alone, 960x540, for its ring.
#[derive(Default)]
pub struct Alone {
    pub source: *mut sys::obs_source_t,
    texrender: *mut sys::gs_texrender_t,
    stage: *mut sys::gs_stagesurf_t,
    sequence: u64,
    last: Vec<u8>,
}

// SAFETY: touched from the render thread and the engine's thread under the
// mutex; the handles are libobs's and thread-safe.
unsafe impl Send for Alone {}

// SAFETY: the mapping is written from libobs's video thread and read by
// other processes; every write ends with a release store of the sequence.
unsafe impl Send for Ring {}
unsafe impl Sync for Ring {}

impl Ring {
    pub fn new() -> Result<Box<Self>, String> {
        let name = format!("/remux.preview.{}", std::process::id());
        let said = Preview {
            name: name.clone(),
            width: WIDE,
            height: TALL,
            stride: WIDE * 4,
            slots: SLOTS,
        };
        let c_name = CString::new(name).map_err(|_| "bad region name")?;
        // SAFETY: POSIX shared memory, sized to the ring, mapped read/write.
        let base = unsafe {
            libc::shm_unlink(c_name.as_ptr());
            let fd = libc::shm_open(c_name.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
            if fd < 0 {
                return Err(format!("shm_open: {}", std::io::Error::last_os_error()));
            }
            if libc::ftruncate(fd, said.size() as i64) != 0 {
                libc::close(fd);
                return Err("ftruncate failed".into());
            }
            let base = libc::mmap(
                std::ptr::null_mut(),
                said.size(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );
            libc::close(fd);
            if base == libc::MAP_FAILED {
                return Err("mmap failed".into());
            }
            let base = base as *mut u8;
            base.cast::<u32>().write(MAGIC);
            base.add(SEQUENCE_AT).cast::<u64>().write(0);
            base.add(CAMERA_SEQUENCE_AT).cast::<u64>().write(0);
            base.add(SCREEN_SEQUENCE_AT).cast::<u64>().write(0);
            base
        };
        Ok(Box::new(Self {
            said,
            base,
            sequence: AtomicU64::new(0),
            last: Mutex::new(Vec::new()),
            callback_on: false,
            camera: Mutex::new(Alone::default()),
            screen: Mutex::new(Alone::default()),
            render_on: false,
            snap: Mutex::new(None),
        }))
    }

    pub fn said(&self) -> &Preview {
        &self.said
    }

    /// Ask libobs for every frame, scaled, or stop asking.
    pub fn watch(&mut self, on: bool) {
        if on == self.callback_on {
            return;
        }
        let scale = sys::video_scale_info {
            format: sys::video_format_VIDEO_FORMAT_BGRA,
            width: WIDE,
            height: TALL,
            range: sys::video_range_type_VIDEO_RANGE_PARTIAL,
            colorspace: sys::video_colorspace_VIDEO_CS_709,
        };
        let me = self as *mut Self as *mut c_void;
        // SAFETY: `self` is boxed and outlives the callback, which is removed
        // in `watch(false)` and in drop before the box goes.
        unsafe {
            if on {
                sys::obs_add_raw_video_callback(&scale, Some(Self::on_frame), me);
            } else {
                sys::obs_remove_raw_video_callback(Some(Self::on_frame), me);
            }
        }
        self.callback_on = on;
        // Asleep, the kept picture is only getting older: forgotten, so a
        // shot wakes the ring for one of now instead.
        if !on {
            if let Ok(mut last) = self.last.lock() {
                last.clear();
            }
        }
    }

    /// Which source the camera's and the screen's rings show; null for none.
    pub fn alone(&self, camera: *mut sys::obs_source_t, screen: *mut sys::obs_source_t) {
        if let Ok(mut it) = self.camera.lock() {
            it.source = camera;
        }
        if let Ok(mut it) = self.screen.lock() {
            it.source = screen;
        }
    }

    /// Render the two sources alone every frame, or stop.
    pub fn render(&mut self, on: bool) {
        if on == self.render_on {
            return;
        }
        let me = self as *mut Self as *mut c_void;
        // SAFETY: as `watch`.
        unsafe {
            if on {
                sys::obs_add_main_render_callback(Some(Self::on_render), me);
            } else {
                sys::obs_remove_main_render_callback(Some(Self::on_render), me);
            }
        }
        self.render_on = on;
        // As `watch`: asleep, the camera's and the screen's pictures go too.
        if !on {
            for alone in [&self.camera, &self.screen] {
                if let Ok(mut it) = alone.lock() {
                    it.last.clear();
                }
            }
        }
    }

    /// One source alone, once: rendered on the next frame, at most a second
    /// away, and handed back as a JPEG of its own shape.
    /// One source rendered alone, turned left to right when `mirrored`: the
    /// shot of a mirrored camera is the camera as the scene shows it.
    pub fn snap(
        &mut self,
        source: *mut sys::obs_source_t,
        mirrored: bool,
    ) -> Option<(Vec<u8>, u32, u32)> {
        let (w, h) = unsafe {
            (
                sys::obs_source_get_width(source),
                sys::obs_source_get_height(source),
            )
        };
        if w == 0 || h == 0 {
            return None;
        }
        let scale = (WIDE as f64 / w as f64)
            .min(TALL as f64 / h as f64)
            .min(1.0);
        let (width, height) = (
            ((w as f64 * scale).round() as u32).max(1),
            ((h as f64 * scale).round() as u32).max(1),
        );
        *self.snap.lock().ok()? = Some(Snap {
            source,
            width,
            height,
            taken: None,
        });
        let was_on = self.render_on;
        self.render(true);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let taken = loop {
            if let Some(taken) = self
                .snap
                .lock()
                .ok()
                .and_then(|mut s| s.as_mut().and_then(|s| s.taken.take()))
            {
                break Some(taken);
            }
            if std::time::Instant::now() >= until {
                break None;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        if let Ok(mut s) = self.snap.lock() {
            *s = None;
        }
        self.render(was_on);
        let mut taken = taken?;
        if mirrored {
            mirror(&mut taken, width as usize);
        }
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 80)
            .encode(
                &taken,
                width as u16,
                height as u16,
                jpeg_encoder::ColorType::Bgra,
            )
            .ok()?;
        Some((out, width, height))
    }

    /// The asked-for snap, inside the graphics context: rendered, staged and
    /// copied out, its texture and stage gone again before this returns.
    unsafe fn take_snap(&self) {
        let Ok(mut asked) = self.snap.try_lock() else {
            return;
        };
        let Some(snap) = asked.as_mut().filter(|s| s.taken.is_none()) else {
            return;
        };
        let (w, h) = (
            sys::obs_source_get_width(snap.source),
            sys::obs_source_get_height(snap.source),
        );
        if w == 0 || h == 0 {
            return;
        }
        let texrender = sys::gs_texrender_create(
            sys::gs_color_format_GS_BGRA,
            sys::gs_zstencil_format_GS_ZS_NONE,
        );
        let stage =
            sys::gs_stagesurface_create(snap.width, snap.height, sys::gs_color_format_GS_BGRA);
        if sys::gs_texrender_begin(texrender, snap.width, snap.height) {
            let clear = crate::vec4(0.0, 0.0, 0.0, 1.0);
            sys::gs_clear(sys::GS_CLEAR_COLOR, &clear, 0.0, 0);
            sys::gs_ortho(0.0, w as f32, 0.0, h as f32, -100.0, 100.0);
            sys::obs_source_video_render(snap.source);
            sys::gs_texrender_end(texrender);
            sys::gs_stage_texture(stage, sys::gs_texrender_get_texture(texrender));
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut linesize: u32 = 0;
            if sys::gs_stagesurface_map(stage, &mut data, &mut linesize) && !data.is_null() {
                let row = (snap.width * 4) as usize;
                let mut whole = Vec::with_capacity(row * snap.height as usize);
                for y in 0..snap.height as usize {
                    whole.extend_from_slice(std::slice::from_raw_parts(
                        data.add(y * linesize as usize),
                        row,
                    ));
                }
                sys::gs_stagesurface_unmap(stage);
                snap.taken = Some(whole);
            }
        }
        sys::gs_stagesurface_destroy(stage);
        sys::gs_texrender_destroy(texrender);
    }

    unsafe extern "C" fn on_render(param: *mut c_void, _cx: u32, _cy: u32) {
        // SAFETY: `param` is the ring; this runs inside libobs's graphics
        // context, where the gs_* calls are allowed.
        let ring = unsafe { &*(param as *const Self) };
        // SAFETY: as above.
        unsafe { ring.take_snap() };
        for (alone, offset_of, counts_at) in [
            (
                &ring.camera,
                Preview::camera_offset as fn(&Preview, u32) -> usize,
                CAMERA_SEQUENCE_AT,
            ),
            (
                &ring.screen,
                Preview::screen_offset as fn(&Preview, u32) -> usize,
                SCREEN_SEQUENCE_AT,
            ),
        ] {
            let Ok(mut it) = alone.try_lock() else {
                continue;
            };
            if it.source.is_null() {
                continue;
            }
            unsafe {
                if it.texrender.is_null() {
                    it.texrender = sys::gs_texrender_create(
                        sys::gs_color_format_GS_BGRA,
                        sys::gs_zstencil_format_GS_ZS_NONE,
                    );
                    it.stage =
                        sys::gs_stagesurface_create(WIDE, TALL, sys::gs_color_format_GS_BGRA);
                }
                let (w, h) = (
                    sys::obs_source_get_width(it.source),
                    sys::obs_source_get_height(it.source),
                );
                if w == 0 || h == 0 {
                    continue;
                }
                sys::gs_texrender_reset(it.texrender);
                if !sys::gs_texrender_begin(it.texrender, WIDE, TALL) {
                    continue;
                }
                let clear = crate::vec4(0.0, 0.0, 0.0, 1.0);
                sys::gs_clear(sys::GS_CLEAR_COLOR, &clear, 0.0, 0);
                // The source's own pixels mapped onto the whole 960x540.
                sys::gs_ortho(0.0, w as f32, 0.0, h as f32, -100.0, 100.0);
                sys::obs_source_video_render(it.source);
                sys::gs_texrender_end(it.texrender);
                sys::gs_stage_texture(it.stage, sys::gs_texrender_get_texture(it.texrender));
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut linesize: u32 = 0;
                if !sys::gs_stagesurface_map(it.stage, &mut data, &mut linesize) || data.is_null() {
                    continue;
                }
                let slot = fills(it.sequence, SLOTS);
                let dst = ring.base.add(offset_of(&ring.said, slot));
                let stride = ring.said.stride as usize;
                let row = (WIDE * 4) as usize;
                let mut whole = Vec::with_capacity(row * TALL as usize);
                for y in 0..TALL as usize {
                    let src = std::slice::from_raw_parts(data.add(y * linesize as usize), row);
                    std::ptr::copy_nonoverlapping(src.as_ptr(), dst.add(y * stride), row);
                    whole.extend_from_slice(src);
                }
                sys::gs_stagesurface_unmap(it.stage);
                it.sequence += 1;
                (*ring.base.add(counts_at).cast::<AtomicU64>())
                    .store(it.sequence, Ordering::Release);
                it.last = whole;
            }
        }
    }

    /// The last frame of the camera or the screen alone, as a JPEG.
    pub fn shot_alone(&self, camera: bool) -> Option<(Vec<u8>, u32, u32)> {
        let it = if camera {
            self.camera.lock().ok()?
        } else {
            self.screen.lock().ok()?
        };
        if it.last.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 80)
            .encode(
                &it.last,
                WIDE as u16,
                TALL as u16,
                jpeg_encoder::ColorType::Bgra,
            )
            .ok()?;
        Some((out, WIDE, TALL))
    }

    unsafe extern "C" fn on_frame(param: *mut c_void, frame: *mut sys::video_data) {
        // SAFETY: `param` is the ring `watch` registered; `frame` is
        // libobs's for the duration of the call.
        unsafe {
            let ring = &*(param as *const Self);
            let frame = &*frame;
            let slot = fills(ring.sequence.load(Ordering::Relaxed), SLOTS);
            let dst = ring.base.add(ring.said.offset(slot));
            let stride = ring.said.stride as usize;
            let src_stride = frame.linesize[0] as usize;
            let row = (WIDE * 4) as usize;
            let mut whole = Vec::with_capacity(row * TALL as usize);
            for y in 0..TALL as usize {
                let src = std::slice::from_raw_parts(frame.data[0].add(y * src_stride), row);
                std::ptr::copy_nonoverlapping(src.as_ptr(), dst.add(y * stride), row);
                whole.extend_from_slice(src);
            }
            let next = ring.sequence.fetch_add(1, Ordering::Relaxed) + 1;
            (*ring.base.add(SEQUENCE_AT).cast::<AtomicU64>()).store(next, Ordering::Release);
            if let Ok(mut last) = ring.last.lock() {
                *last = whole;
            }
        }
    }

    /// The last frame as a JPEG, and its size.
    pub fn shot(&self) -> Option<(Vec<u8>, u32, u32)> {
        let last = self.last.lock().ok()?;
        if last.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        let encoder = jpeg_encoder::Encoder::new(&mut out, 80);
        encoder
            .encode(
                &last,
                WIDE as u16,
                TALL as u16,
                jpeg_encoder::ColorType::Bgra,
            )
            .ok()?;
        Some((out, WIDE, TALL))
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        self.watch(false);
        self.render(false);
        // SAFETY: the graphics objects go inside the graphics context.
        unsafe {
            sys::obs_enter_graphics();
            for alone in [&self.camera, &self.screen] {
                if let Ok(it) = alone.lock() {
                    if !it.texrender.is_null() {
                        sys::gs_texrender_destroy(it.texrender);
                        sys::gs_stagesurface_destroy(it.stage);
                    }
                }
            }
            sys::obs_leave_graphics();
        }
        // SAFETY: mapped in `new`.
        unsafe {
            libc::munmap(self.base.cast(), self.said.size());
        }
    }
}

/// BGRA rows turned left to right, in place.
fn mirror(pixels: &mut [u8], width: usize) {
    for row in pixels.chunks_exact_mut(width * 4) {
        for x in 0..width / 2 {
            let (a, b) = (x * 4, (width - 1 - x) * 4);
            for c in 0..4 {
                row.swap(a + c, b + c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_mirrored_shot_is_turned_left_to_right() {
        let mut pixels = vec![
            1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6,
        ];
        super::mirror(&mut pixels, 3);
        assert_eq!(
            pixels,
            vec![3, 3, 3, 3, 2, 2, 2, 2, 1, 1, 1, 1, 6, 6, 6, 6, 5, 5, 5, 5, 4, 4, 4, 4]
        );
    }
}
