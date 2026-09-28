//! remux's sound filters, as adapters of the domain's contract
//! (`remuxd_domain::sound::mixer`): the gate first. The daemon injects them
//! into a motor, which hosts them; nothing else of ours depends on this crate.
//!
//! Everything about filtering sound is here, the way everything about
//! compiling a picture's filter is in `remux-shader`.

pub mod gate;
