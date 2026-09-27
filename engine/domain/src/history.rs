//! The record of lives: one line of JSON per broadcast, appended when it
//! ends, in `~/.config/remux/history.jsonl`. What went out is sampled off
//! the muxer every two seconds while on air and summed up here; the
//! platforms' peak rides along. `remux history` reads it back.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::protocol::Outgoing;

/// `REMUX_HISTORY`, or `~/.config/remux/history.jsonl`.
pub fn path() -> PathBuf {
    if let Some(said) = std::env::var_os("REMUX_HISTORY") {
        return PathBuf::from(said);
    }
    crate::os::config_dir().join("history.jsonl")
}

/// A live that ended, as the file keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Broadcast {
    pub started: i64,
    pub ended: i64,
    /// Where it went, by the destinations' names.
    pub destinations: Vec<String>,
    #[serde(default)]
    pub peak_viewers: Option<u32>,
    #[serde(default)]
    pub samples: u32,
    #[serde(default)]
    pub video_kbps_avg: u32,
    #[serde(default)]
    pub video_kbps_peak: u32,
    #[serde(default)]
    pub audio_kbps: u32,
    #[serde(default)]
    pub fps_avg: u32,
    #[serde(default)]
    pub fps_min: u32,
    /// The last size seen, `1920x1080`.
    #[serde(default)]
    pub resolution: String,
}

/// A live in progress: what it has sent so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sampler {
    started: i64,
    destinations: Vec<String>,
    last_sample: i64,
    samples: Vec<Outgoing>,
}

/// How often the muxer is asked while on air.
pub const EVERY_SECS: i64 = 2;

impl Sampler {
    pub fn start(started: i64, destinations: Vec<String>) -> Self {
        Self {
            started,
            destinations,
            last_sample: started,
            samples: Vec::new(),
        }
    }

    /// Whether it is time to ask the muxer again.
    pub fn due(&self, now: i64) -> bool {
        now - self.last_sample >= EVERY_SECS
    }

    /// A reading off the muxer. Off air everything reads zero, and a zero
    /// before the first packet is not a sample.
    pub fn sample(&mut self, now: i64, outgoing: Outgoing) {
        self.last_sample = now;
        if outgoing.video_kbps > 0 || outgoing.fps > 0 {
            self.samples.push(outgoing);
        }
    }

    /// The live, ended now, summed up.
    pub fn finish(self, ended: i64, peak_viewers: Option<u32>) -> Broadcast {
        let n = self.samples.len() as u32;
        let avg = |of: fn(&Outgoing) -> u32| {
            self.samples
                .iter()
                .map(of)
                .sum::<u32>()
                .checked_div(n)
                .unwrap_or(0)
        };
        let last = self.samples.last();
        Broadcast {
            started: self.started,
            ended,
            destinations: self.destinations,
            peak_viewers,
            samples: n,
            video_kbps_avg: avg(|o| o.video_kbps),
            video_kbps_peak: self.samples.iter().map(|o| o.video_kbps).max().unwrap_or(0),
            audio_kbps: avg(|o| o.audio_kbps),
            fps_avg: avg(|o| o.fps),
            fps_min: self.samples.iter().map(|o| o.fps).min().unwrap_or(0),
            resolution: last
                .map(|o| format!("{}x{}", o.width, o.height))
                .unwrap_or_default(),
        }
    }
}

/// One more line on the file. A write that fails is not a reason to stop
/// anything: the record goes missing, the live does not.
pub fn append(path: &Path, broadcast: &Broadcast) -> Result<(), String> {
    use std::io::Write;
    let line = serde_json::to_string(broadcast).map_err(|e| e.to_string())?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    writeln!(file, "{line}").map_err(|e| e.to_string())
}

/// Every live on record, newest first; a line that will not parse is skipped.
pub fn read(path: &Path) -> Vec<Broadcast> {
    let mut all: Vec<Broadcast> = std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    all.reverse();
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(video_kbps: u32, fps: u32) -> Outgoing {
        Outgoing {
            width: 1920,
            height: 1080,
            fps,
            video_kbps,
            audio_kbps: 160,
        }
    }

    #[test]
    fn a_live_is_sampled_every_two_seconds_and_summed_up_when_it_ends() {
        let mut live = Sampler::start(100, vec!["tw".into(), "yt".into()]);
        assert!(!live.due(101));
        assert!(live.due(102));
        live.sample(102, Outgoing::default());
        live.sample(104, out(6000, 30));
        live.sample(106, out(5000, 28));
        let record = live.finish(160, Some(12));
        assert_eq!(
            record.samples, 2,
            "a zero before the first packet is not a sample"
        );
        assert_eq!(
            (record.video_kbps_avg, record.video_kbps_peak),
            (5500, 6000)
        );
        assert_eq!((record.fps_avg, record.fps_min), (29, 28));
        assert_eq!(record.resolution, "1920x1080");
        assert_eq!(record.ended - record.started, 60);
        assert_eq!(Sampler::start(0, vec![]).finish(1, None).resolution, "");
    }

    #[test]
    fn the_file_is_appended_and_read_back_newest_first() {
        let path = std::env::temp_dir().join(format!("remux-history-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(read(&path).is_empty());
        let first = Sampler::start(1, vec!["a".into()]).finish(2, None);
        let second = Sampler::start(3, vec!["b".into()]).finish(4, Some(1));
        append(&path, &first).unwrap();
        append(&path, &second).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, b"not json\n"))
            .unwrap();
        assert_eq!(read(&path), vec![second, first]);
        let _ = std::fs::remove_file(path);
    }
}
