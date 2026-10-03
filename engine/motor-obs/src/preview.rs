//! The preview, written where a face reads it: the shared-memory ring
//! (`remuxd_domain::picture::preview`), filled here from the scene's own
//! texture, scaled to 960x540 BGRA on the GPU and read back a frame later.
//! The last frame is kept for `remux shot`.

use std::ffi::{c_void, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use remuxd_domain::picture::preview::{
    fills, Preview, CAMERA_SEQUENCE_AT, MAGIC, SCREEN_SEQUENCE_AT, SEQUENCE_AT, SLOTS,
    STAGED_SEQUENCE_AT,
};

use libobs as sys;

/// How many frames have landed, and a shot waiting for the next one woken
/// when it does. Polled every 20 ms, a shot noticed its frame 14 ms late at
/// the median of 81 ms inside the engine (50 shots, 1080p30). Woken, `remux
/// scene shot --out` took 33 ms at the median and 36 at p90, from 43 and 121
/// (150 shots each, the two engines in turns, three rounds).
#[derive(Default)]
pub struct Landed {
    tally: Mutex<Tally>,
    bumped: Condvar,
}

#[derive(Default)]
struct Tally {
    frames: u64,
    waiting: usize,
}

impl Landed {
    pub fn count(&self) -> u64 {
        self.tally.lock().map_or(0, |t| t.frames)
    }

    /// How many shots are waiting for a frame now.
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.tally.lock().map_or(0, |t| t.waiting)
    }

    /// A frame landed: every shot waiting is woken.
    pub fn bump(&self) {
        if let Ok(mut t) = self.tally.lock() {
            t.frames += 1;
        }
        self.bumped.notify_all();
    }

    /// The count once it is past `seen`, or `None` at `until`.
    pub fn after(&self, seen: u64, until: Instant) -> Option<u64> {
        let mut t = self.tally.lock().ok()?;
        t.waiting += 1;
        let left = until.saturating_duration_since(Instant::now());
        let (mut t, _) = self
            .bumped
            .wait_timeout_while(t, left, |t| t.frames == seen)
            .ok()?;
        t.waiting -= 1;
        (t.frames != seen).then_some(t.frames)
    }
}

/// Two stage surfaces taken in turns: a frame is staged into one while the
/// one staged a frame before is read, so reading never waits for the GPU to
/// finish the copy it was just given.
///
/// The first frame after waking is the exception: it is read at once from
/// where it was staged, because a shot is waiting for it. Read a frame later,
/// a shot of a sleeping ring took 67 ms at the median instead of 33 (600
/// shots each, 1080p30). That one wait holds libobs's graphics thread 0.9 ms at
/// the median and 10 at worst, of the 33 a frame has (900 wakes).
#[derive(Default)]
pub struct Turns {
    at: usize,
    staged: [bool; 2],
    woken: bool,
}

impl Turns {
    /// This frame's surface to stage into, and the one to read, if any.
    pub fn next(&mut self) -> (usize, Option<usize>) {
        let now = self.at;
        let before = 1 - now;
        self.at = before;
        if !self.woken {
            self.woken = true;
            return (now, Some(now));
        }
        let read = self.staged[before].then_some(before);
        self.staged[now] = true;
        (now, read)
    }

    /// Asleep, what is staged is only getting older.
    pub fn forget(&mut self) {
        *self = Self::default();
    }
}

/// A picture drawn into 960x540 on the GPU and read back in turns.
#[derive(Default)]
struct Readback {
    texrender: usize,
    stages: [usize; 2],
    turns: Turns,
}

impl Readback {
    /// Draws the frame with `draw`, `width` by `height` of its own pixels
    /// onto the whole 960x540, stages it, and hands `land` the frame its
    /// turn reads. Inside the graphics context.
    unsafe fn frame(
        &mut self,
        width: u32,
        height: u32,
        draw: impl FnOnce(),
        land: impl FnOnce(&[u8]),
    ) {
        // SAFETY: the caller is inside the graphics context; the handles are
        // made here, used only here and destroyed in `destroy`.
        unsafe {
            if self.texrender == 0 {
                self.texrender = sys::gs_texrender_create(
                    sys::gs_color_format_GS_BGRA,
                    sys::gs_zstencil_format_GS_ZS_NONE,
                ) as usize;
                for stage in &mut self.stages {
                    *stage = sys::gs_stagesurface_create(WIDE, TALL, sys::gs_color_format_GS_BGRA)
                        as usize;
                }
            }
            let texrender = self.texrender as *mut sys::gs_texrender_t;
            sys::gs_texrender_reset(texrender);
            if !sys::gs_texrender_begin(texrender, WIDE, TALL) {
                return;
            }
            let clear = crate::vec4(0.0, 0.0, 0.0, 1.0);
            sys::gs_clear(sys::GS_CLEAR_COLOR, &clear, 0.0, 0);
            sys::gs_ortho(0.0, width as f32, 0.0, height as f32, -100.0, 100.0);
            draw();
            sys::gs_texrender_end(texrender);
            let (now, read) = self.turns.next();
            sys::gs_stage_texture(
                self.stages[now] as *mut sys::gs_stagesurf_t,
                sys::gs_texrender_get_texture(texrender),
            );
            let Some(read) = read else { return };
            let stage = self.stages[read] as *mut sys::gs_stagesurf_t;
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut linesize: u32 = 0;
            if !sys::gs_stagesurface_map(stage, &mut data, &mut linesize) || data.is_null() {
                return;
            }
            let row = (WIDE * 4) as usize;
            let mut whole = Vec::with_capacity(row * TALL as usize);
            for y in 0..TALL as usize {
                whole.extend_from_slice(std::slice::from_raw_parts(
                    data.add(y * linesize as usize),
                    row,
                ));
            }
            sys::gs_stagesurface_unmap(stage);
            land(&whole);
        }
    }

    /// Inside the graphics context.
    unsafe fn destroy(&mut self) {
        if self.texrender == 0 {
            return;
        }
        // SAFETY: made in `frame`, inside the graphics context as this is.
        unsafe {
            sys::gs_texrender_destroy(self.texrender as *mut sys::gs_texrender_t);
            for stage in self.stages {
                sys::gs_stagesurface_destroy(stage as *mut sys::gs_stagesurf_t);
            }
        }
        *self = Self::default();
    }
}

pub const WIDE: u32 = 960;
pub const TALL: u32 = 540;

pub struct Ring {
    said: Preview,
    base: *mut u8,
    sequence: AtomicU64,
    /// The last frame, whole, for a shot.
    last: Mutex<Vec<u8>>,
    /// Bumped whenever `last` or a ring alone takes a frame.
    pub landed: Landed,
    scene: Mutex<Readback>,
    watch_on: bool,
    /// The camera and the screen on their own: each rendered into a texture
    /// on libobs's render thread, staged, and copied into its ring.
    camera: Mutex<Alone>,
    screen: Mutex<Alone>,
    /// The staged scene, off the air, the scene a take puts on it.
    staged: Mutex<Alone>,
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
    readback: Readback,
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
            base.add(STAGED_SEQUENCE_AT).cast::<u64>().write(0);
            base
        };
        Ok(Box::new(Self {
            said,
            base,
            sequence: AtomicU64::new(0),
            last: Mutex::new(Vec::new()),
            landed: Landed::default(),
            scene: Mutex::new(Readback::default()),
            watch_on: false,
            camera: Mutex::new(Alone::default()),
            screen: Mutex::new(Alone::default()),
            staged: Mutex::new(Alone::default()),
            render_on: false,
            snap: Mutex::new(None),
        }))
    }

    pub fn said(&self) -> &Preview {
        &self.said
    }

    /// Read the scene back every frame, or stop.
    pub fn watch(&mut self, on: bool) {
        if on == self.watch_on {
            return;
        }
        let me = self as *mut Self as *mut c_void;
        // SAFETY: `self` is boxed and outlives the callback, which is removed
        // in `watch(false)` and in drop before the box goes.
        unsafe {
            if on {
                sys::obs_add_main_rendered_callback(Some(Self::on_rendered), me);
            } else {
                sys::obs_remove_main_rendered_callback(Some(Self::on_rendered), me);
            }
        }
        self.watch_on = on;
        // Asleep, the kept picture is only getting older: forgotten, so a
        // shot wakes the ring for one of now instead.
        if !on {
            if let Ok(mut last) = self.last.lock() {
                last.clear();
            }
            if let Ok(mut scene) = self.scene.lock() {
                scene.turns.forget();
            }
        }
    }

    /// Which source the camera's and the screen's rings show; null for none.
    pub fn alone(&self, camera: *mut sys::obs_source_t, screen: *mut sys::obs_source_t) {
        for (alone, source) in [(&self.camera, camera), (&self.screen, screen)] {
            if let Ok(mut it) = alone.lock() {
                if it.source != source {
                    it.readback.turns.forget();
                }
                it.source = source;
            }
        }
    }

    /// Which source the staged ring shows: the staged scene's own, or null
    /// for none.
    pub fn stage(&self, source: *mut sys::obs_source_t) {
        if let Ok(mut it) = self.staged.lock() {
            if it.source != source {
                it.readback.turns.forget();
                it.last.clear();
            }
            it.source = source;
        }
    }

    /// Render the sources alone every frame, or stop.
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
            for alone in [&self.camera, &self.screen, &self.staged] {
                if let Ok(mut it) = alone.lock() {
                    it.last.clear();
                    it.readback.turns.forget();
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
            (
                &ring.staged,
                Preview::staged_offset as fn(&Preview, u32) -> usize,
                STAGED_SEQUENCE_AT,
            ),
        ] {
            let Ok(mut it) = alone.try_lock() else {
                continue;
            };
            if it.source.is_null() {
                continue;
            }
            let source = it.source;
            // SAFETY: inside the graphics context, as above.
            let (w, h) = unsafe {
                (
                    sys::obs_source_get_width(source),
                    sys::obs_source_get_height(source),
                )
            };
            if w == 0 || h == 0 {
                continue;
            }
            let it = &mut *it;
            // SAFETY: as above.
            unsafe {
                it.readback.frame(
                    w,
                    h,
                    || sys::obs_source_video_render(source),
                    |whole| {
                        let slot = fills(it.sequence, SLOTS);
                        ring.land(offset_of(&ring.said, slot), whole);
                        it.sequence += 1;
                        (*ring.base.add(counts_at).cast::<AtomicU64>())
                            .store(it.sequence, Ordering::Release);
                        it.last = whole.to_vec();
                        ring.landed.bump();
                    },
                );
            }
        }
    }

    /// One frame into the shared memory at `offset`, row by row.
    fn land(&self, offset: usize, whole: &[u8]) {
        let stride = self.said.stride as usize;
        let row = (WIDE * 4) as usize;
        for (y, src) in whole.chunks_exact(row).enumerate() {
            // SAFETY: the slot at `offset` is TALL rows of `stride` bytes
            // inside the mapping made in `new`.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    src.as_ptr(),
                    self.base.add(offset + y * stride),
                    row,
                );
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

    /// The scene, once libobs has rendered it: drawn the way libobs draws
    /// its own preview, scaled on the GPU. Scaled on the CPU instead, by
    /// libobs's raw video callback (swscale, NV12 to BGRA), the scene's ring
    /// alone cost 23 points of a core awake; this way it costs 3 (30 s awake
    /// against 20 asleep, 1080p30, the same scene).
    unsafe extern "C" fn on_rendered(param: *mut c_void) {
        // SAFETY: `param` is the ring `watch` registered; libobs calls this
        // inside its graphics context, after the main texture is rendered.
        unsafe {
            let ring = &*(param as *const Self);
            let Ok(mut scene) = ring.scene.try_lock() else {
                return;
            };
            let texture = sys::obs_get_main_texture();
            if texture.is_null() {
                return;
            }
            scene.frame(
                sys::gs_texture_get_width(texture),
                sys::gs_texture_get_height(texture),
                || sys::obs_render_main_texture(),
                |whole| {
                    let slot = fills(ring.sequence.load(Ordering::Relaxed), SLOTS);
                    ring.land(ring.said.offset(slot), whole);
                    let next = ring.sequence.fetch_add(1, Ordering::Relaxed) + 1;
                    (*ring.base.add(SEQUENCE_AT).cast::<AtomicU64>())
                        .store(next, Ordering::Release);
                    if let Ok(mut last) = ring.last.lock() {
                        *last = whole.to_vec();
                    }
                    ring.landed.bump();
                },
            );
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
            for alone in [&self.camera, &self.screen, &self.staged] {
                if let Ok(mut it) = alone.lock() {
                    it.readback.destroy();
                }
            }
            if let Ok(mut scene) = self.scene.lock() {
                scene.destroy();
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
    use std::time::{Duration, Instant};

    use super::{Landed, Turns};

    #[test]
    fn a_frame_landing_wakes_the_shot_waiting_for_it() {
        let landed = std::sync::Arc::new(Landed::default());
        let seen = landed.count();
        let until = Instant::now() + Duration::from_secs(10);
        let waiter = {
            let landed = landed.clone();
            std::thread::spawn(move || landed.after(seen, until))
        };
        while landed.waiting() == 0 {
            std::thread::yield_now();
        }
        landed.bump();
        assert_eq!(waiter.join().unwrap(), Some(seen + 1));
        assert!(
            Instant::now() < until,
            "woken by the frame, not the deadline"
        );
    }

    #[test]
    fn a_frame_landed_before_the_wait_is_not_waited_for() {
        let landed = Landed::default();
        let seen = landed.count();
        landed.bump();
        assert_eq!(landed.after(seen, Instant::now()), Some(seen + 1));
    }

    #[test]
    fn no_frame_landing_is_given_up_at_the_deadline() {
        let landed = Landed::default();
        assert_eq!(landed.after(landed.count(), Instant::now()), None);
    }

    #[test]
    fn the_first_frame_after_waking_is_read_at_once_from_where_it_was_staged() {
        let mut turns = Turns::default();
        assert_eq!(turns.next(), (0, Some(0)));
    }

    #[test]
    fn then_each_frame_reads_the_one_staged_a_frame_before() {
        let mut turns = Turns::default();
        turns.next();
        assert_eq!(turns.next(), (1, None), "the first was read already");
        assert_eq!(turns.next(), (0, Some(1)));
        assert_eq!(turns.next(), (1, Some(0)));
        assert_eq!(turns.next(), (0, Some(1)));
    }

    #[test]
    fn a_ring_woken_again_never_reads_what_it_staged_before_it_slept() {
        let mut turns = Turns::default();
        turns.next();
        turns.next();
        turns.next();
        turns.forget();
        assert_eq!(turns.next(), (0, Some(0)));
        assert_eq!(turns.next(), (1, None));
    }

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
