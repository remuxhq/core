//! What plays next, as arithmetic. No audio here, so `cargo test` drives it.
//!
//! Ported from `assets/js/studio/music.ts`. A live runs for hours on twenty
//! tracks, so the order matters more than it looks: a straight shuffle repeats
//! a track two after itself often enough to notice, and a fixed order gets old
//! by the second pass. This shuffles but refuses to play anything it played in
//! the last few, which is what a radio does and what nobody notices, which is
//! the point.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub title: String,
    pub artist: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub title: String,
    pub tracks: Vec<Track>,
}

fn text(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The catalogue as the server sends it, narrowed at the boundary. A track
/// with no url is not a track; a playlist with no tracks is not a playlist.
pub fn parse_playlists(raw: &serde_json::Value) -> Vec<Playlist> {
    let Some(list) = raw.get("playlists").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            let tracks: Vec<Track> = item
                .get("tracks")
                .and_then(serde_json::Value::as_array)
                .map(|tracks| {
                    tracks
                        .iter()
                        .filter(|track| !text(track.get("url")).is_empty())
                        .map(|track| Track {
                            title: text(track.get("title")),
                            artist: track
                                .get("artist")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string),
                            url: text(track.get("url")),
                        })
                        .collect()
                })
                .unwrap_or_default();
            (!tracks.is_empty()).then(|| Playlist {
                name: text(item.get("name")),
                title: text(item.get("title")),
                tracks,
            })
        })
        .collect()
}

/// What a folder of files is, as a playlist.
///
/// Ported from `Remux.Music.Catalog` so that the engine and the app agree
/// about what counts as a track and what it is called. Stream-safe libraries
/// name files the same way, `Artist - Title.mp3`, so the artist is taken from
/// the name when it is there and left alone when it is not. Nobody is asked to
/// fill in metadata; that is the whole opinion of this feature.
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "ogg", "opus", "wav", "flac"];

/// Is this a file worth playing?
pub fn is_audio(file: &str) -> bool {
    if file.starts_with('.') {
        // Hidden files, and macOS scatters `._` resource forks through any
        // folder that has ever been on a USB stick.
        return false;
    }
    match file.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => {
            AUDIO_EXTENSIONS.contains(&extension.to_lowercase().as_str())
        }
        _ => false,
    }
}

/// A track from a file name, or nothing when the file is not audio.
pub fn track_from(file: &str, url: String) -> Option<Track> {
    if !is_audio(file) {
        return None;
    }
    let base = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    match base.split_once(" - ") {
        Some((artist, title)) => Some(Track {
            title: title.trim().to_string(),
            artist: Some(artist.trim().to_string()),
            url,
        }),
        None => Some(Track {
            title: base.trim().to_string(),
            artist: None,
            url,
        }),
    }
}

/// A folder name as a person reads it: `drum-and-bass` becomes "Drum and bass".
pub fn genre_title(name: &str) -> String {
    let words: Vec<&str> = name
        .split(['-', '_', ' '])
        .filter(|word| !word.is_empty())
        .collect();
    let joined = words.join(" ");
    let mut chars = joined.chars();
    match chars.next() {
        // Elixir's String.capitalize: the first letter up, the rest down.
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// Eight lines instead of a dependency. Which track comes next is not a place
/// that needs a real random number generator, and a pinned crate is a thing to
/// maintain forever.
struct Xorshift(u64);

impl Xorshift {
    fn seeded() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x2545_F491_4F6C_DD1D);
        Self(nanos | 1)
    }

    fn below(&mut self, upper: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % upper.max(1) as u64) as usize
    }
}

/// The order of a playlist over a long live: shuffled, never repeating what it
/// played in the recent past, and never running out.
pub struct Rotation {
    tracks: Vec<Track>,
    memory: usize,
    pick: Box<dyn FnMut(usize) -> usize + Send>,
    recent: Vec<String>,
}

impl Rotation {
    pub fn new(tracks: Vec<Track>) -> Self {
        let mut rng = Xorshift::seeded();
        Self::with_pick(tracks, Box::new(move |upper| rng.below(upper)))
    }

    pub fn with_pick(tracks: Vec<Track>, pick: Box<dyn FnMut(usize) -> usize + Send>) -> Self {
        // Remember about a third of the playlist, so twenty tracks never repeat
        // inside six, and a playlist of two still works.
        let memory = tracks.len() / 3;
        Self {
            tracks,
            memory,
            pick,
            recent: Vec::new(),
        }
    }

    pub fn size(&self) -> usize {
        self.tracks.len()
    }
}

/// A rotation is an endless sequence of tracks, so it is an iterator. It ends
/// only when there is nothing to play, which is the one honest way for an
/// endless thing to stop.
impl Iterator for Rotation {
    type Item = Track;

    fn next(&mut self) -> Option<Track> {
        if self.tracks.is_empty() {
            return None;
        }
        let fresh: Vec<&Track> = self
            .tracks
            .iter()
            .filter(|t| !self.recent.contains(&t.url))
            .collect();
        let from: Vec<&Track> = if fresh.is_empty() {
            self.tracks.iter().collect()
        } else {
            fresh
        };
        let index = (self.pick)(from.len());
        let track = from.get(index).or_else(|| from.first())?;
        let track = (*track).clone();
        self.recent.push(track.url.clone());
        if self.recent.len() > self.memory {
            let excess = self.recent.len() - self.memory;
            self.recent.drain(..excess);
        }
        Some(track)
    }
}

/// The fader is not the gain. Hearing is logarithmic, so a fader that hands
/// its position straight to a gain spends most of its travel arguing about the
/// loud half: 5% of amplitude is -26 dB, still plainly in the room, and
/// everything quieter lives in the first pixel. Squaring it overshoots the
/// other way and leaves a dead zone at the bottom, which is exactly where a
/// music bed is set.
///
/// So the travel is linear in dB between a floor and unity, the way a console
/// fader is: even resolution all the way down, and the bottom is off.
/// -60, not -40. A music bed under somebody coding sits 30 to
/// 40 dB under the voice, and over a 40 dB floor that was the bottom quarter
/// of the travel: "even at the minimum it seems loud", and two positions a
/// person can tell apart were 4 dB apart. Over 60 the bed is the middle of
/// the fader, every 10% is 6 dB, and 5% is -57, out of the room.
pub const FADER_FLOOR_DB: f64 = -60.0;

pub fn fader(position: f64) -> f64 {
    let p = position.clamp(0.0, MAX_GAIN);
    if p <= 0.0 {
        0.0
    } else if p <= 1.0 {
        10f64.powf(FADER_FLOOR_DB * (1.0 - p) / 20.0)
    } else {
        // Past unity it is not a fader any more, it is a boost, and the number
        // is what it says: 2.0 is twice the amplitude, which is +6 dB. The
        // panel has always been marked 0 to 200% and has always meant that.
        p
    }
}

/// How far past unity a fader goes. Twice the amplitude, +6 dB.
///
/// Not an invention: a console's microphone slider runs to 200% and passes
/// that straight through as a multiplier. Without
/// this a quiet microphone cannot be raised at all, which is felt in the first
/// minute of using it. There was a test asserting the missing headroom as
/// correct behaviour, which is how it survived.
pub const MAX_GAIN: f64 = 2.0;

/// What the fader's position sounds like, for a panel that speaks in dB.
pub fn fader_db(position: f64) -> f64 {
    let gain = fader(position);
    if gain <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * gain.log10()
    }
}

/// How far under the voice the music steps. Taste, and the track, decide.
pub const DUCK_DEFAULT_DB: f64 = -18.0;

/// Whether the music should step back: a voice, and a microphone that is
/// actually in the mix. A muted microphone is not, so a voice in it is
/// nobody's business and the bed stays up; a keyboard opens the gate and is
/// not a voice, which is the gate's call (`GateFrame::voice`), not this one.
pub fn ducks(voice: bool, muted: bool) -> bool {
    voice && !muted
}

/// How far the music is stepped back right now, in dB at or below zero.
///
/// It eases. It was a switch on the gate, and a switch on the gate is a
/// switch on every word: the bed dropped eighteen decibels in one block and
/// came back in the next gap, which a listener hears as the music chopping in
/// time with the speech. Down fast enough that the first word is not under
/// the music, up slowly enough that a breath between sentences is not a
/// swell: a radio host's hand on the fader, not a relay.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Duck {
    attenuation_db: f64,
}

impl Duck {
    /// Eighteen decibels in a hundred and fifty milliseconds.
    pub const ATTACK_DB_PER_SECOND: f64 = 120.0;
    /// Eighteen decibels back in three quarters of a second.
    pub const RELEASE_DB_PER_SECOND: f64 = 24.0;

    pub fn attenuation_db(&self) -> f64 {
        self.attenuation_db
    }

    /// One block passing: toward the depth while somebody speaks, back
    /// toward none when they stop, never past either.
    pub fn step(&mut self, speaking: bool, duck_db: f64, elapsed: f64) -> f64 {
        let target = if speaking { duck_db.min(0.0) } else { 0.0 };
        self.attenuation_db = if target < self.attenuation_db {
            (self.attenuation_db - Self::ATTACK_DB_PER_SECOND * elapsed).max(target)
        } else {
            (self.attenuation_db + Self::RELEASE_DB_PER_SECOND * elapsed).min(target)
        };
        self.attenuation_db
    }
}

/// How loud the music should be right now: the fader, stepped back by
/// however far the duck is at this block.
pub fn music_gain(position: f64, ducked_db: f64) -> f64 {
    music_fader(position) * 10f64.powf(ducked_db / 20.0)
}

/// The loudest the music ever goes out at. A live of a voice and a screen has
/// the music as a perk under them, and a fader that reached unity put the bed
/// level with the voice at its top and made the bottom third the only useful
/// part: "even the minimum is loud". The voice's fader keeps unity and above;
/// this one ends eighteen decibels under, which is a bed at its loudest.
pub const MUSIC_CEILING_DB: f64 = -18.0;

/// The music's fader: linear in dB from the floor to its own ceiling, 4.2 dB
/// every 10% of travel, off at the bottom. See `MUSIC_CEILING_DB`.
pub fn music_fader(position: f64) -> f64 {
    let p = position.clamp(0.0, 1.0);
    if p <= 0.0 {
        0.0
    } else {
        10f64.powf((FADER_FLOOR_DB + (MUSIC_CEILING_DB - FADER_FLOOR_DB) * p) / 20.0)
    }
}

/// Where the music's fader lands, in dB, for the reading beside it.
pub fn music_fader_db(position: f64) -> f64 {
    let gain = music_fader(position);
    if gain <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * gain.log10()
    }
}

/// How long the music takes to leave or arrive, in seconds. Long enough
/// that a cut is not a click, short enough that a track change still feels
/// like one: a codec with losses spreads a click over the frames around it,
/// and that was heard as a hiss at every change of track.
pub const FADE_SECONDS: f64 = 0.015;

/// The music's way in and out of a track: a gain that walks between 0 and 1
/// at the fade's pace, and a change (a new track, or stopping) held until the
/// music is all the way out. The mixer multiplies the bed by `gain()` and
/// asks `ready()` each block for the change it can now make.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
    gain: f64,
    /// Where the gain is going: 1 while music plays, 0 on the way out.
    towards: f64,
    /// Whether a change is waiting for the music to be all the way out.
    holding: bool,
}

impl Default for Fade {
    fn default() -> Self {
        Self {
            gain: 0.0,
            towards: 1.0,
            holding: false,
        }
    }
}

impl Fade {
    /// Something wants to change what plays: take the music out first.
    pub fn leave(&mut self) {
        self.towards = 0.0;
        self.holding = true;
    }

    /// The change was made: bring the new music in from silence.
    pub fn arrive(&mut self) {
        self.gain = 0.0;
        self.towards = 1.0;
        self.holding = false;
    }

    /// One block later. Returns the gain to multiply this block's bed by.
    pub fn step(&mut self, seconds: f64) -> f64 {
        let travel = seconds / FADE_SECONDS;
        if self.towards > self.gain {
            self.gain = (self.gain + travel).min(self.towards);
        } else if self.towards < self.gain {
            self.gain = (self.gain - travel).max(self.towards);
        }
        self.gain
    }

    /// Whether a held change may be made now: the music is all the way out.
    pub fn ready(&self) -> bool {
        self.holding && self.gain <= 0.0
    }

    pub fn gain(&self) -> f64 {
        self.gain
    }

    /// Whether a change of what plays waits for the music to leave.
    ///
    /// It waits while something is playing and the bed is still audible;
    /// with nothing playing there is nothing to take out, and holding the
    /// change then is a track that never starts. The mixer asked this with
    /// two conditions inline and a comment, which is where the fade's own
    /// rule had no test.
    pub fn holds(&self, playing: bool) -> bool {
        playing && self.gain > 0.0
    }
}

#[cfg(test)]
mod tests {
    // A change held while nothing plays is a track that never starts: the
    // first cut of the fade held every change, and choosing a genre from
    // silence did nothing at all.
    #[test]
    fn a_change_waits_only_while_there_is_music_to_take_out() {
        let mut fade = super::Fade::default();
        assert!(!fade.holds(true), "the bed is not up yet");
        assert!(!fade.holds(false));
        fade.arrive();
        fade.step(super::FADE_SECONDS);
        assert!(fade.holds(true), "a track is playing and audible");
        assert!(!fade.holds(false), "nothing plays: nothing to take out");
    }

    #[test]
    fn the_music_fader_ends_eighteen_decibels_under_and_walks_the_same_way_down() {
        assert!(
            (music_fader_db(1.0) - MUSIC_CEILING_DB).abs() < 1e-9,
            "its loudest is a bed"
        );
        assert!(
            (music_fader_db(0.5) + 39.0).abs() < 0.001,
            "halfway between -60 and -18"
        );
        assert!((music_fader_db(0.1) + 55.8).abs() < 0.001, "4.2 dB a notch");
        assert_eq!(music_fader(0.0), 0.0, "off at the bottom");
        assert!(
            (music_fader(2.0) - music_fader(1.0)).abs() < 1e-9,
            "and nothing past the top"
        );
        assert!(
            fader(1.0) > music_fader(1.0),
            "the voice's fader still reaches unity"
        );
    }

    #[test]
    fn a_track_arrives_from_silence_and_leaves_to_it_in_the_fade_time() {
        let mut fade = Fade::default();
        fade.arrive();
        assert_eq!(fade.gain(), 0.0, "in from silence");
        // 10 ms blocks: 15 ms of travel is one and a half blocks
        assert!((fade.step(0.01) - 0.6667).abs() < 0.001);
        assert_eq!(fade.step(0.01), 1.0);
        assert_eq!(fade.step(0.01), 1.0, "and it stays");
        fade.leave();
        assert!(!fade.ready(), "not until the music is out");
        fade.step(0.01);
        assert!(!fade.ready());
        assert_eq!(fade.step(0.01), 0.0);
        assert!(fade.ready(), "now the change can be made");
        fade.arrive();
        assert!(!fade.ready(), "the change was made; nothing is held");
    }

    #[test]
    fn leaving_while_arriving_turns_around_from_where_it_is() {
        let mut fade = Fade::default();
        fade.arrive();
        fade.step(0.01);
        fade.leave();
        let after = fade.step(0.005);
        assert!(
            after > 0.0 && after < 0.6667,
            "down from two thirds, not from one"
        );
    }

    #[test]
    fn a_quiet_microphone_can_be_raised_above_unity() {
        assert!((fader(1.0) - 1.0).abs() < 1e-9, "unity is unity");
        assert!(
            (fader(2.0) - 2.0).abs() < 1e-9,
            "200% is twice the amplitude"
        );
        assert!((fader_db(2.0) - 6.0206).abs() < 0.001, "which is +6 dB");
    }

    #[test]
    fn the_boost_stops_where_the_panel_stops() {
        assert!((fader(3.0) - MAX_GAIN).abs() < 1e-9);
        assert!((fader(f64::INFINITY) - MAX_GAIN).abs() < 1e-9);
    }

    #[test]
    fn below_unity_it_is_still_the_fader_it_was() {
        // Linear in dB, floor at -60: half way is -30.
        assert!((fader_db(0.5) + 30.0).abs() < 0.001);
        assert!((fader_db(0.1) + 54.0).abs() < 0.001);
        assert_eq!(fader(0.0), 0.0);
    }
    use super::*;

    fn track(n: usize) -> Track {
        Track {
            title: format!("t{n}"),
            artist: None,
            url: format!("/music/g/t{n}.mp3"),
        }
    }

    #[test]
    fn the_catalogue_is_narrowed_at_the_boundary_junk_and_all() {
        let parsed = parse_playlists(&serde_json::json!({
            "playlists": [
                { "name": "lofi", "title": "Lofi", "tracks": [
                    { "title": "Sunset", "artist": "Harris Heller", "url": "/music/lofi/a.mp3" },
                    { "title": "no url" }
                ]},
                { "name": "empty", "title": "Empty", "tracks": [] },
                "nonsense"
            ]
        }));

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "lofi");
        assert_eq!(
            parsed[0].tracks,
            vec![Track {
                title: "Sunset".into(),
                artist: Some("Harris Heller".into()),
                url: "/music/lofi/a.mp3".into()
            }]
        );
    }

    #[test]
    fn nothing_at_all_is_an_empty_catalogue_not_a_crash() {
        assert!(parse_playlists(&serde_json::Value::Null).is_empty());
        assert!(parse_playlists(&serde_json::json!({ "playlists": "no" })).is_empty());
    }

    #[test]
    fn a_rotation_never_repeats_what_it_played_in_the_recent_past() {
        let tracks: Vec<Track> = (0..20).map(track).collect();
        let mut rotation = Rotation::new(tracks);
        let mut played: Vec<String> = Vec::new();

        for _ in 0..200 {
            let next = rotation.next().expect("a track");
            // Six is a third of twenty: the window it refuses to repeat inside.
            let window = played.len().saturating_sub(6);
            assert!(
                !played[window..].contains(&next.url),
                "{} came back after {}",
                next.url,
                played[window..].join(", ")
            );
            played.push(next.url);
        }
        // And it does use the whole playlist.
        let distinct: std::collections::HashSet<&String> = played.iter().collect();
        assert_eq!(distinct.len(), 20);
    }

    #[test]
    fn a_playlist_of_one_keeps_playing_it() {
        let mut rotation = Rotation::new(vec![track(0)]);
        assert_eq!(
            rotation.next().map(|t| t.url),
            Some("/music/g/t0.mp3".into())
        );
        assert_eq!(
            rotation.next().map(|t| t.url),
            Some("/music/g/t0.mp3".into())
        );
    }

    #[test]
    fn an_empty_playlist_plays_nothing() {
        let mut rotation = Rotation::new(Vec::new());
        assert_eq!(rotation.next(), None);
        assert_eq!(rotation.size(), 0);
    }

    #[test]
    fn the_order_is_shuffled_not_the_file_order() {
        let tracks: Vec<Track> = (0..8).map(track).collect();
        // A pick that always takes the last candidate: the order must not be
        // t0 through t7.
        let mut rotation =
            Rotation::with_pick(tracks.clone(), Box::new(|upper| upper.saturating_sub(1)));
        let first: Vec<String> = (0..8)
            .filter_map(|_| rotation.next().map(|t| t.url))
            .collect();
        let in_file_order: Vec<String> = tracks.iter().map(|t| t.url.clone()).collect();
        assert_ne!(first, in_file_order);
    }

    #[test]
    fn talking_ducks_the_music_and_letting_go_brings_it_back() {
        assert_eq!(music_gain(0.8, 0.0), music_fader(0.8));
        // 18 dB under is an eighth of the level: still there, well out of the way.
        assert!((music_gain(0.8, DUCK_DEFAULT_DB) - music_fader(0.8) * 0.1259).abs() < 0.01);
        assert_eq!(music_gain(0.0, DUCK_DEFAULT_DB), 0.0);
    }

    #[test]
    fn the_fader_is_linear_in_db_so_its_travel_is_even_all_the_way_down() {
        assert_eq!(fader(0.0), 0.0);
        assert_eq!(fader(1.0), 1.0);
        assert_eq!(fader_db(1.0), 0.0);
        assert!((fader_db(0.5) - FADER_FLOOR_DB / 2.0).abs() < 1e-9);
        // A linear fader put 5% at -26 dB and was still in the room; squaring
        // it put the same 5% at -52 and took it out of earshot.
        // A bed for coding sits 30 to 40 dB under the voice, which with the
        // old -40 floor was the bottom quarter of the travel and with a
        // -60 floor is the middle: every 10% is 6 dB, and 5% is -57.
        assert!((fader_db(0.05) - -57.0).abs() < 0.01);
        assert!((fader_db(0.25) - -45.0).abs() < 0.01);
        // The bottom of the travel is off, not merely quiet.
        assert_eq!(fader_db(0.0), f64::NEG_INFINITY);
    }

    #[test]
    fn every_step_of_the_fader_moves_the_level_by_the_same_number_of_db() {
        let step = fader_db(0.2) - fader_db(0.1);
        assert!((fader_db(0.9) - fader_db(0.8) - step).abs() < 1e-9);
        assert!((step - -FADER_FLOOR_DB / 10.0).abs() < 1e-9);
    }

    #[test]
    fn the_fader_refuses_a_position_outside_its_travel() {
        // Below nothing is nothing; above the panel's travel is the panel's
        // travel. It used to stop at unity, which meant the top half of the
        // slider did nothing at all.
        assert_eq!(fader(-1.0), 0.0);
        assert_eq!(fader(3.0), MAX_GAIN);
    }

    #[test]
    fn how_far_the_music_ducks_is_a_setting_not_a_constant() {
        let loud = music_gain(1.0, -6.0);
        let quiet = music_gain(1.0, -30.0);
        assert!(loud > quiet);
        // from the music's own top, which is a bed and not unity
        let top = music_fader(1.0);
        assert!((loud - top * 0.501).abs() < 0.001);
        assert!((quiet - top * 0.0316).abs() < 0.0001);
        assert_eq!(DUCK_DEFAULT_DB, -18.0);
    }

    #[test]
    fn a_file_is_a_track_when_it_is_audio() {
        for name in [
            "a.mp3", "a.M4A", "a.aac", "a.ogg", "a.opus", "a.wav", "a.FLAC",
        ] {
            assert!(is_audio(name), "{name} should be audio");
        }
        for name in ["notes.txt", "cover.jpg", "README", "song", "a."] {
            assert!(!is_audio(name), "{name} should not be");
        }
    }

    // macOS scatters `._` resource forks through any folder that has been on a
    // USB stick, and they are named after the real files beside them.
    #[test]
    fn hidden_files_are_not_tracks() {
        assert!(!is_audio(".DS_Store"));
        assert!(!is_audio("._Harris Heller - Guilty Spark.mp3"));
    }

    // Stream-safe libraries all name files this way, so the artist comes for
    // free and nobody is asked to fill in metadata.
    #[test]
    fn the_artist_comes_out_of_the_file_name_when_it_is_there() {
        let track = track_from(
            "Harris Heller - Guilty Spark.mp3",
            "/music/lofi/x.mp3".into(),
        )
        .expect("a track");
        assert_eq!(track.artist.as_deref(), Some("Harris Heller"));
        assert_eq!(track.title, "Guilty Spark");
    }

    #[test]
    fn a_file_with_no_artist_in_its_name_keeps_its_whole_name() {
        let track = track_from("Guilty Spark.mp3", "/x".into()).expect("a track");
        assert_eq!(track.artist, None);
        assert_eq!(track.title, "Guilty Spark");
    }

    // Only the first " - " splits: a title with a dash in it stays whole.
    #[test]
    fn only_the_first_dash_separates_the_artist() {
        let track = track_from("A - B - C.mp3", "/x".into()).expect("a track");
        assert_eq!(track.artist.as_deref(), Some("A"));
        assert_eq!(track.title, "B - C");
    }

    #[test]
    fn a_genre_folder_reads_like_a_person_wrote_it() {
        assert_eq!(genre_title("lofi"), "Lofi");
        assert_eq!(genre_title("drum-and-bass"), "Drum and bass");
        assert_eq!(genre_title("synth_wave"), "Synth wave");
        assert_eq!(genre_title("EDM"), "Edm");
        assert_eq!(genre_title(""), "");
    }

    // The duck eases in and out. It was a switch on the gate, and a switch on
    // the gate is a switch on every word: the music dropped eighteen decibels
    // in one block and came back in the next gap, which a listener hears as
    // the bed chopping in time with the speech.
    #[test]
    fn the_duck_eases_in_over_a_tenth_of_a_second_rather_than_dropping_at_once() {
        let mut duck = Duck::default();
        let after_one_block = duck.step(true, -18.0, 0.01);
        assert!(
            after_one_block < 0.0 && after_one_block > -6.0,
            "one block in it is on its way, not there: {after_one_block}"
        );
        for _ in 0..20 {
            duck.step(true, -18.0, 0.01);
        }
        assert!(
            duck.attenuation_db() <= -17.9,
            "and there within a fifth of a second"
        );
    }

    #[test]
    fn the_duck_eases_out_over_more_than_half_a_second() {
        let mut duck = Duck::default();
        for _ in 0..30 {
            duck.step(true, -18.0, 0.01);
        }
        for _ in 0..10 {
            duck.step(false, -18.0, 0.01);
        }
        let after_a_tenth = duck.attenuation_db();
        assert!(
            after_a_tenth < -10.0,
            "a tenth of a second after the voice stops the bed is still down: {after_a_tenth}"
        );
        for _ in 0..70 {
            duck.step(false, -18.0, 0.01);
        }
        assert_eq!(duck.attenuation_db(), 0.0, "and back within a second");
    }

    #[test]
    fn the_duck_never_goes_below_the_depth_asked_for_nor_above_none() {
        let mut duck = Duck::default();
        for _ in 0..200 {
            assert!(duck.step(true, -12.0, 0.01) >= -12.0);
        }
        for _ in 0..200 {
            assert!(duck.step(false, -12.0, 0.01) <= 0.0);
        }
    }

    // A muted microphone is not in the mix, so a voice in it is nobody's
    // business: the bed stays up. It ducked anyway, on the gate alone.
    #[test]
    fn a_muted_microphone_never_ducks() {
        assert!(ducks(true, false));
        assert!(!ducks(true, true));
        assert!(!ducks(false, false));
    }
}
