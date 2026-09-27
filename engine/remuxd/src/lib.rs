//! The remux engine. No interface: clients drive it over a socket.
//!
//! What is here is the transport and the wiring: `boot` is the daemon as a
//! function, and a `main` gives it a motor. The decisions are
//! `remuxd-domain`, a crate that sees no framework; the machine is a motor
//! behind the domain's ports; the seam is a fact of the build.

pub mod boot;
pub mod library;
pub mod prefs;
pub mod server;
pub mod wire;

pub use remuxd_domain::log;

pub use remuxd_domain::{engine, gate, levels, music, protocol, sources};
