//! Audio meters, drawn the way OBS draws them.
//!
//! Ported from `assets/js/studio/levels.ts`. The numbers are not taste: they
//! are read out of obs-studio, `frontend/components/VolumeMeter.cpp`, so a
//! level that looks safe here looks safe in the program every streamer already
//! has an eye calibrated for.

/// A level meter's constants, as OBS defines them.
pub struct Meter;

impl Meter {
    pub const FLOOR_DB: f64 = -60.0;
    pub const WARNING_DB: f64 = -20.0;
    pub const ERROR_DB: f64 = -9.0;
    pub const TICK_DB: f64 = 6.0;
    pub const PEAK_HOLD_SECONDS: f64 = 1.0;
    pub const PEAK_DECAY_DB_PER_SECOND: f64 = 11.76;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Nominal,
    Warning,
    Error,
}

/// RMS of an analyser's byte time-domain data, which sits around 128.
pub fn rms_of_bytes(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let sum: f64 = bytes
        .iter()
        .map(|b| {
            let d = (*b as f64 - 128.0) / 128.0;
            d * d
        })
        .sum();
    (sum / bytes.len() as f64).sqrt()
}

pub fn meter_width(rms: f64) -> f64 {
    (rms * 300.0).min(100.0)
}

/// dBFS of an amplitude, floored so that silence is a number and not negative
/// infinity, which no meter can draw.
pub fn decibels(amplitude: f64) -> f64 {
    (20.0 * amplitude.max(1e-7).log10()).max(Meter::FLOOR_DB)
}

/// The amplitude a dB figure means: what a slider marked in dB sends.
pub fn amplitude(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Where the bar sits after this block: it jumps to any louder block and
/// falls at the meter's rate between them, which is what OBS's bar does and
/// what a person can read. A bar drawn from each block's own reading jumped
/// ten decibels between a kick and the gap after it and made a fader look
/// broken while it worked.
pub fn falling(bar_db: f64, now_db: f64, elapsed: f64) -> f64 {
    now_db.max((bar_db - Meter::PEAK_DECAY_DB_PER_SECOND * elapsed).max(Meter::FLOOR_DB))
}

/// The loudest sample in a block, as an amplitude. The bar is a peak meter,
/// like OBS's: the RMS of a block reads ten to fifteen decibels under the
/// peaks a person hears, which is why a track at full fader never reached the
/// red and a bed at a quarter fader drew nothing while it was plainly audible.
pub fn peak_of(samples: impl Iterator<Item = f64>) -> f64 {
    samples.fold(0.0, |loudest, sample| loudest.max(sample.abs()))
}

/// Where a level sits on the bar: 0% at the floor, 100% at full scale.
pub fn meter_place(db: f64) -> f64 {
    (((db - Meter::FLOOR_DB) / -Meter::FLOOR_DB) * 100.0).clamp(0.0, 100.0)
}

pub fn band_of(db: f64) -> Band {
    if db >= Meter::ERROR_DB {
        Band::Error
    } else if db >= Meter::WARNING_DB {
        Band::Warning
    } else {
        Band::Nominal
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peak {
    pub db: f64,
    pub held_for: f64,
}

/// The peak marker: it jumps to any louder reading, holds, then falls at a
/// fixed rate. `elapsed` is seconds since the last reading.
pub fn hold_peak(peak: Peak, db: f64, elapsed: f64) -> Peak {
    if db >= peak.db {
        return Peak { db, held_for: 0.0 };
    }
    let held_for = peak.held_for + elapsed;
    if held_for < Meter::PEAK_HOLD_SECONDS {
        return Peak {
            db: peak.db,
            held_for,
        };
    }
    Peak {
        db: (peak.db - Meter::PEAK_DECAY_DB_PER_SECOND * elapsed).max(Meter::FLOOR_DB),
        held_for,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_bytes_reads_an_analysers_byte_time_domain_data_around_128() {
        assert_eq!(rms_of_bytes(&[128u8; 256]), 0.0);
        let square: Vec<u8> = (0..256).map(|i| if i % 2 == 1 { 0 } else { 255 }).collect();
        assert!((rms_of_bytes(&square) - 0.996).abs() < 0.001);
    }

    #[test]
    fn meter_width_maps_rms_to_a_percentage_and_caps_at_100() {
        assert_eq!(meter_width(0.0), 0.0);
        assert!((meter_width(0.1) - 30.0).abs() < 1e-9);
        assert_eq!(meter_width(1.0), 100.0);
    }

    #[test]
    fn decibels_floors_silence_at_the_bottom_of_the_meter() {
        assert_eq!(decibels(1.0), 0.0);
        assert_eq!(decibels(0.0), Meter::FLOOR_DB);
        assert_eq!(decibels(0.5).round(), -6.0);
    }

    #[test]
    fn meter_place_puts_the_floor_at_zero_and_full_scale_at_a_hundred() {
        assert_eq!(meter_place(-60.0), 0.0);
        assert_eq!(meter_place(0.0), 100.0);
        assert_eq!(meter_place(-30.0), 50.0);
        assert_eq!(meter_place(-90.0), 0.0);
    }

    #[test]
    fn band_of_follows_obs_green_under_20_yellow_to_9_red_above() {
        assert_eq!(band_of(-21.0), Band::Nominal);
        assert_eq!(band_of(-20.0), Band::Warning);
        assert_eq!(band_of(-10.0), Band::Warning);
        assert_eq!(band_of(-9.0), Band::Error);
        assert_eq!(band_of(0.0), Band::Error);
    }

    #[test]
    fn hold_peak_jumps_up_at_once_holds_a_second_then_falls() {
        let quiet = Peak {
            db: -40.0,
            held_for: 0.0,
        };
        assert_eq!(
            hold_peak(quiet, -10.0, 0.1),
            Peak {
                db: -10.0,
                held_for: 0.0
            }
        );

        let held = hold_peak(
            Peak {
                db: -10.0,
                held_for: 0.0,
            },
            -50.0,
            0.5,
        );
        assert_eq!(
            held,
            Peak {
                db: -10.0,
                held_for: 0.5
            }
        );

        let falling = hold_peak(
            Peak {
                db: -10.0,
                held_for: 0.9,
            },
            -50.0,
            0.2,
        );
        assert!((falling.db - (-10.0 - 11.76 * 0.2)).abs() < 1e-9);
    }

    #[test]
    fn hold_peak_never_falls_below_the_meters_floor() {
        assert_eq!(
            hold_peak(
                Peak {
                    db: -58.0,
                    held_for: 5.0
                },
                -60.0,
                1.0
            )
            .db,
            Meter::FLOOR_DB
        );
    }

    #[test]
    fn the_bar_jumps_up_at_once_and_falls_at_the_meters_rate() {
        assert_eq!(falling(-40.0, -12.0, 0.01), -12.0, "up at once");
        let after = falling(-12.0, -60.0, 1.0);
        assert!(
            (after - (-12.0 - Meter::PEAK_DECAY_DB_PER_SECOND)).abs() < 1e-9,
            "{after}"
        );
        assert_eq!(
            falling(-59.0, -60.0, 10.0),
            Meter::FLOOR_DB,
            "never below the floor"
        );
        // Twenty decibels in 1.7 seconds, OBS's own number.
        let fell = -12.0 - falling(-12.0, -60.0, 1.7);
        assert!((fell - 20.0).abs() < 0.01, "{fell}");
    }

    #[test]
    fn the_peak_is_the_loudest_sample_either_way_round() {
        assert_eq!(peak_of([0.1, -0.7, 0.3].into_iter()), 0.7);
        assert_eq!(peak_of(std::iter::empty()), 0.0);
    }
}
