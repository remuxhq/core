//! Audio filtering: the contract every motor hosts, and remux's filters.
//!
//! A filter takes the sound as the host has it: `f32` samples, interleaved,
//! at the host's sample rate and channel count, in buffers of whatever size
//! the host likes. It gives back as many samples as it was given, in place,
//! [`Filter::latency_frames`] late, and the host takes that latency back from
//! the source's timing so the sound stays on the picture. A motor hosts a
//! filter once, in whatever its media stack calls an audio filter (libobs's
//! `filter_audio`, a native callback), and every filter here runs in it.
//!
//! Everything about filtering sound is in this crate, the way everything
//! about filtering a picture is in `remux-shader`: the motor hands it samples
//! and gets samples back.

pub mod gate;

/// What a motor hosts.
pub trait Filter: Send {
    /// Filter `samples`, interleaved, in place.
    fn process(&mut self, samples: &mut [f32]);

    /// How late the sound comes out, in frames.
    fn latency_frames(&self) -> usize;
}
