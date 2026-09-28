//! Which of a camera's modes to run.
//!
//! Left to itself a capture session runs a preset, and a preset picks the
//! device's *best* mode for that size, which for a webcam that can do sixty
//! is the sixty. Sixty frames a second into a picture drawn at thirty is
//! every other frame thrown away, twice the bytes down a USB 2 hub shared
//! with the other camera, and half the exposure a frame is allowed in a room
//! lit for thirty: darker, noisier, and never sharp. The mode is a decision,
//! so it is here and not in the adapter.

/// One mode a camera offers: a size, how the pixels are laid out (a four
/// character code as the device names it), and the frame rates it can hold.
#[derive(Debug, Clone, PartialEq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub pixels: String,
    /// Each rate the device will hold this mode at, one per range it offers.
    pub rates: Vec<f64>,
}

/// The mode to run, by its place in the list the device gave, and the rate
/// to hold it at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chosen {
    pub mode: usize,
    pub rate: f64,
}

/// Among `modes`, the one to run for a picture `wanted` pixels across and
/// tall, held at `rate`.
///
/// Exactly the wanted size when the device has it at that rate, the largest
/// that fits inside it otherwise, and none at all when nothing fits, which
/// leaves the session to its preset. Planar (`420v`) over packed (`yuvs`) at
/// the same size, since the packed modes a webcam advertises at this size run
/// at ten frames a second and the planar one is what the rest of the path
/// reads anyway.
#[must_use]
pub fn choose(modes: &[Mode], wanted: (u32, u32), rate: f64) -> Option<Chosen> {
    let holds = |mode: &Mode| mode.rates.iter().any(|r| (r - rate).abs() < 0.1);
    let fits = |mode: &Mode| mode.width <= wanted.0 && mode.height <= wanted.1;
    // Larger first, planar before packed among equals: the first one left
    // standing is the answer.
    let mut candidates: Vec<(usize, &Mode)> = modes
        .iter()
        .enumerate()
        .filter(|(_, mode)| holds(mode) && fits(mode))
        .collect();
    candidates.sort_by_key(|(_, mode)| {
        (
            std::cmp::Reverse(mode.width * mode.height),
            mode.pixels != PLANAR,
        )
    });
    candidates
        .first()
        .map(|(mode, _)| Chosen { mode: *mode, rate })
}

/// Y'CbCr 4:2:0 in two planes, what a webcam calls `420v` and what the
/// compositor reads without a conversion.
const PLANAR: &str = "420v";

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: u32, height: u32, pixels: &str, rates: &[f64]) -> Mode {
        Mode {
            width,
            height,
            pixels: pixels.into(),
            rates: rates.to_vec(),
        }
    }

    // The formats a common USB webcam lists.
    fn hp_430() -> Vec<Mode> {
        vec![
            mode(320, 240, "yuvs", &[30.0]),
            mode(320, 240, "420v", &[30.0, 25.0, 20.0]),
            mode(640, 360, "yuvs", &[30.0]),
            mode(640, 360, "420v", &[30.0, 25.0, 20.0]),
            mode(640, 480, "yuvs", &[30.0]),
            mode(640, 480, "420v", &[30.0, 25.0, 20.0]),
            mode(800, 600, "420v", &[30.0, 25.0, 20.0]),
            mode(800, 600, "yuvs", &[20.0]),
            mode(1024, 576, "420v", &[30.0, 25.0, 20.0]),
            mode(1024, 576, "yuvs", &[15.0]),
            mode(1280, 720, "420v", &[60.0, 30.0, 25.0]),
            mode(1280, 720, "yuvs", &[10.0]),
            mode(1920, 1080, "420v", &[30.0, 25.0, 20.0]),
            mode(1920, 1080, "yuvs", &[5.0]),
        ]
    }

    #[test]
    fn the_hp_runs_720p_planar_at_thirty_and_never_the_sixty() {
        let chosen = choose(&hp_430(), (1280, 720), 30.0).expect("a mode");
        assert_eq!(
            chosen,
            Chosen {
                mode: 10,
                rate: 30.0
            }
        );
    }

    #[test]
    fn a_camera_without_the_size_at_that_rate_gets_the_largest_that_fits() {
        let modes = vec![
            mode(640, 480, "420v", &[30.0]),
            mode(1024, 576, "420v", &[30.0]),
            mode(1280, 720, "420v", &[60.0]),
            mode(1920, 1080, "420v", &[30.0]),
        ];
        assert_eq!(
            choose(&modes, (1280, 720), 30.0),
            Some(Chosen {
                mode: 1,
                rate: 30.0
            })
        );
    }

    #[test]
    fn planar_wins_over_packed_at_the_same_size() {
        let modes = vec![
            mode(1280, 720, "yuvs", &[30.0]),
            mode(1280, 720, "420v", &[30.0]),
        ];
        assert_eq!(
            choose(&modes, (1280, 720), 30.0),
            Some(Chosen {
                mode: 1,
                rate: 30.0
            })
        );
    }

    #[test]
    fn a_rate_a_hair_off_thirty_still_counts_as_thirty() {
        // 30.00003, as UVC devices state it
        let modes = vec![mode(1280, 720, "420v", &[30.00003])];
        assert_eq!(
            choose(&modes, (1280, 720), 30.0),
            Some(Chosen {
                mode: 0,
                rate: 30.0
            })
        );
    }

    #[test]
    fn nothing_that_fits_is_nothing_and_the_preset_stays() {
        let modes = vec![
            mode(1920, 1080, "420v", &[30.0]),
            mode(1280, 720, "420v", &[60.0]),
        ];
        assert_eq!(choose(&modes, (1280, 720), 30.0), None);
        assert_eq!(choose(&[], (1280, 720), 30.0), None);
    }
}
