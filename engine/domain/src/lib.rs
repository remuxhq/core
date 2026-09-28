//! Everything the engine decides, with nothing plugged in.
//!
//! Split out of `remuxd` on purpose. The rule that the domain never touches an
//! Apple framework used to be a comment; now it is `Cargo.toml`, which cannot
//! be talked around. It also means this crate's coverage number is about
//! decisions rather than about glue, so a gate on it says something true.

pub mod audio_layers;
pub mod bug;
pub mod camera;
pub mod chat;
pub mod cli;
pub mod clips;
pub mod config;
pub mod daemon;
pub mod destinations;
pub mod engine;
pub mod gate;
pub mod health;
pub mod history;
pub mod journal;
pub mod layers;
pub mod levels;
pub mod log;
pub mod login;
pub mod music;
pub mod os;
pub mod plan;
pub mod preview;
pub mod protocol;
pub mod recording;
pub mod remembered;
pub mod scene;
pub mod scenes;
pub mod session;
pub mod socket;
pub mod sources;
pub mod timer;
pub mod wait;
pub mod wire;
