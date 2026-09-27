//! The noise gate's detector, as pure arithmetic over 128-sample blocks.
//!
//! Ported one for one from `assets/js/studio/gate.ts`, tests included, because
//! the studio page and the engine have to sound the same. The comments there
//! are the record of what was calibrated by ear; they are kept here rather
//! than summarised, because the numbers mean nothing without them.
//!
//! A proximity filter by level: loud, near sounds (a voice, a mechanical
//! keyboard) open the gate; distant voices and room noise never reach the
//! threshold and stay attenuated. Opens fast, closes slow, so it does not
//! pump. Two detectors, OR-ed:
//!
//! * HIGH band (>3 kHz): walls muffle highs, so speech from outside barely
//!   registers here, while consonants and the keyboard next to the mic do.
//! * FULL band: a close mic with low physical gain makes YOUR voice loud in
//!   every band, vowels included, and distant voices quiet. This is what keeps
//!   soft, fast syllables from being chopped, because vowels live below 1 kHz.
//!
//! Defaults assume a cardioid mic with its front face at the mouth, the
//! keyboard roughly behind it, and the room far and muffled. Calibrated by ear
//! with a USB condenser at close to minimum physical gain.
//!
//! Everything is computed in `f64` even though the samples are `f32`. That is
//! not an accident: JavaScript widens every `Float32Array` read to a double,
//! so accumulating in `f32` here would drift away from the studio's numbers
//! and the ported tests would stop meaning the same thing.

/// How long each detector has to agree before the gate believes it.
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct GateParams {
    /// ≈ -55 dB on the >3 kHz band: keyboard clicks from behind the mic.
    pub hf: f64,
    /// ≈ -20 dBFS full band: your voice, close and on-axis, gain low.
    pub full: f64,
    /// A closed gate sits at ≈ -40 dB: the room is gone, but it is not a vacuum.
    pub floor: f64,
    /// Stays open between syllables and words.
    pub hold_ms: f64,
    /// The full band must persist this long, so an impulse does not count.
    pub attack_ms: f64,
    /// The highs must persist this long. A key press does.
    pub hf_attack_ms: f64,
    /// ≈ +6 dB when the gate opened on highs only: a keyboard, with no voice.
    pub keys_boost: f64,
}

impl Default for GateParams {
    fn default() -> Self {
        Self {
            hf: 0.0018,
            full: 0.1,
            floor: 0.01,
            hold_ms: 450.0,
            attack_ms: 50.0,
            hf_attack_ms: 12.0,
            keys_boost: 2.0,
        }
    }
}

/// What the gate decided for this block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateFrame {
    pub gain: f64,
    pub open: bool,
    /// Whether the full band opened it: a voice, not a keyboard. The duck
    /// follows this and never `open`, or typing chops the music.
    pub voice: bool,
}

/// What the two detectors are hearing, for the meter that says where to put
/// the thresholds. A gate is set by eye, against your room and your voice.
#[derive(
    Default,
    Debug,
    Clone,
    Copy,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
pub struct GateLevels {
    pub full: f64,
    pub hf: f64,
}

/// A change to some of the parameters. Every field optional on purpose: the
/// panel sends what the person touched, not the whole set.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GatePatch {
    pub hf: Option<f64>,
    pub full: Option<f64>,
    pub floor: Option<f64>,
    pub hold_ms: Option<f64>,
    pub attack_ms: Option<f64>,
    pub hf_attack_ms: Option<f64>,
    pub keys_boost: Option<f64>,
}

pub const BLOCK: usize = 128;

/// How far the sound runs behind its detector: the studio's 60 ms, which the
/// defaults were tuned with. Longer than the attack, so a word from silence
/// is heard whole.
pub const LOOKAHEAD_MS: f64 = 60.0;

/// Milliseconds as a count of frames at this rate.
pub fn frames_for(ms: f64, sample_rate: f64) -> usize {
    (ms * sample_rate / 1000.0).round() as usize
}

/// Milliseconds as a count of 128-sample blocks at this rate.
pub fn blocks_for(ms: f64, sample_rate: f64) -> u32 {
    // JavaScript's Math.round is half-up, Rust's f64::round is half-away-from
    // zero. Identical for the positive values this is ever called with.
    (ms * sample_rate / 1000.0 / BLOCK as f64).round() as u32
}

pub struct GateDetector {
    sample_rate: f64,
    /// One-pole high pass at 3 kHz. Detector only: the audio never goes
    /// through it, so nothing is filtered out of what the audience hears.
    hp_a: f64,
    params: GateParams,
    hold_blocks: u32,
    attack_blocks: u32,
    hf_attack_blocks: u32,
    env_hf: f64,
    env_full: f64,
    gain: f64,
    hold: u32,
    run: u32,
    run_hf: u32,
    mode: f64,
    /// Whether the full band was what opened it, kept through the hold so
    /// the duck sees a voice and not a keyboard. See [`GateFrame::voice`].
    by_voice: bool,
    hp_x: f64,
    hp_y: f64,
}

impl GateDetector {
    pub fn new(sample_rate: f64, params: GateParams) -> Self {
        let mut gate = Self {
            sample_rate,
            hp_a: 1.0 / (1.0 + (2.0 * std::f64::consts::PI * 3000.0) / sample_rate),
            params,
            hold_blocks: 0,
            attack_blocks: 0,
            hf_attack_blocks: 0,
            env_hf: 0.0,
            env_full: 0.0,
            gain: 1.0,
            hold: 0,
            run: 0,
            run_hf: 0,
            mode: 1.0,
            by_voice: false,
            hp_x: 0.0,
            hp_y: 0.0,
        };
        gate.retime();
        gate
    }

    pub fn params(&self) -> GateParams {
        self.params
    }

    pub fn levels(&self) -> GateLevels {
        GateLevels {
            full: self.env_full,
            hf: self.env_hf,
        }
    }

    /// The whole set at once, which is what the engine holds and hands over.
    /// [`GateDetector::update`] is the other half: a patch, which is what a
    /// panel sends when one slider moves.
    pub fn retune(&mut self, params: GateParams) {
        self.params = params;
        self.retime();
    }

    /// Live tuning from the panel: no reload, the next block uses the new
    /// values.
    pub fn update(&mut self, patch: GatePatch) {
        let p = &mut self.params;
        if let Some(v) = patch.hf {
            p.hf = v
        }
        if let Some(v) = patch.full {
            p.full = v
        }
        if let Some(v) = patch.floor {
            p.floor = v
        }
        if let Some(v) = patch.hold_ms {
            p.hold_ms = v
        }
        if let Some(v) = patch.attack_ms {
            p.attack_ms = v
        }
        if let Some(v) = patch.hf_attack_ms {
            p.hf_attack_ms = v
        }
        if let Some(v) = patch.keys_boost {
            p.keys_boost = v
        }
        self.retime();
    }

    pub fn step(&mut self, block: &[f32]) -> GateFrame {
        let n = block.len();
        let p = self.params;
        if n == 0 {
            return GateFrame {
                gain: self.gain,
                open: self.hold > 0,
                voice: self.hold > 0 && self.by_voice,
            };
        }
        let mut sum_hf = 0.0;
        let mut sum_full = 0.0;
        for sample in block {
            let x = *sample as f64;
            let y = self.hp_a * (self.hp_y + x - self.hp_x);
            self.hp_x = x;
            self.hp_y = y;
            sum_hf += y * y;
            sum_full += x * x;
        }
        let n = n as f64;
        // Fast decay, so impulses die quickly.
        self.env_hf = (sum_hf / n).sqrt().max(self.env_hf * 0.9);
        self.env_full = (sum_full / n).sqrt().max(self.env_full * 0.85);
        // The full band must be above the threshold for the attack time before
        // it counts as a voice: a slap or a chair knock is over long before
        // that. The count leaks rather than resets, because a voice dips under
        // the threshold at every closure ("teste" has two). Demanding an
        // unbroken run meant a voice a little further from the mic never got
        // there at all, and the gate ate whole words. An impulse still dies:
        // it gains one and loses one.
        self.run = if self.env_full > p.full {
            (self.run + 1).min(self.attack_blocks)
        } else {
            self.run.saturating_sub(1)
        };
        self.run_hf = if self.env_hf > p.hf {
            (self.run_hf + 1).min(self.hf_attack_blocks)
        } else {
            self.run_hf.saturating_sub(1)
        };
        let voice = self.run >= self.attack_blocks;
        let keys = self.run_hf >= self.hf_attack_blocks;
        let open = voice || keys;
        // The keys boost is for a click with nobody talking. The highs cross
        // their threshold long before the full band finishes its attack, so
        // deciding by `voice` alone rode every word up by the boost and slid
        // it back down over the release: the voice pumped. Any real voice
        // energy in the full band means it is you, so the boost is off.
        if open {
            // A voice, whatever its level. The full band over its threshold
            // is one; so is real energy under the highs; and so is a signal
            // whose energy sits mostly below 3 kHz, which is every voice and
            // no keyboard click: a quiet voice on a quiet headset opens the
            // gate through its sibilants alone, and by level alone it read as
            // keys, was boosted like a click, and never ducked the music.
            let heard = voice || self.env_full > p.full / 2.0 || self.env_full > 4.0 * self.env_hf;
            // And a voice stays one until the gate closes: a breath or an
            // "s" between two words reads as keys by itself, and it set the
            // boost the next vowel came in on, and let the duck go under it.
            self.by_voice = heard || (self.by_voice && self.hold > 0);
            self.hold = self.hold_blocks;
            self.mode = if self.by_voice { 1.0 } else { p.keys_boost };
        } else if self.hold > 0 {
            self.hold -= 1;
        }
        let target = if open || self.hold > 0 {
            self.mode
        } else {
            p.floor
        };
        // Fast attack, roughly 220 ms release.
        let rate = if target > self.gain { 0.6 } else { 0.012 };
        self.gain += (target - self.gain) * rate;
        GateFrame {
            gain: self.gain,
            open: target != p.floor,
            voice: target != p.floor && self.by_voice,
        }
    }

    fn retime(&mut self) {
        self.hold_blocks = blocks_for(self.params.hold_ms, self.sample_rate);
        self.attack_blocks = blocks_for(self.params.attack_ms, self.sample_rate);
        self.hf_attack_blocks = blocks_for(self.params.hf_attack_ms, self.sample_rate);
    }
}

/// The gate over the sound as a microphone delivers it: interleaved, in
/// callbacks of whatever size the system likes. The framing lives here and
/// not in the adapter, because it is where the gate's time comes from.
pub struct Gated {
    detector: GateDetector,
    channels: usize,
    lookahead_frames: usize,
    /// The sound, `lookahead_frames` behind the detector: one line over the
    /// interleaved samples, so the channels stay paired. None for no delay.
    line: Option<DelayLine>,
    /// Samples that did not fill a whole block, kept for the next push:
    /// dropping the remainder would be dropping audio.
    spare: Vec<f32>,
}

/// One block through the gate.
#[derive(Debug, Clone, PartialEq)]
pub struct GatedBlock {
    pub frame: GateFrame,
    pub levels: GateLevels,
    /// The block, interleaved, at the gate's gain.
    pub samples: Vec<f32>,
}

impl Gated {
    pub fn new(detector: GateDetector, channels: usize, lookahead_frames: usize) -> Self {
        Self {
            detector,
            channels: channels.max(1),
            lookahead_frames,
            line: (lookahead_frames > 0)
                .then(|| DelayLine::new(lookahead_frames * channels.max(1))),
            spare: Vec::new(),
        }
    }

    pub fn retune(&mut self, params: GateParams) {
        self.detector.retune(params);
    }

    /// How long before the first sample of the next push the next sample
    /// out was heard, in frames: what waits for a whole block, and the
    /// lookahead. The wire is stamped from the moment a sound was heard, so
    /// a stamp that forgot the lookahead would put the voice that much late
    /// against the lips.
    pub fn held_frames(&self) -> usize {
        self.spare.len() / self.channels + self.lookahead_frames
    }

    /// Whole blocks of [`BLOCK`] frames, as the studio's worklet has them:
    /// the detector hears the first channel, every channel takes its gain.
    /// It was fed 128 interleaved samples, which is 64 frames of stereo,
    /// against a detector that counts 128: every time on the panel ran at
    /// half, and its high pass, running across L,R,L,R, read the highs
    /// 5 dB low.
    pub fn push(&mut self, samples: &[f32]) -> Vec<GatedBlock> {
        self.spare.extend_from_slice(samples);
        let size = BLOCK * self.channels;
        let mut blocks = Vec::new();
        while self.spare.len() >= size {
            let block: Vec<f32> = self.spare.drain(..size).collect();
            let first: Vec<f32> = block.iter().step_by(self.channels).copied().collect();
            let frame = self.detector.step(&first);
            let samples = match self.line.as_mut() {
                Some(line) => {
                    let mut out = vec![0.0; block.len()];
                    line.process(&block, &mut out, frame.gain);
                    out
                }
                None => block
                    .iter()
                    .map(|s| (*s as f64 * frame.gain) as f32)
                    .collect(),
            };
            blocks.push(GatedBlock {
                frame,
                levels: self.detector.levels(),
                samples,
            });
        }
        blocks
    }
}

/// Lookahead. The audio path is delayed and the detector is not, so the gate
/// opens *before* the sound that opened it reaches the output, and word onsets
/// survive the attack wait intact. One line per channel.
pub struct DelayLine {
    ring: Vec<f32>,
    write: usize,
}

impl DelayLine {
    pub fn new(samples: usize) -> Self {
        Self {
            ring: vec![0.0; samples],
            write: 0,
        }
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32], gain: f64) {
        let look_ahead = self.ring.len();
        for (i, sample) in input.iter().enumerate() {
            let idx = (self.write + i) % look_ahead;
            output[i] = (self.ring[idx] as f64 * gain) as f32; // the delayed sample, gated
            self.ring[idx] = *sample; // store the fresh one
        }
        self.write = (self.write + input.len()) % look_ahead;
    }
}

/// The boundary with whatever is driving us: a socket message is untrusted, so
/// only the numeric gate keys survive it.
pub fn parse_gate_params(raw: &serde_json::Value) -> GatePatch {
    let mut patch = GatePatch::default();
    let Some(object) = raw.as_object() else {
        return patch;
    };
    let take = |key: &str| object.get(key).and_then(serde_json::Value::as_f64);
    // Two spellings: the status's own, which the panel and the CLI send back,
    // and the studio page's camelCase, which came first. Only the first
    // was read for a while, and a slider for a field with an underscore in
    // its name was accepted, ignored, and back where it was a second later.
    let either = |snake: &str, camel: &str| take(snake).or_else(|| take(camel));
    patch.hf = take("hf");
    patch.full = take("full");
    patch.floor = take("floor");
    patch.hold_ms = either("hold_ms", "holdMs");
    patch.attack_ms = either("attack_ms", "attackMs");
    patch.hf_attack_ms = either("hf_attack_ms", "hfAttackMs");
    patch.keys_boost = either("keys_boost", "keysBoost");
    patch
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    // The wire spells the fields the way the status does, attack_ms and
    // keys_boost; the parser only knew the studio page's attackMs and
    // keysBoost, so a panel's Attack and Keys boost sliders were accepted and
    // ignored, and snapped back a second later. Both spellings are taken.
    #[test]
    fn a_patch_is_read_in_the_status_s_own_spelling_as_well_as_the_page_s() {
        let snake = parse_gate_params(&serde_json::json!({
            "attack_ms": 80.0, "keys_boost": 3.0, "hold_ms": 300.0, "hf_attack_ms": 20.0
        }));
        assert_eq!(snake.attack_ms, Some(80.0));
        assert_eq!(snake.keys_boost, Some(3.0));
        assert_eq!(snake.hold_ms, Some(300.0));
        assert_eq!(snake.hf_attack_ms, Some(20.0));
        let camel = parse_gate_params(&serde_json::json!({ "attackMs": 80.0, "keysBoost": 3.0 }));
        assert_eq!(camel.attack_ms, Some(80.0));
        assert_eq!(camel.keys_boost, Some(3.0));
    }

    /// A phase-continuous sine, sliced into 128-sample blocks.
    fn tone(hz: f64, amplitude: f64, blocks: usize) -> Vec<Vec<f32>> {
        (0..blocks)
            .map(|b| {
                (0..BLOCK)
                    .map(|i| {
                        let t = (b * BLOCK + i) as f64;
                        (amplitude * (2.0 * std::f64::consts::PI * hz * t / SR).sin()) as f32
                    })
                    .collect()
            })
            .collect()
    }

    fn silence(blocks: usize) -> Vec<Vec<f32>> {
        tone(0.0, 0.0, blocks)
    }

    /// A voice the way a voice is: above the threshold, with the short closure
    /// every syllable has (a "t", a "p", the gap between words). Nobody
    /// sustains an unbroken level for 50 ms.
    fn syllables(hz: f64, amplitude: f64, blocks: usize) -> Vec<Vec<f32>> {
        (0..blocks)
            .map(|b| {
                let closure = b % 17 < 3; // ~8 ms of near-silence every ~45 ms
                (0..BLOCK)
                    .map(|i| {
                        let t = (b * BLOCK + i) as f64;
                        let v = amplitude * (2.0 * std::f64::consts::PI * hz * t / SR).sin();
                        (if closure { v * 0.03 } else { v }) as f32
                    })
                    .collect()
            })
            .collect()
    }

    fn run(gate: &mut GateDetector, blocks: &[Vec<f32>]) -> GateFrame {
        let mut last = GateFrame {
            gain: f64::NAN,
            open: false,
            voice: false,
        };
        for block in blocks {
            last = gate.step(block);
        }
        last
    }

    fn gate_with(patch: GatePatch) -> GateDetector {
        let mut gate = GateDetector::new(SR, GateParams::default());
        gate.update(patch);
        gate
    }

    #[test]
    fn blocks_for_turns_milliseconds_into_blocks_at_the_sample_rate() {
        assert_eq!(blocks_for(50.0, SR), 19);
        assert_eq!(blocks_for(12.0, SR), 5);
        assert_eq!(blocks_for(450.0, SR), 169);
    }

    #[test]
    fn silence_closes_the_gate_down_to_the_floor() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        let GateFrame { gain, open, .. } = run(&mut gate, &silence(1000));
        assert!(!open);
        let floor = GateParams::default().floor;
        assert!(gain < 0.011 && gain >= floor, "gain {gain}");
    }

    #[test]
    fn a_loud_low_voice_opens_the_gate_after_the_attack_time() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        run(&mut gate, &silence(600));
        assert!(
            run(
                &mut gate,
                &tone(200.0, 0.5, blocks_for(50.0, SR) as usize - 1)
            )
            .open,
            "the highs open it first"
        );
        let GateFrame { gain, open, .. } = run(&mut gate, &tone(200.0, 0.5, 30));
        assert!(open);
        assert!(gain > 0.9, "gain {gain}");
    }

    #[test]
    fn highs_alone_the_keyboard_open_the_gate_with_the_keys_boost() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        run(&mut gate, &silence(600));
        let GateFrame { gain, open, voice } = run(&mut gate, &tone(5000.0, 0.02, 80));
        assert!(open);
        assert!(!voice, "a keyboard is not a voice, and the duck must know");
        let boost = GateParams::default().keys_boost;
        assert!(gain > 1.9 && gain <= boost, "gain {gain}");
    }

    // A quiet voice on a quiet microphone: the full band never reaches the
    // opening threshold, the sibilants open the gate through the highs, and
    // the mixer heard "keys". A Bluetooth headset speaks at this level, and
    // its owner had to shout to duck the music. A voice carries most of its
    // energy under 3 kHz; a keyboard click carries most of its above. That
    // ratio does not care how loud either is.
    fn quiet_voice(blocks: usize) -> Vec<Vec<f32>> {
        let low = tone(200.0, 0.03, blocks);
        let sibilance = tone(6000.0, 0.005, blocks);
        low.iter()
            .zip(&sibilance)
            .map(|(a, b)| a.iter().zip(b).map(|(x, y)| x + y).collect())
            .collect()
    }

    #[test]
    fn a_quiet_voice_with_its_sibilants_is_a_voice_not_a_keyboard() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        run(&mut gate, &silence(600));
        let frame = run(&mut gate, &quiet_voice(80));
        assert!(frame.open, "the highs open it");
        assert!(frame.voice, "and it is a voice, whatever its level");
        assert!(frame.gain <= 1.0, "so it is not boosted like a click");
    }

    #[test]
    fn a_voice_is_a_voice_to_the_duck() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        run(&mut gate, &silence(600));
        let frame = run(&mut gate, &tone(200.0, 0.5, 80));
        assert!(frame.open && frame.voice);
    }

    #[test]
    fn an_impulse_shorter_than_the_attack_never_opens_the_full_band() {
        // Highs off: this is the voice detector alone.
        let mut gate = gate_with(GatePatch {
            hf: Some(1.0),
            ..Default::default()
        });
        run(&mut gate, &silence(600));
        let mut blocks = tone(200.0, 0.9, 3);
        blocks.extend(silence(400));
        for block in &blocks {
            assert!(!gate.step(block).open);
        }
    }

    #[test]
    fn a_voice_with_the_closures_every_voice_has_opens_the_voice_path() {
        let mut gate = gate_with(GatePatch {
            hf: Some(1.0),
            ..Default::default()
        });
        run(&mut gate, &silence(600));
        // ~320 ms of speech
        let GateFrame { gain, open, .. } = run(&mut gate, &syllables(200.0, 0.5, 120));
        assert!(open, "a real voice must reach the full-band detector");
        assert!(gain > 0.9, "gain {gain}");
    }

    #[test]
    fn a_voice_is_not_mistaken_for_a_keyboard_it_comes_out_at_its_own_level() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        run(&mut gate, &silence(600));
        let GateFrame { gain, .. } = run(&mut gate, &syllables(200.0, 0.5, 120));
        assert!(
            gain <= 1.05,
            "the voice left {gain:.2}x louder (+{:.1} dB): the keys boost rode it",
            20.0 * gain.log10()
        );
    }

    // A word, then a breath or an "s" with little low end, then the next
    // vowel. The hiss alone reads as keys; it set the mode to the keys boost
    // and the hold carried it, so the vowel came in at +6 dB and the voice
    // flag dropped under it, letting the music swell back mid-sentence.
    #[test]
    fn a_hiss_after_a_word_is_still_the_voice_and_does_not_ride_the_next_vowel_up() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        run(&mut gate, &silence(600));
        run(&mut gate, &syllables(200.0, 0.3, 120));
        let hiss = run(&mut gate, &tone(6000.0, 0.005, 25));
        assert!(hiss.voice, "the duck must not let go between two words");
        let vowel = run(&mut gate, &syllables(200.0, 0.3, 1));
        assert!(
            vowel.gain <= 1.05,
            "the vowel came in {:.1} dB up",
            20.0 * vowel.gain.log10()
        );
    }

    #[test]
    fn a_voice_near_the_threshold_stays_open_instead_of_chattering() {
        let mut gate = gate_with(GatePatch {
            hf: Some(1.0),
            ..Default::default()
        });
        run(&mut gate, &silence(600));
        // Speaking from a little further away: the level sits around the
        // threshold, dipping under it every syllable the way a voice does.
        let speech = syllables(200.0, 0.16, 300);
        let attack = blocks_for(GateParams::default().attack_ms, SR) as usize + 8;
        for block in &speech[..attack] {
            gate.step(block);
        }
        let closed = speech[attack..]
            .iter()
            .filter(|b| !gate.step(b).open)
            .count();
        assert_eq!(
            closed,
            0,
            "once open, the gate shut again for {closed} blocks ({} ms of speech eaten)",
            (closed * BLOCK * 1000) as f64 / SR
        );
    }

    #[test]
    fn the_hold_keeps_the_gate_open_between_words_then_it_closes() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        run(&mut gate, &silence(600));
        run(&mut gate, &tone(200.0, 0.5, 40));
        assert!(run(&mut gate, &silence(100)).open);
        assert!(!run(&mut gate, &silence(200)).open);
    }

    // The engine hands the gate its whole set with retune, which replaced
    // the numbers and not the times counted from them: a Hold or an Attack
    // moved on the panel was on the status and never in the gate.
    #[test]
    fn retune_takes_the_times_too() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        gate.retune(GateParams {
            hold_ms: 100.0,
            ..GateParams::default()
        });
        run(&mut gate, &silence(600));
        run(&mut gate, &tone(200.0, 0.5, 40));
        let closed_after = (0..400)
            .position(|_| !gate.step(&silence(1)[0]).open)
            .expect("it closes");
        let ms = (closed_after * BLOCK) as f64 * 1000.0 / SR;
        assert!(
            ms < 200.0,
            "closed {ms:.0} ms after the voice, the hold is 100"
        );
    }

    #[test]
    fn update_retunes_the_thresholds_live() {
        let mut gate = GateDetector::new(SR, GateParams::default());
        run(&mut gate, &silence(600));
        gate.update(GatePatch {
            hf: Some(1.0),
            full: Some(1.0),
            ..Default::default()
        });
        assert!(!run(&mut gate, &tone(200.0, 0.5, 60)).open);
        gate.update(GatePatch {
            full: Some(GateParams::default().full),
            ..Default::default()
        });
        assert!(run(&mut gate, &tone(200.0, 0.5, 60)).open);
    }

    /// Mono blocks as a microphone delivers them: stereo, L = R, interleaved,
    /// in one run the caller slices as it likes.
    fn stereo(blocks: &[Vec<f32>]) -> Vec<f32> {
        blocks.iter().flatten().flat_map(|s| [*s, *s]).collect()
    }

    /// Frames from the first sample of `after` to the first block the gate
    /// reports closed, pushed in callbacks of an odd size, as CoreAudio does.
    fn frames_open_into(gated: &mut Gated, before: &[f32], after: &[f32]) -> usize {
        for chunk in before.chunks(941 * 2) {
            gated.push(chunk);
        }
        let mut frames = 0;
        for chunk in after.chunks(941 * 2) {
            for block in gated.push(chunk) {
                if !block.frame.open {
                    return frames;
                }
                frames += block.samples.len() / 2;
            }
        }
        frames
    }

    // The engine gated 128 interleaved stereo samples at a time, which is 64
    // frames, while the detector counts its blocks as 128: every time on the
    // panel ran at half, the hold of 450 ms closed at about 245 and took the
    // ends of words with it. The studio steps on one channel of 128 frames.
    #[test]
    fn interleaved_stereo_holds_the_gate_as_long_as_the_panel_says() {
        let mut gated = Gated::new(GateDetector::new(SR, GateParams::default()), 2, 0);
        let held = frames_open_into(
            &mut gated,
            &stereo(&[silence(600), tone(200.0, 0.5, 375)].concat()),
            &stereo(&silence(1000)),
        );
        let hold = (GateParams::default().hold_ms * SR / 1000.0) as usize;
        assert!(
            held >= hold,
            "closed {:.0} ms after the voice stopped, the hold is {} ms",
            held as f64 * 1000.0 / SR,
            GateParams::default().hold_ms
        );
    }

    #[test]
    fn interleaved_stereo_reads_the_highs_as_one_channel_does() {
        let mut mono = GateDetector::new(SR, GateParams::default());
        run(&mut mono, &tone(1000.0, 0.1, 200));
        let mut gated = Gated::new(GateDetector::new(SR, GateParams::default()), 2, 0);
        let last = gated
            .push(&stereo(&tone(1000.0, 0.1, 200)))
            .pop()
            .expect("blocks");
        let apart = 20.0 * (last.levels.hf / mono.levels().hf).log10();
        assert!(apart.abs() < 0.5, "the highs read {apart:.1} dB off");
    }

    /// The output for a word that starts from silence, through a gate with
    /// this much lookahead, lined up with its input: (input, output) pairs of
    /// the first channel over the word's first `ms`.
    fn onset_through(lookahead_frames: usize, ms: f64) -> Vec<(f32, f32)> {
        let mut gated = Gated::new(
            GateDetector::new(SR, GateParams::default()),
            2,
            lookahead_frames,
        );
        let quiet = 600 * BLOCK;
        // A word that never repeats itself, so a sample lined up with the
        // wrong one cannot pass for it: a glide from 200 to 500 Hz, rising
        // over 5 ms the way a voice does, not with a click the highs hear.
        let word: Vec<f32> = (0..400 * BLOCK)
            .map(|n| {
                let t = n as f64 / SR;
                let rise = (t / 0.005).min(1.0);
                let phase = 2.0 * std::f64::consts::PI * (200.0 * t + 300.0 * t * t / 2.0);
                (0.2 * rise * phase.sin()) as f32
            })
            .collect();
        let input: Vec<f32> = [vec![0.0; quiet], word]
            .concat()
            .iter()
            .flat_map(|s| [*s, *s])
            .collect();
        let output: Vec<f32> = input
            .chunks(941 * 2)
            .flat_map(|chunk| gated.push(chunk))
            .flat_map(|block| block.samples)
            .collect();
        let frames = (ms * SR / 1000.0) as usize;
        (quiet..quiet + frames)
            .map(|n| (input[2 * n], output[2 * (n + lookahead_frames)]))
            .collect()
    }

    // The engine gated the block it had just measured, with no delay: a word
    // from silence waited the attack at the floor, -40 dB, and its first
    // syllable was gone (a median 19 ms in a recording, 45 at worst). The
    // gate runs 60 ms behind its detector for that.
    #[test]
    fn a_word_from_silence_keeps_its_onset() {
        let lookahead = frames_for(LOOKAHEAD_MS, SR);
        let eaten = onset_through(lookahead, 20.0)
            .iter()
            .filter(|(input, output)| input.abs() > 0.05 && (output / input) < 0.89)
            .count();
        assert_eq!(eaten, 0, "{eaten} samples of the onset under -1 dB");
        // The highs open a loud word after their own attack, 13 ms; before
        // that, with no lookahead, it is at the floor.
        let without = onset_through(0, 10.0);
        assert!(
            without
                .iter()
                .all(|(input, output)| (output / input).abs() < 0.5 || input.abs() <= 0.05),
            "the negative control: with no lookahead the onset is eaten"
        );
    }

    // The wire is stamped from the moment a sound was heard: what leaves the
    // gate next is older than the next callback by what it holds back, the
    // remainder of a block and the lookahead. Forgetting the second puts the
    // voice 60 ms late against the lips.
    #[test]
    fn what_the_gate_holds_back_counts_the_lookahead() {
        let lookahead = frames_for(LOOKAHEAD_MS, SR);
        let mut gated = Gated::new(GateDetector::new(SR, GateParams::default()), 2, lookahead);
        assert_eq!(gated.held_frames(), lookahead);
        gated.push(&vec![0.0; (BLOCK + 40) * 2]);
        assert_eq!(gated.held_frames(), lookahead + 40);
    }

    #[test]
    fn delay_line_plays_the_input_back_after_the_lookahead_scaled_by_the_gain() {
        let mut line = DelayLine::new(2 * BLOCK);
        let mut impulse = vec![0.0f32; BLOCK];
        impulse[0] = 1.0;
        let mut out = vec![0.0f32; BLOCK];
        line.process(&impulse, &mut out, 1.0);
        assert_eq!(out[0], 0.0);
        line.process(&vec![0.0; BLOCK], &mut out, 1.0);
        assert_eq!(out[0], 0.0);
        line.process(&vec![0.0; BLOCK], &mut out, 0.5);
        assert_eq!(out[0], 0.5);
    }

    #[test]
    fn parse_gate_params_keeps_only_the_numeric_keys_from_an_untrusted_message() {
        let patch = parse_gate_params(&serde_json::json!({
            "hf": 0.5, "full": "loud", "bogus": 1, "holdMs": 300
        }));
        assert_eq!(
            patch,
            GatePatch {
                hf: Some(0.5),
                hold_ms: Some(300.0),
                ..Default::default()
            }
        );
        assert_eq!(
            parse_gate_params(&serde_json::Value::Null),
            GatePatch::default()
        );
        assert_eq!(
            parse_gate_params(&serde_json::json!("nope")),
            GatePatch::default()
        );
    }
}
