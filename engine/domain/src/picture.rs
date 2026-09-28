//! The picture: what is shown, where, and how. The scene and its layers, the
//! sources behind them, the camera's shape and place, the preview a face
//! reads, the timers drawn on it. Its filters are `remux-shader`, a crate of
//! their own for the WGSL parser they need.

pub mod camera;
pub mod layers;
pub mod preview;
pub mod scene;
pub mod scenes;
pub mod sources;
pub mod timer;
