//! Everything the engine decides, with nothing plugged in.
//!
//! Split out of `remuxd` on purpose. The rule that the domain never touches an
//! Apple framework used to be a comment; now it is `Cargo.toml`, which cannot
//! be talked around. It also means this crate's coverage number is about
//! decisions rather than about glue, so a gate on it says something true.

pub mod air;
pub mod app;
pub mod bench;
pub mod bug;
pub mod companions;
pub mod config;
pub mod daemon;
pub mod engine;
pub mod health;
pub mod log;
pub mod os;
pub mod picture;
pub mod protocol;
pub mod remembered;
pub mod socket;
pub mod sound;
pub mod wait;
