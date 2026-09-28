//! Sound filters: the contract every motor hosts, the gate's side of it the
//! domain keeps (its settings, what it decides, what it hears), and the
//! meters' scale. The filters themselves are adapters the daemon injects into
//! a motor (`remux-mixer`); the domain never runs one.

pub mod gate;
pub mod levels;

use gate::{GateFrame, GateLevels, GateParams};

/// What a motor hosts: the sound as the host has it, `f32` samples
/// interleaved at the host's sample rate and channel count, in buffers of
/// whatever size the host likes. The same number of samples comes back, in
/// place, [`Filter::latency_frames`] late, and the host takes that latency
/// back from the source's timing so the sound stays on the picture.
pub trait Filter: Send {
    /// Filter `samples`, interleaved, in place.
    fn process(&mut self, samples: &mut [f32]);

    /// How late the sound comes out, in frames.
    fn latency_frames(&self) -> usize;
}

/// The gate a motor hosts on a microphone: a [`Filter`] that also says what
/// it last decided and heard, for the status.
pub trait GateFilter: Filter {
    fn heard(&self) -> Option<(GateFrame, GateLevels)>;
}

/// How a motor is given its gate: made once per microphone, at the host's
/// sample rate and channel count, with the settings of the moment.
pub type MakeGate = Box<dyn Fn(f64, usize, GateParams) -> Box<dyn GateFilter> + Send + Sync>;
