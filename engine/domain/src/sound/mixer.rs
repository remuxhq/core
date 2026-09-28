//! Audio filtering as the domain tunes it: the gate (`remux-mixer`'s, with
//! the boundary a socket message crosses to change it) and the meters' scale.

pub mod gate;
pub mod levels;
