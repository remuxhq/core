use super::*;
use crate::protocol::Category;
use crate::sources::{DisplayId, WindowId};

fn engine() -> Engine {
    Engine::new()
}

/// A machine with two monitors and three windows: enough shape for every
/// decision the engine makes about sources. The built-in is display 1 and
/// the monitor is display 3, an ordering a real machine produces and the
/// reason position is never the handle.
struct ThisMachine;

impl Sources for ThisMachine {
    fn available(&self) -> Result<Available, String> {
        Ok(Available {
            screens: vec![
                Screen {
                    id: DisplayId(1),
                    name: "Built-in Retina Display".into(),
                },
                Screen {
                    id: DisplayId(3),
                    name: "VG2791R".into(),
                },
            ],
            windows: vec![
                Window {
                    id: WindowId(10),
                    title: "tmux a".into(),
                    app: "Ghostty".into(),
                },
                Window {
                    id: WindowId(11),
                    title: "remux".into(),
                    app: "Brave Browser".into(),
                },
                Window {
                    id: WindowId(12),
                    title: "notes".into(),
                    app: "TextEdit".into(),
                },
            ],
            cameras: vec![
                Named {
                    id: "6C707041".into(),
                    name: "MacBook Pro Camera".into(),
                },
                Named {
                    id: "0x111000".into(),
                    name: "HP 430/435 FHD Webcam".into(),
                },
            ],
            mics: vec![
                Named {
                    id: "c9bf1690".into(),
                    name: "HyperX DuoCast".into(),
                },
                Named {
                    id: "BuiltInMic".into(),
                    name: "MacBook Pro Microphone".into(),
                },
            ],
            apps: vec!["Spotify".into(), "Brave Browser".into()],
        })
    }
}

struct Refused;

impl Sources for Refused {
    fn available(&self) -> Result<Available, String> {
        Err("macOS refused: screen recording".into())
    }
}

// Setting up is the part nobody wants to do twice.
#[test]
fn what_was_chosen_comes_back_after_a_restart() {
    let (mut engine, played, _) = machine_with_music();
    engine.handle(Command::Mic {
        device: Some("HyperX".into()),
    });
    engine.handle(Command::Mirror { on: true });
    engine.handle(Command::Volume { level: 1.5 });
    engine.handle(Command::Genre { name: "edm".into() });
    engine.handle(Command::CardText {
        which: Card::StartingSoon,
        text: "back at nine".into(),
    });
    let setup = engine.remembered();
    // Never state: an engine that came up publishing because it was
    // publishing when the machine slept goes live in an empty room.
    let written = crate::remembered::write(&setup).expect("written");
    assert!(!written.contains("on_air"), "{written}");
    assert!(!written.contains("recording"), "{written}");

    let (mut next, next_played, _) = machine_with_music();
    next.restore(&crate::remembered::read(&written));
    let now = next.status();
    assert_eq!(now.mic.as_deref(), Some("HyperX DuoCast"));
    assert!(now.mirrored);
    assert_eq!(now.faders.mic, 1.5);
    assert_eq!(now.words.starting, "back at nine");
    assert!(!now.on_air);
    // The genre came back as the shelf, not as music: nothing plays until
    // somebody turns it on, and then it is that shelf. An engine that came
    // up playing was the music that "kept playing on its own with the app
    // closed": every engine started for a smoke played the last genre
    // through the speakers of a room nobody was in.
    assert!(
        now.music.is_none(),
        "restoring a genre started it: {:?}",
        now.music
    );
    assert!(next_played.lock().expect("played").is_empty());
    next.handle(Command::Music { on: true });
    let track = next_played
        .lock()
        .expect("played")
        .first()
        .cloned()
        .flatten();
    assert!(
        track.as_deref().is_some_and(|t| t.contains("edm")),
        "music on resumes the remembered shelf, played {track:?}"
    );
    let _ = played;
}

// Losing a webcam is not a reason to lose the gate settings somebody spent
// an evening on.
#[test]
fn a_device_that_is_gone_does_not_take_the_rest_of_the_setup_with_it() {
    let (mut engine, _, _) = machine_with_music();
    let setup = crate::remembered::Remembered {
        camera: Some("a camera nobody has".into()),
        mic: Some("HyperX".into()),
        mirrored: true,
        ..Default::default()
    };
    engine.restore(&setup);
    assert_eq!(engine.status().camera, None, "it must not invent one");
    assert_eq!(engine.status().mic.as_deref(), Some("HyperX DuoCast"));
    assert!(engine.status().mirrored);
}

#[test]
fn the_status_carries_what_the_engine_has_done() {
    let mut engine = Engine::new();
    let _ = engine.handle(Command::Mute { on: true });
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status");
    };
    // The stamp is the clock; what is asserted is the sentence and that
    // asking for the status did not itself become a line.
    assert_eq!(status.log.len(), 1);
    assert!(status.log[0].ends_with("mic muted"), "got {:?}", status.log);
}

#[test]
fn devices_come_from_whatever_is_plugged_in() {
    let mut engine = Engine::with_sources(Box::new(ThisMachine));
    let Reply::Devices(devices) = engine.handle(Command::Devices) else {
        panic!("devices answers with devices")
    };
    assert_eq!(devices.screens.len(), 2);
    assert_eq!(devices.screens[1].name, "VG2791R");
    // A window is shown as who owns it and then what it says, because a
    // browser window is titled after the page and neither half alone
    // tells you which one it is.
    assert_eq!(devices.windows[1].name, "Brave Browser — remux");
}

// An empty list would read as "you have no monitors", which sends a person
// looking at their cables instead of at System Settings.
#[test]
fn a_refused_grant_comes_back_as_a_sentence_not_an_empty_list() {
    let mut engine = Engine::with_sources(Box::new(Refused));
    assert_eq!(
        engine.handle(Command::Devices),
        Reply::Error {
            message: "macOS refused: screen recording".into()
        }
    );
}

#[test]
fn an_engine_with_nothing_plugged_in_says_so_truthfully() {
    let mut engine = engine();
    assert_eq!(
        engine.handle(Command::Devices),
        Reply::Devices(Devices::default())
    );
}

#[test]
fn a_fresh_engine_is_off_air_and_not_recording() {
    let mut engine = engine();
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert!(!status.on_air);
    assert!(!status.recording);
    assert_eq!(status.viewers, None);
}

#[test]
fn an_engine_that_captures_nothing_cannot_go_live() {
    let mut engine = engine();
    let reply = engine.handle(Command::GoLive);
    assert!(
        matches!(reply, Reply::Error { .. }),
        "nothing is plugged in, so there is nothing to send, got {reply:?}"
    );
    assert!(!engine.status().on_air);
    // Stopping what never started is not an error: a panel that lost track
    // should be able to say stop and be believed.
    assert_eq!(engine.handle(Command::Stop), Reply::Ok);
    assert!(!engine.status().on_air);
}

#[test]
fn recording_needs_no_destination_so_it_does_not_touch_the_air() {
    let (mut engine, _) = publishing_engine(None);
    engine.set_recordings(Some("/tmp/films".into()));
    engine.handle(Command::RecordStart);
    assert!(engine.status().recording);
    assert!(!engine.status().on_air, "recording is not going live");
    engine.handle(Command::RecordStop);
    assert!(!engine.status().recording);
}

#[test]
fn recording_survives_going_live_and_coming_back_off() {
    let (mut engine, _) = publishing_engine(None);
    engine.set_recordings(Some("/tmp/films".into()));
    engine.handle(Command::RecordStart);
    engine.handle(Command::GoLive);
    engine.handle(Command::Stop);
    assert!(
        engine.status().recording,
        "stopping the live must not stop the file"
    );
}

#[test]
fn the_live_card_is_the_absence_of_a_card() {
    let mut engine = engine();
    engine.handle(Command::Card {
        which: Card::StartingSoon,
    });
    assert_eq!(engine.status().card, Some(Card::StartingSoon));
    engine.handle(Command::Card { which: Card::Live });
    assert_eq!(engine.status().card, None);
}

// The panic button had a bug worth keeping a test for: it hid the picture
// and left the microphone open, which is the worst of both.
#[test]
fn hiding_everything_mutes_you_too() {
    let (mut engine, _) = publishing_engine(None);
    engine.handle(Command::Screen { display: 1 });
    engine.handle(Command::GoLive);
    engine.handle(Command::HideEverything);
    assert!(
        engine.status().muted,
        "hide everything means hide your voice too"
    );
    assert_eq!(engine.status().card, Some(Card::BackInAMoment));
    assert_eq!(engine.status().screen, None);
    assert_eq!(engine.status().camera, None);
    assert!(
        engine.status().on_air,
        "it is a break, not the end of the live"
    );
}

#[test]
fn a_fader_outside_its_travel_is_brought_back_into_it() {
    let mut engine = engine();
    engine.handle(Command::Volume { level: 3.0 });
    assert!(
        (engine.mic_db() - 6.0206).abs() < 0.001,
        "it stops at the 200% the panel offers, which is +6 dB, not at unity"
    );
    engine.handle(Command::Volume { level: -1.0 });
    assert_eq!(engine.mic_db(), f64::NEG_INFINITY);
}

#[test]
fn the_duck_only_ever_goes_downward() {
    let mut engine = engine();
    engine.handle(Command::Duck { db: 12.0 });
    assert_eq!(
        engine.duck_db(),
        0.0,
        "a duck that raises the music is not a duck"
    );
    engine.handle(Command::Duck { db: -99.0 });
    assert_eq!(engine.duck_db(), -30.0);
}

#[test]
fn every_verb_in_the_protocol_is_one_this_build_answers() {
    // There used to be a `Reply::Unsupported` and a test naming whichever
    // verb was still missing; it was `next-track`, then `arm`, and now
    // there is nothing to name. A reply nothing can produce is a reply
    // somebody will write a reader for, so it went with the last gap.
    //
    // What is asserted instead is the property that made it safe to
    // delete: every command decodes into something `handle` answers, which
    // the compiler proves by the match being exhaustive, and this proves
    // by asking for one of each shape.
    let (mut engine, _) = publishing_engine(None);
    for command in [
        Command::Status,
        Command::Devices,
        Command::Grants,
        Command::Chat {
            since: 0,
            follow: false,
        },
        Command::Mute { on: true },
        Command::Arm {
            adapter: 1,
            on: true,
        },
    ] {
        let reply = engine.handle(command.clone());
        assert!(
            !matches!(reply, Reply::Error { message } if message.contains("cannot")),
            "{command:?} came back as unanswerable"
        );
    }
}

#[test]
fn quitting_is_a_decision_the_engine_records_and_does_not_act_on() {
    let mut engine = engine();
    assert!(!engine.quitting());
    assert_eq!(engine.handle(Command::Quit), Reply::Ok);
    assert!(
        engine.quitting(),
        "the transport reads this and stops accepting"
    );
}

fn machine() -> Engine {
    Engine::with_sources(Box::new(ThisMachine))
}

// A screen is chosen by its display id, never by its position: the
// capturer and the display list order the same hardware differently, and
// a real machine shows it (the built-in is display 1, the monitor is 3).
#[test]
fn a_screen_is_chosen_by_its_display_id_and_answers_with_its_name() {
    let mut engine = machine();
    let Reply::Status(status) = engine.handle(Command::Screen { display: 3 }) else {
        panic!("choosing a screen answers with the new status")
    };
    assert_eq!(status.screen, Some("VG2791R".into()));
}

// "no display 9" alone sends a person hunting. Saying what there is
// instead turns a dead end into the answer.
#[test]
fn asking_for_a_display_that_is_not_there_says_what_is() {
    let mut engine = machine();
    let Reply::Error { message } = engine.handle(Command::Screen { display: 9 }) else {
        panic!("an absent display is an error")
    };
    assert!(message.contains("no display 9"), "{message}");
    assert!(
        message.contains("Built-in Retina Display and VG2791R"),
        "it should say what there is: {message}"
    );
    assert_eq!(engine.status().screen, None, "and change nothing");
}

#[test]
fn a_window_is_chosen_by_part_of_its_title() {
    let mut engine = machine();
    let Reply::Status(status) = engine.handle(Command::Window {
        query: "tmux".into(),
    }) else {
        panic!("choosing a window answers with the new status")
    };
    assert_eq!(status.screen, Some("Ghostty — tmux a".into()));
}

// The CLI has always taken part of a name, and a browser window is titled
// after the page it is showing, so the application has to match too.
#[test]
fn a_window_is_found_by_the_application_that_owns_it() {
    let mut engine = machine();
    engine.handle(Command::Window {
        query: "brave".into(),
    });
    assert_eq!(engine.status().screen, Some("Brave Browser — remux".into()));
}

#[test]
fn asking_for_a_window_that_is_not_there_changes_nothing() {
    let mut engine = machine();
    engine.handle(Command::Screen { display: 3 });
    let Reply::Error { message } = engine.handle(Command::Window {
        query: "photoshop".into(),
    }) else {
        panic!("an absent window is an error")
    };
    assert!(message.contains("photoshop"), "{message}");
    assert_eq!(
        engine.status().screen,
        Some("VG2791R".into()),
        "a miss must not drop what was already chosen"
    );
}

// A window replaces a screen and a screen replaces a window: there is one
// picture, and this is what is behind it.
#[test]
fn choosing_one_source_replaces_the_other() {
    let mut engine = machine();
    engine.handle(Command::Screen { display: 1 });
    assert_eq!(
        engine.status().screen,
        Some("Built-in Retina Display".into())
    );
    engine.handle(Command::Window {
        query: "notes".into(),
    });
    assert_eq!(engine.status().screen, Some("TextEdit — notes".into()));
    engine.handle(Command::Screen { display: 3 });
    assert_eq!(engine.status().screen, Some("VG2791R".into()));
}

#[test]
fn a_refused_grant_is_a_sentence_when_choosing_too() {
    let mut engine = Engine::with_sources(Box::new(Refused));
    assert!(matches!(
        engine.handle(Command::Screen { display: 1 }),
        Reply::Error { .. }
    ));
    assert!(matches!(
        engine.handle(Command::Window { query: "x".into() }),
        Reply::Error { .. }
    ));
}

#[test]
fn the_camera_and_the_microphone_are_separate_lists() {
    let mut engine = machine();
    // "HyperX" is a microphone, so asking the camera list for it must miss
    // rather than quietly find the mic.
    let Reply::Error { message } = engine.handle(Command::Camera {
        device: Some("hyperx".into()),
    }) else {
        panic!("a camera named after a microphone is not a camera")
    };
    assert!(message.contains("MacBook Pro Camera"), "{message}");
    assert_eq!(engine.status().camera, None);

    engine.handle(Command::Mic {
        device: Some("hyperx".into()),
    });
    assert_eq!(engine.status().mic, Some("HyperX DuoCast".into()));
}

#[test]
fn a_camera_is_found_by_part_of_its_name_and_by_its_id() {
    let mut engine = machine();
    engine.handle(Command::Camera {
        device: Some("webcam".into()),
    });
    assert_eq!(engine.status().camera, Some("HP 430/435 FHD Webcam".into()));

    engine.handle(Command::Camera {
        device: Some("6C707041".into()),
    });
    assert_eq!(
        engine.status().camera,
        Some("MacBook Pro Camera".into()),
        "an id is the handle a saved preference holds"
    );
}

// Turning the camera off is a different thing from never having chosen
// one, and both are allowed: the camera is a slot in the picture.
#[test]
fn a_camera_can_be_turned_off_again() {
    let mut engine = machine();
    engine.handle(Command::Camera {
        device: Some("webcam".into()),
    });
    assert!(engine.status().camera.is_some());
    engine.handle(Command::Camera { device: None });
    assert_eq!(engine.status().camera, None);
}

#[test]
fn asking_for_a_device_that_is_not_plugged_in_says_what_is() {
    let mut engine = machine();
    let Reply::Error { message } = engine.handle(Command::Mic {
        device: Some("Rode NT-USB".into()),
    }) else {
        panic!("an absent device is an error")
    };
    assert!(message.contains("Rode NT-USB"), "{message}");
    assert!(
        message.contains("HyperX DuoCast and MacBook Pro Microphone"),
        "it should say what there is: {message}"
    );
    assert_eq!(engine.status().mic, None, "and change nothing");
}

/// A pipeline that writes down what it was told, so a test can ask whether
/// the capture was actually pointed somewhere rather than only whether the
/// status changed. The two disagreeing is the bug worth catching.
type Published = std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>;

#[derive(Default)]
struct Wrote {
    told: std::sync::Arc<std::sync::Mutex<Vec<Behind>>>,
    cameras: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    mics: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    played: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    levels: Faders,
    shown: Shown,
    counting: std::sync::Arc<std::sync::Mutex<Option<Duration>>>,
    /// Every destination it was told to publish to, and a `None` for every
    /// time it was told to stop, in order.
    published: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    /// Every file it was told to write, and a `None` for every stop.
    recorded: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    mirrored: std::sync::Arc<std::sync::Mutex<bool>>,
    gated: std::sync::Arc<std::sync::Mutex<Option<GateParams>>>,
    heard_here: std::sync::Arc<std::sync::Mutex<bool>>,
    /// Every time the speakers were told anything, on or off, in order. What
    /// "left alone" means is this not growing.
    speaker_calls: std::sync::Arc<std::sync::Mutex<Vec<bool>>>,
    /// The last thing it was told about the preview: on, off, or never.
    previewed: std::sync::Arc<std::sync::Mutex<Option<bool>>>,
    /// Set by a test to say the track that was playing has run out.
    ran_out: std::sync::Arc<std::sync::Mutex<bool>>,
    refuse: Option<String>,
}

impl Picture for Wrote {
    fn capture(&mut self, behind: Behind) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.told.lock().expect("told").push(behind);
        Ok(())
    }
    fn camera(&mut self, device: Option<&str>) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.cameras
            .lock()
            .expect("cameras")
            .push(device.map(str::to_string));
        Ok(())
    }
    fn show(&mut self, card: Card, line: &str, counting: Option<Duration>) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.shown
            .lock()
            .expect("shown")
            .push((card, line.to_string()));
        *self.counting.lock().expect("counting") = counting;
        Ok(())
    }
    fn mirror(&mut self, on: bool) {
        *self.mirrored.lock().expect("mirrored") = on;
    }
    fn previewing(&mut self, on: bool) {
        *self.previewed.lock().expect("previewed") = Some(on);
    }
    fn shot(&mut self, of: Framed) -> Option<(Vec<u8>, u32, u32)> {
        // Three bytes that are not a JPEG: what this proves is the path,
        // and a fake that produced a real one would be proving libjpeg.
        // The two shapes differ so a caller asking for the wrong one shows
        // up as the wrong size rather than as nothing at all.
        match of {
            Framed::Scene => Some((vec![0xff, 0xd8, 0xff], 480, 270)),
            Framed::Camera => Some((vec![0xff, 0xd8, 0xff], 480, 360)),
            Framed::Screen => Some((vec![0xff, 0xd8, 0xff], 480, 300)),
        }
    }
    fn stop(&mut self) {}
    fn camera_flowing(&self) -> Flowing {
        Flowing {
            captured: 3,
            frames: 3,
            width: 1280,
            height: 720,
            held: None,
        }
    }
    fn flowing(&self) -> Flowing {
        // Nothing is flowing until something has been pointed at, which is
        // what a cold engine looks like and what makes "there is no
        // picture to send yet" a thing a test can reach.
        if self.told.lock().expect("told").is_empty() {
            return Flowing::default();
        }
        Flowing {
            captured: 7,
            frames: 42,
            width: 1920,
            height: 1080,
            held: None,
        }
    }
}

impl Sound for Wrote {
    fn music_ended(&mut self) -> bool {
        // Exactly once, like the real one: asking twice must not skip a
        // track, which is the bug this shape exists to make impossible.
        std::mem::replace(&mut *self.ran_out.lock().expect("ran out"), false)
    }
    fn mic(&mut self, device: Option<&str>) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.mics
            .lock()
            .expect("mics")
            .push(device.map(str::to_string));
        Ok(())
    }
    fn play(&mut self, track: Option<&Track>) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.played
            .lock()
            .expect("played")
            .push(track.map(|t| t.title.clone()));
        Ok(())
    }
    fn levels(
        &mut self,
        mic: f64,
        music: f64,
        duck: f64,
        muted: bool,
        to_stream: bool,
        screen_sound: bool,
    ) -> Result<(), String> {
        *self.levels.lock().expect("levels") =
            Some((mic, music, duck, muted, to_stream, screen_sound));
        Ok(())
    }
    fn monitor(&mut self, on: bool) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.speaker_calls.lock().expect("speaker calls").push(on);
        *self.heard_here.lock().expect("heard here") = on;
        Ok(())
    }
    fn speakers(&self) -> Option<String> {
        self.heard_here
            .lock()
            .expect("heard here")
            .then(|| "Desk speakers".to_string())
    }
    fn gate(&mut self, params: GateParams) {
        *self.gated.lock().expect("gated") = Some(params);
    }
    fn mixing(&self) -> Mixing {
        Mixing {
            frames: 480,
            level_db: -18.0,
            peak_db: -14.0,
            music_db: -30.0,
            music_peak_db: -26.0,
            music_out_db: -20.0,
            music_out_peak_db: -18.0,
            ducked_db: 0.0,
            playing: true,
            monitor_db: -60.0,
        }
    }
    fn hearing(&self) -> Hearing {
        Hearing {
            samples: 4800,
            level_db: -21.0,
            peak_db: -17.0,
            gate_levels: crate::gate::GateLevels {
                full: 0.05,
                hf: 0.001,
            },
            gate_open: true,
            gain_db: 0.0,
            starved: 0,
            buffered: 0,
            dropped: 0,
            screen_samples: 0,
            screen_complaint: None,
            complaint: None,
        }
    }
}

impl Air for Wrote {
    fn publish(&mut self, url: &str) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        self.published
            .lock()
            .expect("published")
            .push(Some(url.to_string()));
        Ok(())
    }
    fn unpublish(&mut self) {
        self.published.lock().expect("published").push(None);
    }
    /// Up while the last thing on the record is a stream that started; a
    /// test ends one from outside by pushing a `None`, the way a relay that
    /// hung up would.
    fn still_publishing(&mut self) -> bool {
        matches!(
            self.published.lock().expect("published").last(),
            Some(Some(_))
        )
    }
    fn record(&mut self, into: &str) -> Result<String, String> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        let name = format!("{into}/remux-whenever.mp4");
        self.recorded
            .lock()
            .expect("recorded")
            .push(Some(name.clone()));
        Ok(name)
    }
    fn stop_recording(&mut self) {
        self.recorded.lock().expect("recorded").push(None);
    }
}

impl Pipeline for Wrote {
    fn grants(&self) -> (Grant, Grant, Grant) {
        (Grant::Granted, Grant::Granted, Grant::NotAsked)
    }
}

// ---- going live actually sends something ------------------------------

fn publishing_engine(refuse: Option<String>) -> (Engine, Published) {
    let published: Published = Default::default();
    let pipeline = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: Default::default(),
        levels: Default::default(),
        shown: Default::default(),
        counting: Default::default(),
        published: published.clone(),
        recorded: Default::default(),
        mirrored: Default::default(),
        gated: Default::default(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: Default::default(),
        refuse,
    };
    (
        Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_destination(Some(DESTINATION.into())),
        published,
    )
}

const DESTINATION: &str = "rtmp://hub/scene?user=remux&pass=not-a-real-key";

// A scene is the setup of now under a name, and a switch puts it back whole:
// what is behind the picture, the camera's corner, the mirror; kept across
// restarts by the words that chose the window, not its id.
#[test]
fn a_scene_keeps_the_setup_and_a_switch_puts_it_back() {
    use crate::scene::{Corner, LayoutPatch};
    let (mut one, _) = publishing_engine(None);
    one.handle(Command::Screen { display: 1 });
    one.handle(Command::Layout {
        patch: LayoutPatch {
            corner: Some(Corner::TopLeft),
            ..LayoutPatch::default()
        },
    });
    one.handle(Command::Mirror { on: true });
    assert!(matches!(
        one.handle(Command::SceneSave {
            name: "code".into()
        }),
        Reply::Status(_)
    ));
    one.handle(Command::Layout {
        patch: LayoutPatch {
            corner: Some(Corner::BottomRight),
            ..LayoutPatch::default()
        },
    });
    one.handle(Command::Mirror { on: false });
    one.handle(Command::Share { on: false });
    let put_back = one.handle(Command::SceneSwitch {
        name: "code".into(),
    });
    assert!(matches!(put_back, Reply::Status(_)), "{put_back:?}");
    assert_eq!(one.status().layout.corner, Corner::TopLeft);
    assert!(one.status().mirrored);
    assert!(
        one.status().screen.is_some(),
        "the screen is back behind the picture"
    );
    assert_eq!(one.status().scene.as_deref(), Some("code"));
    assert_eq!(one.status().scenes, vec!["code".to_string()]);
    assert!(matches!(
        one.handle(Command::SceneSwitch {
            name: "nope".into()
        }),
        Reply::Error { .. }
    ));
    let mut fresh = engine();
    fresh.restore(&one.remembered());
    assert_eq!(
        fresh.status().scenes,
        vec!["code".to_string()],
        "a restart keeps the scenes"
    );
    assert!(matches!(
        one.handle(Command::SceneForget {
            name: "code".into()
        }),
        Reply::Status(_)
    ));
    assert!(one.status().scenes.is_empty());
}

// The screen's sound can be one application's: the name as the system
// lists it, matched loosely, the switch turned on with it, and a name that
// is not running refused with the names that are.
#[test]
fn hearing_an_application_names_it_as_the_system_does_and_sends_it() {
    let mut one = Engine::with_sources(Box::new(ThisMachine));
    assert!(matches!(
        one.handle(Command::Hear {
            apps: vec!["spot".into()]
        }),
        Reply::Status(_)
    ));
    assert_eq!(one.status().hearing_apps, vec!["Spotify".to_string()]);
    assert!(one.status().screen_sound);
    let Reply::Error { message } = one.handle(Command::Hear {
        apps: vec!["Zoom".into()],
    }) else {
        panic!("an app that is not running is refused")
    };
    assert!(
        message.contains("Zoom") && message.contains("Brave Browser"),
        "{message}"
    );
    assert!(matches!(
        one.handle(Command::Hear { apps: vec![] }),
        Reply::Status(_)
    ));
    assert!(one.status().hearing_apps.is_empty());
}

// Straight to every armed destination, one door each, all of them or none:
// the engine hands the file's outlets to the pipeline, and a door that
// refuses takes the open ones down with it.
#[test]
fn going_live_sends_to_every_armed_destination_in_the_file() {
    use crate::destinations::{add, write, Local};
    let path = std::env::temp_dir().join(format!("remuxd-test-dest-{}.json", std::process::id()));
    let mut kept = Vec::new();
    add(
        &mut kept,
        "yt",
        "youtube",
        "rtmp://a.rtmp.youtube.com/live2",
        "yt-key",
    )
    .unwrap();
    add(
        &mut kept,
        "tw",
        "twitch",
        "rtmp://live.twitch.tv/app",
        "tw-key",
    )
    .unwrap();
    write(&path, &kept).unwrap();
    let (engine, published) = publishing_engine(None);
    let mut engine = engine
        .with_destination(None)
        .with_app(Box::new(Local::new(path.clone())));
    engine.handle(Command::Screen { display: 1 });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    assert_eq!(
        *published.lock().expect("published"),
        vec![
            Some("rtmp://a.rtmp.youtube.com/live2/yt-key".to_string()),
            Some("rtmp://live.twitch.tv/app/tw-key".to_string())
        ]
    );
    let _ = std::fs::remove_file(path);
}

// A live that ended is one line on the history file, whichever way it ended,
// naming where it went.
#[test]
fn a_live_that_ended_is_written_down() {
    let path =
        std::env::temp_dir().join(format!("remuxd-test-history-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let (engine, published) = publishing_engine(None);
    let mut engine = engine.with_history(Some(path.clone()));
    engine.handle(Command::Screen { display: 1 });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    engine.tick();
    assert_eq!(engine.handle(Command::Stop), Reply::Ok);
    let kept = crate::history::read(&path);
    assert_eq!(kept.len(), 1);
    assert!(kept[0].ended >= kept[0].started);
    // ended on its own: the relay hung up
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    published.lock().expect("published").push(None);
    engine.tick();
    assert!(!engine.status().on_air);
    assert_eq!(crate::history::read(&path).len(), 2);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_clip_nobody_has_is_refused_by_name_and_the_folder_is_named() {
    let mut one = engine();
    let Reply::Error { message } = one.handle(Command::Clip {
        name: "no-such-clip".into(),
    }) else {
        panic!("a missing clip is an error")
    };
    assert!(
        message.contains("no-such-clip") && message.contains("clips"),
        "{message}"
    );
}

// The camera's layout is the engine's, read back by every face, kept across
// restarts, and drawn by the pipeline whatever is behind the picture.
#[test]
fn the_layout_is_held_remembered_and_handed_to_the_picture() {
    use crate::scene::{Corner, LayoutPatch, Shape};
    let mut one = engine();
    let reply = one.handle(Command::Layout {
        patch: LayoutPatch {
            corner: Some(Corner::TopLeft),
            shape: Some(Shape::Circle),
            ..LayoutPatch::default()
        },
    });
    assert!(matches!(reply, Reply::Status(_)));
    let layout = one.status().layout;
    assert_eq!(
        (layout.corner, layout.shape, layout.share),
        (Corner::TopLeft, Shape::Circle, 0.25)
    );
    assert_eq!(one.remembered().layout, layout);
    let mut fresh = engine();
    fresh.restore(&one.remembered());
    assert_eq!(fresh.status().layout, layout, "a restart keeps the corner");
}

// A plan is confirmed by its fingerprint: the live goes on what was printed
// and refused on anything else, so an agent cannot confirm a plan it did not
// see and a person cannot go on a screen that changed under the prompt.
#[test]
fn a_live_confirmed_on_a_stale_plan_is_refused() {
    let (mut engine, published) = publishing_engine(None);
    engine.handle(Command::Screen { display: 1 });
    let Reply::Plan(plan) = engine.handle(Command::Plan) else {
        panic!("plan answers with a plan")
    };
    assert!(matches!(
        engine.handle(Command::Live {
            plan: plan.fingerprint.wrapping_add(1)
        }),
        Reply::Error { .. }
    ));
    assert!(published.lock().expect("published").is_empty());
    engine.handle(Command::Mute { on: true });
    assert!(
        matches!(
            engine.handle(Command::Live {
                plan: plan.fingerprint
            }),
            Reply::Error { .. }
        ),
        "muting after the plan is a change a person confirms"
    );
    let Reply::Plan(fresh) = engine.handle(Command::Plan) else {
        panic!()
    };
    assert_eq!(
        engine.handle(Command::Live {
            plan: fresh.fingerprint
        }),
        Reply::Ok
    );
    assert!(engine.status().on_air);
}

#[test]
fn going_live_sends_the_picture_to_the_destination() {
    let (mut engine, published) = publishing_engine(None);
    engine.handle(Command::Screen { display: 1 });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    assert!(engine.status().on_air);
    assert!(
        engine.status().on_air_since.is_some(),
        "the clock starts here, on the engine's own time, so every face \
             shows the same running time for the same live"
    );
    assert_eq!(
        *published.lock().expect("published"),
        vec![Some(DESTINATION.to_string())],
        "go live must reach the pipeline, not just flip a flag"
    );
    engine.handle(Command::Stop);
    assert_eq!(engine.status().on_air_since, None, "and stops with the air");
}

#[test]
fn an_engine_with_nowhere_to_go_does_not_claim_the_air() {
    let (mut engine, published) = publishing_engine(None);
    engine.set_destination(None);
    let reply = engine.handle(Command::GoLive);
    assert!(
        matches!(reply, Reply::Error { .. }),
        "with no destination going live is an error, got {reply:?}"
    );
    assert!(!engine.status().on_air);
    assert!(published.lock().expect("published").is_empty());
}

// The flag follows the stream: a relay that hangs up takes "on air" with it,
// on the next tick, and the log says so. It used to say on air for as long
// as nobody pressed Stop.
#[test]
fn a_stream_that_ends_on_its_own_takes_on_air_with_it() {
    let (mut engine, published) = publishing_engine(None);
    engine.handle(Command::Screen { display: 1 });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    engine.tick();
    assert!(
        engine.status().on_air,
        "a live that is going stays on air through a tick"
    );
    published.lock().expect("published").push(None);
    engine.tick();
    assert!(
        !engine.status().on_air,
        "the stream ended and the flag did not follow"
    );
    assert!(engine.status().on_air_since.is_none());
    let said = engine.reported().log.join("\n");
    assert!(
        said.contains("ended on its own"),
        "the log says nothing about it: {said}"
    );
}

#[test]
fn a_publisher_that_refuses_leaves_the_engine_off_air() {
    let (mut engine, _) = publishing_engine(Some("ffmpeg is not here".into()));
    let reply = engine.handle(Command::GoLive);
    assert!(
        matches!(reply, Reply::Error { .. }),
        "a live that could not start must say so, got {reply:?}"
    );
    assert!(
        !engine.status().on_air,
        "an engine that failed to publish must not report itself on air"
    );
}

#[test]
fn stopping_takes_the_stream_down() {
    let (mut engine, published) = publishing_engine(None);
    engine.handle(Command::Screen { display: 1 });
    engine.handle(Command::GoLive);
    assert_eq!(engine.handle(Command::Stop), Reply::Ok);
    assert!(!engine.status().on_air);
    assert_eq!(
        *published.lock().expect("published"),
        vec![Some(DESTINATION.to_string()), None],
        "stop must take the stream down, not just flip a flag"
    );
}

#[test]
fn recording_writes_a_file_and_says_so() {
    let (mut engine, _) = publishing_engine(None);
    engine.set_recordings(Some("/tmp/films".into()));
    assert_eq!(engine.handle(Command::RecordStart), Reply::Ok);
    assert!(engine.status().recording);
    assert!(!engine.status().on_air, "recording is not going live");
    assert_eq!(engine.handle(Command::RecordStop), Reply::Ok);
    assert!(!engine.status().recording);
}

#[test]
fn an_engine_with_nowhere_to_write_does_not_claim_to_be_recording() {
    let (mut engine, _) = publishing_engine(None);
    engine.set_recordings(None);
    let reply = engine.handle(Command::RecordStart);
    assert!(
        matches!(reply, Reply::Error { .. }),
        "with no folder recording is an error, got {reply:?}"
    );
    assert!(!engine.status().recording);
}

#[test]
fn a_recorder_that_refuses_leaves_the_clock_stopped() {
    let (mut engine, _) = publishing_engine(Some("the disk is full".into()));
    engine.set_recordings(Some("/tmp/films".into()));
    let reply = engine.handle(Command::RecordStart);
    assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
    assert!(
        !engine.status().recording,
        "a clock ticking over a file that was never opened is a lie"
    );
}

#[test]
fn going_live_with_no_picture_is_refused_at_once() {
    let (mut engine, published) = publishing_engine(None);
    // `Wrote` reports nothing flowing until something is captured, which
    // is what a cold engine looks like.
    let reply = engine.handle(Command::GoLive);
    assert!(
        matches!(reply, Reply::Error { .. }),
        "there is nothing to send, got {reply:?}"
    );
    assert!(!engine.status().on_air);
    assert!(
        published.lock().expect("published").is_empty(),
        "it must not even reach the pipeline"
    );
}

#[test]
fn the_self_view_flips_and_the_status_can_be_read_back() {
    let (mut engine, _) = publishing_engine(None);
    assert!(!engine.status().mirrored, "a camera starts unflipped");
    let Reply::Status(after) = engine.handle(Command::Mirror { on: true }) else {
        panic!("mirror answers with a status, so a panel redraws from one line")
    };
    assert!(after.mirrored);
    engine.handle(Command::Mirror { on: false });
    assert!(!engine.status().mirrored);
}

#[test]
fn flipping_reaches_the_picture_and_not_only_the_status() {
    let mirrored: std::sync::Arc<std::sync::Mutex<bool>> = Default::default();
    let pipeline = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: Default::default(),
        levels: Default::default(),
        shown: Default::default(),
        counting: Default::default(),
        published: Default::default(),
        recorded: Default::default(),
        mirrored: mirrored.clone(),
        gated: Default::default(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: Default::default(),
        refuse: None,
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
    engine.handle(Command::Mirror { on: true });
    assert!(
        *mirrored.lock().expect("mirrored"),
        "the compositor has to be told, not just the status"
    );
}

#[test]
fn the_music_a_panel_can_offer_comes_back_with_the_devices() {
    let mut engine =
        Engine::with_sources(Box::new(ThisMachine)).with_library(Box::new(ThreeGenres));
    let Reply::Devices(devices) = engine.handle(Command::Devices) else {
        panic!("devices answers with devices")
    };
    assert_eq!(
        devices
            .genres
            .iter()
            .map(|g| g.id.as_str())
            .collect::<Vec<_>>(),
        vec!["lofi", "edm", "synthwave"],
        "a panel that can play music has to be able to offer some"
    );
    assert_eq!(
        devices.genres[0].name, "Lofi",
        "the name is the readable one"
    );
}

#[test]
fn a_panel_can_read_the_faders_back_where_it_put_them() {
    let (mut engine, _) = publishing_engine(None);
    engine.handle(Command::Volume { level: 1.5 });
    engine.handle(Command::MusicVolume { level: 0.3 });
    engine.handle(Command::Duck { db: -24.0 });
    let faders = engine.status().faders;
    assert!((faders.mic - 1.5).abs() < 1e-9, "the microphone past unity");
    assert!((faders.music - 0.3).abs() < 1e-9);
    assert!((faders.duck_db + 24.0).abs() < 1e-9);
}

#[test]
fn the_faders_start_where_the_engine_starts_them() {
    let faders = engine().status().faders;
    assert!((faders.mic - 1.0).abs() < 1e-9, "unity, not silence");
    assert_eq!(faders, crate::protocol::Faders::default());
}

#[test]
fn a_panel_can_read_back_what_the_cards_were_told_to_say() {
    let (mut engine, _) = publishing_engine(None);
    engine.handle(Command::CardText {
        which: Card::StartingSoon,
        text: "Chegando já".into(),
    });
    assert_eq!(engine.status().words.starting, "Chegando já");
    assert_eq!(
        engine.status().words.back,
        crate::card::BACK_IN_A_MOMENT,
        "the one that was not changed still says what it said"
    );
}

#[test]
fn the_gate_is_tuned_a_slider_at_a_time_and_answers_with_the_whole_set() {
    let gated: std::sync::Arc<std::sync::Mutex<Option<GateParams>>> = Default::default();
    let pipeline = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: Default::default(),
        levels: Default::default(),
        shown: Default::default(),
        counting: Default::default(),
        published: Default::default(),
        recorded: Default::default(),
        mirrored: Default::default(),
        gated: gated.clone(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: Default::default(),
        refuse: None,
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));

    let before = engine.status().gate;
    let Reply::Status(after) = engine.handle(Command::Gate {
        patch: serde_json::json!({ "full": 0.2 }),
    }) else {
        panic!("the gate answers with a status, so every face redraws from one answer")
    };
    assert!(
        (after.gate.full - 0.2).abs() < 1e-9,
        "the slider that moved"
    );
    assert!(
        (after.gate.hf - before.hf).abs() < 1e-9,
        "and only that one: a patch is not a replacement"
    );
    assert_eq!(
        gated.lock().expect("gated").map(|p| p.full),
        Some(0.2),
        "the microphone has to be told, not just the status"
    );
}

#[test]
fn a_panel_can_ask_for_a_picture_of_what_is_going_out() {
    let (mut engine, _) = publishing_engine(None);
    let Reply::Shot {
        jpeg,
        width,
        height,
    } = engine.handle(Command::Shot { of: Framed::Scene })
    else {
        panic!("a shot answers with a shot")
    };
    assert_eq!((width, height), (480, 270));
    assert_eq!(
        jpeg, "/9j/",
        "the bytes come back as base64, not as a number"
    );
}

#[test]
fn an_engine_with_no_picture_says_so_rather_than_sending_an_empty_one() {
    let mut engine = engine();
    assert!(
        matches!(
            engine.handle(Command::Shot { of: Framed::Scene }),
            Reply::Error { .. }
        ),
        "nothing is plugged in, so there is nothing to show"
    );
}

#[test]
fn a_panel_can_ask_what_the_machine_allows() {
    let (mut engine, _) = publishing_engine(None);
    assert_eq!(
        engine.handle(Command::Grants),
        Reply::Grants {
            screen: Grant::Granted,
            camera: Grant::Granted,
            microphone: Grant::NotAsked,
        },
        "all three, always: the one that is wrong is what somebody is looking for"
    );
}

#[test]
fn an_engine_that_captures_nothing_has_been_allowed_nothing() {
    let mut engine = engine();
    let Reply::Grants { screen, .. } = engine.handle(Command::Grants) else {
        panic!("grants answers with grants")
    };
    assert_eq!(screen, Grant::Refused, "true rather than convenient");
}

/// An app with two destinations and something said in the chat.
struct AnApp;

impl Watching for AnApp {
    fn reachable(&self) -> bool {
        true
    }
    fn destinations(&self) -> Vec<Destination> {
        vec![
            Destination {
                id: 1,
                name: "tico".into(),
                platform: "twitch".into(),
                status: "live".into(),
                armed: true,
                sandbox: false,
                connected: true,
                account: None,
                category: None,
                category_id: None,
                viewers_peak: None,
                trouble: None,
                viewers: Some(42),
                channel: Some("http://localhost:8880/tico/".into()),
                title: Some("Rust at midnight".into()),
                description: None,
            },
            Destination {
                id: 2,
                name: "teco".into(),
                platform: "youtube".into(),
                status: "off".into(),
                armed: false,
                sandbox: false,
                connected: true,
                account: None,
                category: None,
                category_id: None,
                viewers_peak: None,
                trouble: None,
                // Off air and nobody asked: the two are different and only
                // one of them is a zero.
                viewers: None,
                channel: None,
                title: None,
                description: None,
            },
        ]
    }
    fn viewers(&self) -> Option<u32> {
        Some(12)
    }
    fn arm(&self, _adapter: i64, _on: bool) -> Result<(), String> {
        Ok(())
    }
    fn sandbox(&self, _adapter: i64, _on: bool) -> Result<(), String> {
        Ok(())
    }
    fn retitle(&self, _: i64, _: Option<&str>, _: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn announce(&self, _adapter: i64) -> Result<(), String> {
        Ok(())
    }
    fn disconnect(&self, _adapter: i64) -> Result<(), String> {
        Ok(())
    }
    fn categorize(&self, _adapter: i64, _id: &str, _name: &str) -> Result<(), String> {
        Ok(())
    }
    fn search_categories(&self, _adapter: i64, _query: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn the_engine_repeats_the_app_rather_than_holding_a_copy() {
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(AnApp));
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert!(status.app, "and says the app is reachable");
    assert_eq!(status.destinations.len(), 2);
    assert_eq!(status.destinations[0].name, "tico");
    assert_eq!(
        status.destinations[0].title.as_deref(),
        Some("Rust at midnight"),
        "what the live is called rides on the row, so every face can show it"
    );
    assert_eq!(status.viewers, Some(12));
}

/// What an app was asked to call a live: the destination, the title, the
/// line under it.
type Asked = std::sync::Arc<std::sync::Mutex<Vec<(i64, Option<String>, Option<String>)>>>;

/// An app that writes down what it was asked to call a live.
struct Retitled(Asked);

impl Watching for Retitled {
    fn categorize(&self, adapter: i64, id: &str, name: &str) -> Result<(), String> {
        self.0
            .lock()
            .expect("retitled")
            .push((adapter, Some(format!("filed {id} {name}")), None));
        Ok(())
    }
    fn search_categories(&self, adapter: i64, query: &str) -> Result<(), String> {
        self.0
            .lock()
            .expect("retitled")
            .push((adapter, Some(format!("searched {query}")), None));
        Ok(())
    }
    fn found(&self) -> Option<Found> {
        Some(Found {
            adapter: 2,
            query: "sci".into(),
            items: vec![Category {
                id: "509670".into(),
                name: "Science & Technology".into(),
            }],
        })
    }
    fn reachable(&self) -> bool {
        true
    }
    fn destinations(&self) -> Vec<Destination> {
        Vec::new()
    }
    fn viewers(&self) -> Option<u32> {
        None
    }
    fn arm(&self, _adapter: i64, _on: bool) -> Result<(), String> {
        Ok(())
    }
    fn retitle(
        &self,
        adapter: i64,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<(), String> {
        self.0.lock().expect("retitled").push((
            adapter,
            title.map(str::to_string),
            description.map(str::to_string),
        ));
        Ok(())
    }
    fn sandbox(&self, adapter: i64, on: bool) -> Result<(), String> {
        // Written down as a title, like the announcement, so one list serves.
        self.0.lock().expect("retitled").push((
            adapter,
            Some(if on { "sandbox on" } else { "sandbox off" }.into()),
            None,
        ));
        Ok(())
    }
    fn disconnect(&self, adapter: i64) -> Result<(), String> {
        self.0
            .lock()
            .expect("retitled")
            .push((adapter, Some("disconnected".into()), None));
        Ok(())
    }
    fn announce(&self, adapter: i64) -> Result<(), String> {
        // Written down as a title of "announced", so one list serves.
        self.0
            .lock()
            .expect("retitled")
            .push((adapter, Some("announced".into()), None));
        Ok(())
    }
    fn notices(&mut self) -> Vec<String> {
        vec!["! tico: the token was refused".into()]
    }
}

// OBS has "Update": tell the platform now, not at the next Go live. The
// engine only pushes, so the platform's refusal comes back as a notice
// and lands in the log with the destination's name.
#[test]
fn an_announcement_is_asked_of_the_app_and_its_refusal_reaches_the_log() {
    let asked: Asked = Default::default();
    let mut told =
        Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
    assert_eq!(told.handle(Command::Announce { adapter: 2 }), Reply::Ok);
    assert_eq!(
        asked.lock().expect("retitled").last(),
        Some(&(2, Some("announced".to_string()), None))
    );
    told.tick();
    let Reply::Status(now) = told.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    let log = now.log;
    assert!(
        log.iter()
            .any(|line| line.contains("! tico: the token was refused")),
        "the notice is in the log: {log:?}"
    );
}

// The title is the app's column, like `armed`: the engine asks and
// repeats, and never keeps a copy that could disagree with the row.
#[test]
fn disconnecting_is_asked_of_the_app_and_kept_nowhere() {
    let asked: Asked = Default::default();
    let mut told =
        Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
    assert_eq!(told.handle(Command::Disconnect { adapter: 2 }), Reply::Ok);
    assert_eq!(
        asked.lock().expect("retitled").last(),
        Some(&(2, Some("disconnected".to_string()), None))
    );
}

// The category is the app's column, like the title: the engine asks, keeps
// nothing, and the search's answer rides on the status for every face.
#[test]
fn a_category_is_asked_of_the_app_and_the_search_answers_on_the_status() {
    let asked: Asked = Default::default();
    let mut told =
        Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
    assert_eq!(
        told.handle(Command::Categorize {
            adapter: 2,
            id: "509670".into(),
            name: "Science & Technology".into()
        }),
        Reply::Ok
    );
    assert_eq!(
        asked.lock().expect("retitled").last(),
        Some(&(
            2,
            Some("filed 509670 Science & Technology".to_string()),
            None
        ))
    );
    assert_eq!(
        told.handle(Command::Categories {
            adapter: 2,
            query: "sci".into()
        }),
        Reply::Ok
    );
    let Reply::Status(now) = told.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    let found = now
        .categories
        .expect("the app's answer rides on the status");
    assert_eq!(found.query, "sci");
    assert_eq!(found.items[0].name, "Science & Technology");
}

// The sandbox is the app's column, like armed: the engine asks and keeps
// nothing, and every face reads it back on the rows.
#[test]
fn the_sandbox_is_asked_of_the_app_and_never_kept_here() {
    let asked: Asked = Default::default();
    let mut told =
        Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
    assert_eq!(
        told.handle(Command::Sandbox {
            adapter: 2,
            on: true
        }),
        Reply::Ok
    );
    assert_eq!(
        asked.lock().expect("retitled").last(),
        Some(&(2, Some("sandbox on".to_string()), None))
    );
    // and an engine with no app says so
    let mut alone = Engine::with_sources(Box::new(ThisMachine));
    assert!(matches!(
        alone.handle(Command::Sandbox {
            adapter: 2,
            on: false
        }),
        Reply::Error { .. }
    ));
}

#[test]
fn a_title_is_asked_of_the_app_and_never_kept_here() {
    let asked: Asked = Default::default();
    let mut told =
        Engine::with_sources(Box::new(ThisMachine)).with_app(Box::new(Retitled(asked.clone())));
    let reply = told.handle(Command::Retitle {
        adapter: 2,
        title: Some("Rust at midnight".into()),
        description: None,
    });
    assert_eq!(reply, Reply::Ok);
    assert_eq!(
        *asked.lock().expect("retitled"),
        vec![(2, Some("Rust at midnight".to_string()), None)]
    );
    // And with nobody to ask, it says so rather than pretending.
    let reply = engine().handle(Command::Retitle {
        adapter: 2,
        title: Some("x".into()),
        description: None,
    });
    assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
}

#[test]
fn an_engine_with_no_app_is_a_real_configuration_and_not_a_broken_one() {
    let mut engine = engine();
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert!(!status.app);
    assert!(status.destinations.is_empty());
    assert_eq!(
        status.viewers, None,
        "nobody answered is not nobody watching"
    );
    // And it says so rather than pretending it armed something.
    assert!(matches!(
        engine.handle(Command::Arm {
            adapter: 1,
            on: true
        }),
        Reply::Error { .. }
    ));
}

/// An engine whose wire has said three things, numbered from seven.
fn told_engine() -> Engine {
    use crate::chat::Feed;
    use crate::wire::Line;
    let mut feed = Feed::starting_at(7);
    for (id, body) in [("m7", "hello"), ("m8", "gg"), ("m9", "first!")] {
        feed.push(Line {
            id: id.into(),
            platform: "twitch".into(),
            channel: "tw".into(),
            from: "ana".into(),
            body: body.into(),
        });
    }
    feed.reachable = true;
    Engine::with_sources(Box::new(ThisMachine))
        .with_chat(std::sync::Arc::new(std::sync::Mutex::new(feed)))
}

#[test]
fn the_chat_says_whether_its_wire_is_up() {
    let mut told = told_engine();
    let Reply::Chat { reachable, lines } = told.handle(Command::Chat {
        since: 0,
        follow: false,
    }) else {
        panic!("chat answers with chat")
    };
    assert!(reachable);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].body, "hello");

    // A quiet room and a wire that is down look identical in the list
    // and must not look identical in the answer.
    let Reply::Chat { reachable, lines } = engine().handle(Command::Chat {
        since: 0,
        follow: false,
    }) else {
        panic!("chat answers with chat")
    };
    assert!(!reachable);
    assert!(lines.is_empty());
}

// A restart is the case. The count that said somebody was watching lived
// in the engine's memory, so an engine restarted under a live panel never
// published a preview frame again and the panel showed the last one it had
// for ever: screen, camera and scene, all still, and a card that never
// appeared. Watching is a lease: a face renews it while it draws, and an
// engine that has not heard from one in a few seconds stops publishing.
#[test]
fn watching_is_a_lease_a_face_keeps_renewing() {
    let previewed: std::sync::Arc<std::sync::Mutex<Option<bool>>> = Default::default();
    let pipeline = Wrote {
        previewed: previewed.clone(),
        ..Default::default()
    };
    let mut engine = machine().with_pipeline(Box::new(pipeline));
    let told = || *previewed.lock().expect("previewed");

    engine.handle(Command::Watching { on: true });
    assert_eq!(told(), Some(true), "a face drawing turns the preview on");
    for _ in 0..WATCH_LEASE_TICKS - 1 {
        engine.tick();
    }
    assert_eq!(told(), Some(true), "still within the lease");
    engine.handle(Command::Watching { on: true });
    for _ in 0..WATCH_LEASE_TICKS - 1 {
        engine.tick();
    }
    assert_eq!(told(), Some(true), "renewed in time, so still on");
    engine.tick();
    assert_eq!(told(), Some(false), "nobody renewed: the preview stops");
    engine.handle(Command::Watching { on: true });
    assert_eq!(
        told(),
        Some(true),
        "and a face coming back turns it on again"
    );
}

// A face asks once a second, and a thousand lines a second for nothing
// new is the cost that made the cap forty. So it asks for what came after
// the last line it has, by sequence, and gets only that.
#[test]
fn the_chat_hands_over_only_what_came_after_since() {
    let mut told = told_engine();
    let Reply::Chat { lines, .. } = told.handle(Command::Chat {
        since: 8,
        follow: false,
    }) else {
        panic!("chat answers with chat")
    };
    assert_eq!(
        lines.iter().map(|l| l.seq).collect::<Vec<_>>(),
        vec![9],
        "only what is newer than 8"
    );
}

// A spammer's line comes off every face, the overlay on the stream included,
// and stays off: the app keeps the conversation and would hand it back.
#[test]
fn a_hidden_line_is_gone_from_every_face_for_the_rest_of_the_run() {
    let mut told = told_engine();
    assert!(matches!(told.handle(Command::Hide { seq: 8 }), Reply::Ok));
    let Reply::Chat { lines, .. } = told.handle(Command::Chat {
        since: 0,
        follow: false,
    }) else {
        panic!("chat answers with chat")
    };
    assert_eq!(
        lines.iter().map(|l| l.seq).collect::<Vec<_>>(),
        vec![7, 9],
        "the hidden line is out, the rest stay in order"
    );
}

// Deleting is hiding plus a delete down the wire: the line leaves every face
// here at once and the source is told which message, on which destination.
#[test]
fn a_deleted_line_goes_down_the_wire_by_its_id_and_is_hidden_here() {
    let mut told = told_engine();
    assert_eq!(told.handle(Command::Delete { seq: 8 }), Reply::Ok);
    let Reply::Chat { lines, .. } = told.handle(Command::Chat {
        since: 0,
        follow: false,
    }) else {
        panic!("chat answers with chat")
    };
    assert_eq!(lines.iter().map(|l| l.seq).collect::<Vec<_>>(), vec![7, 9]);
    assert!(
        matches!(
            told.handle(Command::Delete { seq: 99 }),
            Reply::Error { .. }
        ),
        "a line the engine no longer has cannot be deleted from here"
    );
}

#[test]
fn hearing_your_own_mix_is_a_switch_every_face_can_read() {
    let (mut engine, _) = publishing_engine(None);
    assert!(!engine.status().monitoring, "the speakers start quiet");
    let Reply::Status(after) = engine.handle(Command::Monitor { on: true }) else {
        panic!("it answers with a status: this switch makes a noise in a room")
    };
    assert!(after.monitoring);
    engine.handle(Command::Monitor { on: false });
    assert!(!engine.status().monitoring);
}

// "I had to be able to hear the music even without streaming it. Sending
// it to the stream is the optional part, not hearing it. If it is on, I
// always hear it." So starting the music opens the speakers, and whether
// the bed reaches the audience is its own switch, on unless somebody says
// otherwise. The speakers stay a switch of their own because on speakers
// the microphone picks the music up and it goes out twice.
#[test]
fn hearing_the_music_is_not_optional_and_sending_it_out_is() {
    let levels: Faders = Default::default();
    let heard_here: std::sync::Arc<std::sync::Mutex<bool>> = Default::default();
    let pipeline = Wrote {
        levels: levels.clone(),
        heard_here: heard_here.clone(),
        ..Default::default()
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine))
        .with_pipeline(Box::new(pipeline))
        .with_library(Box::new(ThreeGenres));
    assert!(
        engine.status().music_to_stream,
        "the bed reaches the audience unless somebody says otherwise"
    );
    assert!(
        !engine.status().monitoring,
        "quiet until there is something to hear"
    );

    engine.handle(Command::Genre { name: "edm".into() });
    assert!(
        engine.status().monitoring,
        "starting the music opens the speakers"
    );
    assert!(
        *heard_here.lock().expect("heard here"),
        "and the pipeline was told"
    );

    let Reply::Status(after) = engine.handle(Command::StreamMusic { on: false }) else {
        panic!("a switch every face reads answers with a status")
    };
    assert!(!after.music_to_stream);
    assert_eq!(
        levels.lock().expect("levels").map(|l| l.4),
        Some(false),
        "the mixer was told to keep the bed out of the mix"
    );
    assert!(
        after.monitoring,
        "and the speakers are untouched: they are the other switch"
    );
}

// The music's own switch is what makes it heard: on opens the speakers
// (proven above), so off closes them, or the switch says one thing and
// the room says another.
// A person with a headset and a pair of speakers wants to know which one
// the bed is on, without opening System Settings; the status names it while
// the speakers are open and says nothing while they are closed.
#[test]
fn the_status_names_the_speakers_while_they_are_open() {
    let (mut engine, _, _) = machine_with_music();
    assert_eq!(engine.reported().speakers, None);
    engine.handle(Command::Genre { name: "edm".into() });
    assert_eq!(engine.reported().speakers.as_deref(), Some("Desk speakers"));
    engine.handle(Command::Monitor { on: false });
    assert_eq!(engine.reported().speakers, None);
}

#[test]
fn turning_the_music_off_closes_the_speakers() {
    let (mut engine, _, _) = machine_with_music();
    engine.handle(Command::Genre { name: "edm".into() });
    assert!(engine.status().monitoring);
    engine.handle(Command::Music { on: false });
    assert!(
        !engine.status().monitoring,
        "the speakers close with the music"
    );
}

#[test]
fn an_engine_that_cannot_open_an_output_says_so_and_stays_quiet() {
    let (mut engine, _) = publishing_engine(Some("no output device".into()));
    let reply = engine.handle(Command::Monitor { on: true });
    assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
    assert!(
        !engine.status().monitoring,
        "a switch that says it is on while nothing is playing is the worst \
             possible answer for this one"
    );
}

#[test]
fn silence_is_the_floor_and_never_full_scale() {
    // Zero dBFS is the loudest sound there is, so a derived default is the
    // worst possible reading for "nothing is connected". A panel drew a
    // full red meter for a microphone that was not open.
    let floor = crate::levels::Meter::FLOOR_DB;
    assert_eq!(Hearing::default().level_db, floor);
    assert_eq!(Mixing::default().level_db, floor);
    assert_eq!(Mixing::default().music_db, floor);
    assert_eq!(Mixing::default().monitor_db, floor);

    let mut engine = engine();
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert_eq!(status.hearing.level_db, floor, "nothing is plugged in");
    assert_eq!(status.mixing.music_db, floor);
}

fn machine_with_pipeline() -> (Engine, std::sync::Arc<std::sync::Mutex<Vec<Behind>>>) {
    let told = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let pipeline = Wrote {
        told: told.clone(),
        cameras: Default::default(),
        mics: Default::default(),
        played: Default::default(),
        levels: Default::default(),
        shown: Default::default(),
        counting: Default::default(),
        published: Default::default(),
        recorded: Default::default(),
        mirrored: Default::default(),
        gated: Default::default(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: Default::default(),
        refuse: None,
    };
    (
        Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_destination(Some(DESTINATION.into())),
        told,
    )
}

#[test]
fn choosing_a_screen_points_the_capture_at_that_display() {
    let (mut engine, told) = machine_with_pipeline();
    engine.handle(Command::Screen { display: 3 });
    assert_eq!(
        *told.lock().expect("told"),
        vec![Behind::Screen(DisplayId(3))]
    );
}

#[test]
fn choosing_a_window_points_the_capture_at_that_window() {
    let (mut engine, told) = machine_with_pipeline();
    engine.handle(Command::Window {
        query: "tmux".into(),
    });
    assert_eq!(
        *told.lock().expect("told"),
        vec![Behind::Window(WindowId(10))]
    );
}

// Every change goes through, on air or not. Swapping the monitor mid-live
// is a thing people do and it is where a pipeline is most likely to break,
// so it must not be a path that only runs off air.
#[test]
fn a_swap_while_on_air_still_reaches_the_capture() {
    let (mut engine, told) = machine_with_pipeline();
    engine.handle(Command::Screen { display: 1 });
    engine.handle(Command::GoLive);
    engine.handle(Command::Screen { display: 3 });
    engine.handle(Command::Window {
        query: "notes".into(),
    });
    assert_eq!(
        *told.lock().expect("told"),
        vec![
            Behind::Screen(DisplayId(1)),
            Behind::Screen(DisplayId(3)),
            Behind::Window(WindowId(12)),
        ]
    );
    assert!(engine.status().on_air, "and it never left the air");
}

// The switch that takes the screen off the live. Nothing is a real state
// with a picture of its own, not an absence: the frames have to keep
// flowing or a viewer cannot tell a deliberate blank from a dead stream.
#[test]
fn sharing_nothing_points_the_capture_at_nothing() {
    let (mut engine, told) = machine_with_pipeline();
    engine.handle(Command::Screen { display: 3 });
    let Reply::Status(status) = engine.handle(Command::Share { on: false }) else {
        panic!("the share switch answers a status")
    };
    assert_eq!(status.screen, None);
    assert_eq!(
        told.lock().expect("told").last(),
        Some(&Behind::Nothing),
        "the capture is told, not merely forgotten"
    );
}

// The status must never claim a capture that did not start. This is the
// whole reason `behind` asks the pipeline before it writes the status.
#[test]
fn a_capture_that_refuses_leaves_the_status_alone() {
    let refusing = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: Default::default(),
        levels: Default::default(),
        shown: Default::default(),
        counting: Default::default(),
        published: Default::default(),
        recorded: Default::default(),
        mirrored: Default::default(),
        gated: Default::default(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: Default::default(),
        refuse: Some("macOS refused: screen recording".into()),
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(refusing));
    let Reply::Error { message } = engine.handle(Command::Screen { display: 3 }) else {
        panic!("a refused capture is an error")
    };
    assert!(message.contains("refused"), "{message}");
    assert_eq!(
        engine.status().screen,
        None,
        "the status must not claim a capture that never started"
    );
}

#[test]
fn what_is_flowing_is_measured_not_assumed() {
    let (mut engine, _) = machine_with_pipeline();
    engine.handle(Command::Screen { display: 1 });
    assert_eq!(
        engine.flowing(),
        Flowing {
            captured: 7,
            frames: 42,
            width: 1920,
            height: 1080,
            held: None
        }
    );
}

/// Every card the engine put up, in order, as the fake pipeline saw them.
type Shown = std::sync::Arc<std::sync::Mutex<Vec<(Card, String)>>>;

/// Where the faders were last put: microphone, music, duck, muted.
type Faders = std::sync::Arc<std::sync::Mutex<Option<(f64, f64, f64, bool, bool, bool)>>>;

// What the Mac plays stays off the air until somebody says so, reaches the
// mixer the moment they do, and the panic button takes it back: nothing of
// the room reaches the audience after that button until it is asked again.
#[test]
fn the_screens_sound_is_off_the_air_until_asked_and_the_panic_button_takes_it_back() {
    let levels: Faders = Default::default();
    let pipeline = Wrote {
        levels: levels.clone(),
        ..Default::default()
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
    assert!(
        !engine.status().screen_sound,
        "the screen is chosen, its sound is not"
    );

    let Reply::Status(sent) = engine.handle(Command::ScreenSound { on: true }) else {
        panic!("a switch every face reads answers with a status")
    };
    assert!(sent.screen_sound);
    assert_eq!(
        levels.lock().expect("levels").map(|l| l.5),
        Some(true),
        "the mixer was told to let the screen's sound into the mix"
    );

    engine.handle(Command::HideEverything);
    assert!(
        !engine.status().screen_sound,
        "the panic button takes it back"
    );
    assert_eq!(levels.lock().expect("levels").map(|l| l.5), Some(false));
}

fn cards_shown(log: &Shown) -> Vec<(Card, String)> {
    log.lock().expect("shown").clone()
}

fn machine_watching_cards() -> (Engine, Shown) {
    let shown: Shown = Default::default();
    let pipeline = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: Default::default(),
        levels: Default::default(),
        shown: shown.clone(),
        counting: Default::default(),
        published: Default::default(),
        recorded: Default::default(),
        mirrored: Default::default(),
        gated: Default::default(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: Default::default(),
        refuse: None,
    };
    (
        Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline)),
        shown,
    )
}

#[test]
fn a_card_reaches_the_picture_with_the_words_on_it() {
    let (mut engine, shown) = machine_watching_cards();
    engine.handle(Command::Card {
        which: Card::StartingSoon,
    });
    assert_eq!(
        cards_shown(&shown),
        vec![(Card::StartingSoon, "Starting soon".into())]
    );
    assert_eq!(engine.status().card, Some(Card::StartingSoon));
}

#[test]
fn changing_the_words_changes_a_card_that_is_already_up() {
    let (mut engine, shown) = machine_watching_cards();
    engine.handle(Command::Card {
        which: Card::StartingSoon,
    });
    engine.handle(Command::CardText {
        which: Card::StartingSoon,
        text: "Chegando já".into(),
    });
    assert_eq!(
        cards_shown(&shown).last(),
        Some(&(Card::StartingSoon, "Chegando já".into())),
        "a card already up must change at once, not the next time it is pressed"
    );
}

#[test]
fn changing_the_words_of_a_card_that_is_down_does_not_put_it_up() {
    let (mut engine, shown) = machine_watching_cards();
    engine.handle(Command::CardText {
        which: Card::BackInAMoment,
        text: "Já volto".into(),
    });
    assert!(
        cards_shown(&shown).is_empty(),
        "nothing should have been shown"
    );
    assert_eq!(engine.status().card, None);
}

#[test]
fn the_engines_own_line_about_itself_is_not_the_operators_to_change() {
    let (mut engine, _) = machine_watching_cards();
    assert!(matches!(
        engine.handle(Command::CardText {
            which: Card::NothingShared,
            text: "whatever".into()
        }),
        Reply::Error { .. }
    ));
}

// A viewer looking at a frozen frame cannot tell a deliberate blank from a
// stream that died, so the engine says which it is.
#[test]
fn turning_the_screen_off_puts_up_the_engines_own_card() {
    let (mut engine, shown) = machine_watching_cards();
    engine.handle(Command::Screen { display: 3 });
    engine.handle(Command::Share { on: false });
    assert_eq!(
        cards_shown(&shown).last(),
        Some(&(Card::NothingShared, "No content shared".into()))
    );
}

// The status says what the operator chose. A client drawing "Back in a
// moment" as selected because the screen happens to be off would be lying.
#[test]
fn the_engines_own_card_is_not_reported_as_the_operators_choice() {
    let (mut engine, _) = machine_watching_cards();
    engine.handle(Command::Share { on: false });
    assert_eq!(engine.status().card, None);
}

#[test]
fn choosing_a_source_again_takes_the_engines_card_down() {
    let (mut engine, shown) = machine_watching_cards();
    engine.handle(Command::Share { on: false });
    engine.handle(Command::Screen { display: 3 });
    assert_eq!(
        cards_shown(&shown).last(),
        Some(&(Card::Live, String::new())),
        "the engine takes its own card down when there is a picture again"
    );
}

// An operator who asked for "Back in a moment" gets it, screen or no
// screen. Theirs outranks the engine's.
#[test]
fn an_operators_card_outranks_the_engines() {
    let (mut engine, shown) = machine_watching_cards();
    engine.handle(Command::Card {
        which: Card::BackInAMoment,
    });
    engine.handle(Command::Share { on: false });
    assert_eq!(
        cards_shown(&shown).last(),
        Some(&(Card::BackInAMoment, "Back in a moment".into())),
        "turning the screen off must not replace the card they chose"
    );
    assert_eq!(engine.status().card, Some(Card::BackInAMoment));
}

// The moment they press "Live" thinking the show is starting is the worst
// possible moment for the picture to go empty, and the only sign would be
// a viewer saying it froze.
#[test]
fn taking_a_card_down_with_nothing_behind_the_picture_leaves_the_blank_up() {
    let (mut engine, shown) = machine_watching_cards();
    engine.handle(Command::Card {
        which: Card::StartingSoon,
    });
    engine.handle(Command::Card { which: Card::Live });
    assert_eq!(
        cards_shown(&shown).last(),
        Some(&(Card::NothingShared, "No content shared".into())),
        "with no screen chosen, taking the card down must not leave the picture empty"
    );
    assert_eq!(
        engine.status().card,
        None,
        "and it is still not their choice"
    );
}

#[test]
fn taking_a_card_down_with_a_screen_behind_it_shows_the_screen() {
    let (mut engine, shown) = machine_watching_cards();
    engine.handle(Command::Screen { display: 3 });
    engine.handle(Command::Card {
        which: Card::BackInAMoment,
    });
    engine.handle(Command::Card { which: Card::Live });
    assert_eq!(
        cards_shown(&shown).last(),
        Some(&(Card::Live, String::new())),
        "there is a screen to go back to"
    );
}

fn machine_watching_the_clock() -> (
    Engine,
    Shown,
    std::sync::Arc<std::sync::Mutex<Option<Duration>>>,
) {
    let shown: Shown = Default::default();
    let counting: std::sync::Arc<std::sync::Mutex<Option<Duration>>> = Default::default();
    let pipeline = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: Default::default(),
        levels: Default::default(),
        shown: shown.clone(),
        counting: counting.clone(),
        published: Default::default(),
        recorded: Default::default(),
        mirrored: Default::default(),
        gated: Default::default(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: Default::default(),
        refuse: None,
    };
    (
        Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_destination(Some(DESTINATION.into())),
        shown,
        counting,
    )
}

// Nobody presses "start the countdown" wanting to stay on the picture they
// are already showing.
#[test]
fn starting_the_countdown_puts_the_starting_card_up() {
    let (mut engine, shown, counting) = machine_watching_the_clock();
    engine.handle(Command::Countdown { seconds: None });
    assert_eq!(engine.status().card, Some(Card::StartingSoon));
    assert_eq!(
        cards_shown(&shown).last().map(|(card, _)| *card),
        Some(Card::StartingSoon)
    );
    assert_eq!(
        *counting.lock().expect("counting"),
        Some(Duration::from_secs(DEFAULT_COUNTDOWN_SECONDS as u64))
    );
}

#[test]
fn the_countdown_is_as_long_as_the_operator_says() {
    let (mut engine, _, counting) = machine_watching_the_clock();
    engine.handle(Command::Countdown { seconds: Some(45) });
    assert_eq!(
        *counting.lock().expect("counting"),
        Some(Duration::from_secs(45))
    );
}

// The countdown belongs to the starting card. Putting another card up, or
// taking it down, stops the clock rather than leaving it running behind
// something else.
#[test]
fn another_card_stops_the_clock() {
    let (mut engine, _, counting) = machine_watching_the_clock();
    engine.handle(Command::Countdown { seconds: Some(60) });
    engine.handle(Command::Card {
        which: Card::BackInAMoment,
    });
    assert_eq!(*counting.lock().expect("counting"), None);
}

#[test]
fn the_countdown_carries_the_operators_own_words() {
    let (mut engine, shown, _) = machine_watching_the_clock();
    engine.handle(Command::CardText {
        which: Card::StartingSoon,
        text: "Começa já".into(),
    });
    engine.handle(Command::Countdown { seconds: Some(30) });
    assert_eq!(
        cards_shown(&shown).last(),
        Some(&(Card::StartingSoon, "Começa já".into())),
        "the clock is added by the picture; the words are the operator's"
    );
}

// Four things at once, and it is one button because it is pressed in the
// moment somebody walks into the room.
#[test]
fn hiding_everything_reaches_the_picture_and_not_only_the_status() {
    let (mut engine, shown, _) = machine_watching_the_clock();
    engine.handle(Command::Screen { display: 3 });
    engine.handle(Command::Camera {
        device: Some("webcam".into()),
    });
    engine.handle(Command::GoLive);

    engine.handle(Command::HideEverything);

    assert_eq!(
        cards_shown(&shown).last(),
        Some(&(Card::BackInAMoment, "Back in a moment".into())),
        "the card has to actually be drawn, not only recorded"
    );
    let status = engine.status();
    assert!(
        status.muted,
        "hiding everything means hiding your voice too"
    );
    assert_eq!(status.camera, None);
    assert_eq!(status.screen, None);
    assert_eq!(status.card, Some(Card::BackInAMoment));
    assert!(status.on_air, "it is a break, not the end of the live");
}

// The card goes up before the screen goes away, so there is never a frame
// with nothing to show in it.
#[test]
fn the_card_is_up_before_the_screen_is_gone() {
    let (mut engine, shown, _) = machine_watching_the_clock();
    engine.handle(Command::Screen { display: 3 });
    engine.handle(Command::HideEverything);
    let cards = cards_shown(&shown);
    assert_eq!(
        cards.len(),
        1,
        "exactly one card, and no blank slipping in behind it: {cards:?}"
    );
    assert_eq!(cards[0].0, Card::BackInAMoment);
}

/// A music folder: three genres,
/// tracks named the way stream-safe libraries name them.
struct ThreeGenres;

impl Library for ThreeGenres {
    fn playlists(&self) -> Vec<Playlist> {
        ["lofi", "edm", "synthwave"]
            .iter()
            .map(|name| Playlist {
                name: (*name).to_string(),
                title: crate::music::genre_title(name),
                tracks: (0..6)
                    .map(|n| Track {
                        title: format!("{name} {n}"),
                        artist: Some("StreamBeats".into()),
                        url: format!("/music/{name}/{n}.mp3"),
                    })
                    .collect(),
            })
            .collect()
    }
}

/// Every file the pipeline was told to play, and a `None` for every stop.
type Played = std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>;
/// The switch a test flips to say the track that was playing has run out.
type RanOut = std::sync::Arc<std::sync::Mutex<bool>>;

/// An engine with three genres, what it played, and that switch.
fn machine_with_music() -> (Engine, Played, RanOut) {
    let played: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>> = Default::default();
    let ran_out: std::sync::Arc<std::sync::Mutex<bool>> = Default::default();
    let pipeline = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: played.clone(),
        levels: Default::default(),
        shown: Default::default(),
        counting: Default::default(),
        published: Default::default(),
        recorded: Default::default(),
        mirrored: Default::default(),
        gated: Default::default(),
        heard_here: Default::default(),
        speaker_calls: Default::default(),
        previewed: Default::default(),
        ran_out: ran_out.clone(),
        refuse: None,
    };
    (
        Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_library(Box::new(ThreeGenres)),
        played,
        ran_out,
    )
}

// Nobody picks a genre in order to not hear it, so choosing one starts it.
#[test]
fn choosing_a_genre_starts_playing_it() {
    let (mut engine, played, _) = machine_with_music();
    let Reply::Status(status) = engine.handle(Command::Genre {
        name: "lofi".into(),
    }) else {
        panic!("choosing a genre answers a status")
    };
    assert!(
        status
            .music
            .as_deref()
            .unwrap_or_default()
            .starts_with("StreamBeats — lofi"),
        "the status says what is playing, got {:?}",
        status.music
    );
    assert_eq!(played.lock().expect("played").len(), 1);
}

#[test]
fn asking_for_a_genre_that_is_not_there_says_what_is() {
    let (mut engine, _, _) = machine_with_music();
    let Reply::Error { message } = engine.handle(Command::Genre {
        name: "drum-and-bass".into(),
    }) else {
        panic!("an absent genre is an error")
    };
    assert!(message.contains("lofi, edm and synthwave"), "{message}");
}

// Eight hours on twenty-five tracks is only bearable because the rotation
// refuses to repeat what it played recently.
#[test]
fn the_next_track_is_never_the_one_just_played() {
    let (mut engine, played, _) = machine_with_music();
    engine.handle(Command::Genre { name: "edm".into() });
    for _ in 0..5 {
        engine.handle(Command::NextTrack);
    }
    let played = played.lock().expect("played").clone();
    assert_eq!(played.len(), 6);
    for pair in played.windows(2) {
        assert_ne!(
            pair[0], pair[1],
            "a track came back immediately: {played:?}"
        );
    }
}

// A bed that plays one file and then goes quiet is worse than no bed at
// all: the panel still names the track, so nobody notices until somebody
// watching says the stream went silent.
// The panic button is pressed when somebody walks into the room. What it
// has to guarantee is that nothing of yours reaches anybody, and half a
// guarantee is not one.
#[test]
fn the_panic_button_leaves_nothing_running() {
    let (mut engine, played, _) = machine_with_music();
    engine.handle(Command::Genre { name: "edm".into() });
    engine.handle(Command::Mic {
        device: Some("HyperX".into()),
    });
    engine.handle(Command::Camera {
        device: Some("MacBook".into()),
    });
    engine.handle(Command::Monitor { on: true });

    engine.handle(Command::HideEverything);

    let now = engine.status();
    assert_eq!(
        now.card,
        Some(Card::BackInAMoment),
        "the card goes up first"
    );
    assert_eq!(now.camera, None, "the camera is off");
    assert_eq!(now.screen, None, "the screen is off");
    assert!(now.muted, "the microphone is muted");
    assert_eq!(
        now.mic, None,
        "and closed: a muted microphone is still an open device"
    );
    assert_eq!(now.music, None, "the music stops");
    assert!(!now.monitoring, "the speakers stop");
    assert!(
        !now.music_to_stream,
        "and the bed stays off the stream until somebody says otherwise"
    );
    // It reached the pipeline and not only the status: a `None` at the end
    // of what was played is the mixer being told to stop.
    assert_eq!(played.lock().expect("played").last(), Some(&None));
}

#[test]
fn a_track_that_runs_out_starts_the_next_one() {
    let (mut engine, played, ran_out) = machine_with_music();
    engine.handle(Command::Genre { name: "edm".into() });
    assert_eq!(played.lock().expect("played").len(), 1);

    *ran_out.lock().expect("ran out") = true;
    engine.tick();
    let played = played.lock().expect("played").clone();
    assert_eq!(played.len(), 2, "the next track never started: {played:?}");
    assert_ne!(played[0], played[1], "it played the same file again");
}

#[test]
fn a_tick_with_nothing_finished_starts_nothing() {
    let (mut engine, played, _) = machine_with_music();
    engine.handle(Command::Genre { name: "edm".into() });
    for _ in 0..10 {
        engine.tick();
    }
    assert_eq!(played.lock().expect("played").len(), 1);
}

// Stopping the music and a track running out arrive in either order, and
// the one that arrives second must not restart what was just stopped.
#[test]
fn a_track_running_out_after_stop_stays_stopped() {
    let (mut engine, played, ran_out) = machine_with_music();
    engine.handle(Command::Genre { name: "edm".into() });
    engine.handle(Command::Music { on: false });
    let before = played.lock().expect("played").len();

    *ran_out.lock().expect("ran out") = true;
    engine.tick();
    assert_eq!(played.lock().expect("played").len(), before);
}

#[test]
fn skipping_with_nothing_playing_says_so_rather_than_guessing() {
    let (mut engine, _, _) = machine_with_music();
    assert!(matches!(
        engine.handle(Command::NextTrack),
        Reply::Error { .. }
    ));
}

// The panel's music control is a switch, not a question. Turning it on
// without a genre chosen picks one rather than refusing.
#[test]
fn turning_music_on_with_no_genre_chosen_picks_one() {
    let (mut engine, played, _) = machine_with_music();
    engine.handle(Command::Music { on: true });
    assert!(engine.status().music.is_some());
    assert_eq!(played.lock().expect("played").len(), 1);
}

#[test]
fn turning_music_off_stops_it_and_says_nothing_is_playing() {
    let (mut engine, played, _) = machine_with_music();
    engine.handle(Command::Genre {
        name: "lofi".into(),
    });
    engine.handle(Command::Music { on: false });
    assert_eq!(engine.status().music, None);
    assert_eq!(
        played.lock().expect("played").last(),
        Some(&None),
        "stopping is telling the pipeline, not only forgetting"
    );
}

#[test]
fn an_engine_with_no_music_folder_says_so_rather_than_pretending() {
    let mut engine = Engine::new();
    assert!(matches!(
        engine.handle(Command::Music { on: true }),
        Reply::Error { .. }
    ));
}

// ---- what cargo-mutants found standing after the split, and the tests that
// ---- say why each line is the way it is.

fn speaking_engine() -> (Engine, std::sync::Arc<std::sync::Mutex<Vec<bool>>>) {
    let speaker_calls: std::sync::Arc<std::sync::Mutex<Vec<bool>>> = Default::default();
    let pipeline = Wrote {
        speaker_calls: speaker_calls.clone(),
        ..Default::default()
    };
    (
        Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_library(Box::new(ThreeGenres)),
        speaker_calls,
    )
}

// Off closes the speakers the music opened; with the speakers already closed
// there is nothing to close, and telling them so anyway is a call an output
// that refuses would turn into an error for a switch that did nothing.
#[test]
fn turning_the_music_off_with_the_speakers_already_closed_leaves_them_alone() {
    let (mut engine, calls) = speaking_engine();
    engine.handle(Command::Genre { name: "edm".into() });
    engine.handle(Command::Monitor { on: false });
    assert_eq!(*calls.lock().expect("calls"), vec![true, false]);
    engine.handle(Command::Music { on: false });
    assert_eq!(
        *calls.lock().expect("calls"),
        vec![true, false],
        "the speakers were not told again"
    );
}

// The speakers open with the first genre and not with every one, so a person
// who turned them off mid-set is not argued with at the next shelf either.
#[test]
fn picking_another_genre_while_already_hearing_it_does_not_open_the_speakers_again() {
    let (mut engine, calls) = speaking_engine();
    engine.handle(Command::Genre {
        name: "lofi".into(),
    });
    engine.handle(Command::Genre { name: "edm".into() });
    assert_eq!(*calls.lock().expect("calls"), vec![true]);
}

// A genre that could not start opens nothing: the speakers follow the music,
// not the attempt.
#[test]
fn a_genre_that_refuses_to_play_opens_no_speakers() {
    let calls: std::sync::Arc<std::sync::Mutex<Vec<bool>>> = Default::default();
    let pipeline = Wrote {
        speaker_calls: calls.clone(),
        refuse: Some("no output".into()),
        ..Default::default()
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine))
        .with_pipeline(Box::new(pipeline))
        .with_library(Box::new(ThreeGenres));
    let reply = engine.handle(Command::Genre {
        name: "lofi".into(),
    });
    assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
    assert!(calls.lock().expect("calls").is_empty());
}

// Rewording a card re-shows it only when it is the one up: rewording the
// other one must not put anything back on the picture.
#[test]
fn rewording_the_card_that_is_not_up_changes_nothing_on_the_picture() {
    let shown: Shown = Default::default();
    let pipeline = Wrote {
        shown: shown.clone(),
        ..Default::default()
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
    engine.handle(Command::Card {
        which: Card::StartingSoon,
    });
    let before = shown.lock().expect("shown").len();
    let reply = engine.handle(Command::CardText {
        which: Card::BackInAMoment,
        text: "back in five".into(),
    });
    assert!(matches!(reply, Reply::Status(_)));
    assert_eq!(
        shown.lock().expect("shown").len(),
        before,
        "nothing was re-shown"
    );
    assert_eq!(engine.status().card, Some(Card::StartingSoon));
    assert_eq!(engine.status().words.back, "back in five");
}

// An engine with no app behind it has no server to name and nothing to
// announce, and its log carries no notice from anybody.
#[test]
fn an_engine_with_no_app_has_no_server_no_notices_and_nothing_to_announce() {
    let mut engine = engine();
    engine.tick();
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert_eq!(status.server, None);
    assert!(
        status.log.iter().all(|line| !line.contains("! ")),
        "no notice from nobody: {:?}",
        status.log
    );
    let reply = engine.handle(Command::Announce { adapter: 1 });
    assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
}

// The engine that captures nothing is honest about it: nothing to hear, and
// nothing to write, even with a folder to write into.
#[test]
fn an_engine_that_captures_nothing_refuses_the_speakers_and_the_file() {
    let mut engine = Engine::new().with_recordings(Some("/tmp/films".into()));
    assert_eq!(
        engine.status().record_dir.as_deref(),
        None,
        "read, never kept"
    );
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert_eq!(status.record_dir.as_deref(), Some("/tmp/films"));
    let reply = engine.handle(Command::RecordStart);
    assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
    assert!(!engine.status().recording);
    let reply = engine.handle(Command::Monitor { on: true });
    assert!(matches!(reply, Reply::Error { .. }), "got {reply:?}");
    assert!(!engine.status().monitoring);
}

// A destination handed over after the fact is the destination: the refusal
// moves from "nowhere to send" to "no picture yet".
#[test]
fn a_destination_set_later_is_the_one_go_live_uses() {
    let mut engine = engine();
    let Reply::Error { message } = engine.handle(Command::GoLive) else {
        panic!("nowhere to send is an error")
    };
    assert!(message.contains("nowhere"), "{message}");
    engine.set_destination(Some("rtmp://hub/scene".into()));
    let Reply::Error { message } = engine.handle(Command::GoLive) else {
        panic!("no picture is an error")
    };
    assert!(message.contains("no picture"), "{message}");
}

// The clock the engine stamps with is the real one: a live that started
// reports a moment in this decade, not zero.
#[test]
fn the_engine_stamps_with_a_real_clock() {
    let (mut engine, _) = publishing_engine(None);
    engine.handle(Command::Screen { display: 1 });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    let since = engine.status().on_air_since.expect("on air since");
    assert!(
        since > 1_700_000_000,
        "{since} is not a moment on the real clock"
    );
}

// A track never ends on an engine that plays nothing, so the tick leaves the
// music where it is.
#[test]
fn a_track_never_ends_where_nothing_plays() {
    let mut engine = Engine::new().with_library(Box::new(ThreeGenres));
    engine.handle(Command::Genre {
        name: "lofi".into(),
    });
    let playing = engine.status().music.clone();
    assert!(playing.is_some());
    engine.tick();
    engine.tick();
    assert_eq!(
        engine.status().music,
        playing,
        "the tick did not skip a track"
    );
}

// The readings a panel asks for in dB come off the faders, and the speakers
// read off the status.
#[test]
fn the_faders_read_in_db_and_the_speakers_read_off_the_status() {
    let (mut engine, _) = speaking_engine();
    assert_eq!(engine.music_db(), crate::music::fader_db(0.85));
    assert_eq!(engine.mic_db(), crate::music::fader_db(1.0));
    assert_eq!(engine.duck_db(), crate::music::DUCK_DEFAULT_DB);
    assert!(!engine.monitoring());
    engine.handle(Command::MusicVolume { level: 0.25 });
    assert_eq!(engine.music_db(), crate::music::fader_db(0.25));
    engine.handle(Command::Genre {
        name: "lofi".into(),
    });
    assert!(engine.monitoring());
}

// ---- the panel and the engine leave together

#[test]
fn an_engine_no_panel_has_spoken_to_runs_until_it_is_told() {
    let mut engine = engine();
    for _ in 0..(PANEL_LEASE_TICKS * 5) {
        engine.tick();
    }
    assert!(!engine.quitting(), "nothing told it to stop");
}

#[test]
fn a_panel_that_stops_saying_it_is_here_stops_the_engine() {
    let mut engine = engine();
    assert_eq!(engine.handle(Command::Present), Reply::Ok);
    for _ in 0..(PANEL_LEASE_TICKS - 1) {
        engine.tick();
    }
    assert!(!engine.quitting(), "still within the lease");
    engine.tick();
    assert!(engine.quitting(), "the lease ran out");
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert!(
        status
            .log
            .iter()
            .any(|line| line.contains("the panel went away")),
        "and the log says why: {:?}",
        status.log
    );
}

#[test]
fn a_panel_that_keeps_saying_it_is_here_keeps_the_engine() {
    let mut engine = engine();
    for _ in 0..10 {
        engine.handle(Command::Present);
        for _ in 0..(PANEL_LEASE_TICKS - 5) {
            engine.tick();
        }
    }
    assert!(!engine.quitting());
}

#[test]
fn saying_so_writes_nothing_in_the_log() {
    let mut engine = engine();
    engine.handle(Command::Present);
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert!(
        status.log.is_empty(),
        "once a second would push everything else off: {:?}",
        status.log
    );
}

// ---- a device plugged in after the window opened

struct Replugged(std::sync::Arc<std::sync::atomic::AtomicU64>);

impl Sources for Replugged {
    fn available(&self) -> Result<Available, String> {
        ThisMachine.available()
    }
    fn generation(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[test]
fn the_status_says_when_the_device_list_moved_so_a_face_reads_it_again() {
    let plugged = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut engine = Engine::with_sources(Box::new(Replugged(plugged.clone())));
    let Reply::Status(before) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert_eq!(before.devices_generation, 0);
    plugged.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let Reply::Status(after) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert_eq!(after.devices_generation, 1, "a headset came back");
}

#[test]
fn an_engine_with_nothing_plugged_in_never_moves_the_list() {
    let mut engine = engine();
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status answers with a status")
    };
    assert_eq!(status.devices_generation, 0);
}

// ---- watching the channel's own page without hearing the bed twice

// Choosing a genre opens the speakers. Closing them again must leave the
// bed exactly where it was for the audience: still playing, still sent.
#[test]
fn the_speakers_close_while_the_bed_keeps_going_out() {
    let (mut engine, calls) = speaking_engine();
    engine.handle(Command::Genre {
        name: "lofi".into(),
    });
    assert!(engine.status().monitoring);
    let Reply::Status(after) = engine.handle(Command::Monitor { on: false }) else {
        panic!("a switch every face reads answers with a status")
    };
    assert!(!after.monitoring, "the speakers are closed");
    assert!(after.music.is_some(), "the bed is still playing");
    assert!(after.music_to_stream, "and still goes out");
    assert_eq!(*calls.lock().expect("calls"), vec![true, false]);
}

// ---- the app knows where the scene goes

/// An app that names the relay, the way a signed-in one does.
struct NamesTheRelay;

impl Watching for NamesTheRelay {
    fn reachable(&self) -> bool {
        true
    }
    fn destinations(&self) -> Vec<Destination> {
        Vec::new()
    }
    fn viewers(&self) -> Option<u32> {
        None
    }
    fn arm(&self, _: i64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn sandbox(&self, _: i64, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn retitle(&self, _: i64, _: Option<&str>, _: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn announce(&self, _: i64) -> Result<(), String> {
        Ok(())
    }
    fn disconnect(&self, _: i64) -> Result<(), String> {
        Ok(())
    }
    fn categorize(&self, _: i64, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn search_categories(&self, _: i64, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn scene(&self) -> Option<String> {
        Some("rtmp://hub:1935/scene?user=remux&pass=token".into())
    }
}

// An engine started with no destination and signed in to an app goes live
// where the app says the scene goes: nobody read a key out of a database
// to start it.
#[test]
fn an_engine_with_no_destination_of_its_own_goes_live_where_the_app_says() {
    let published: Published = Default::default();
    let pipeline = Wrote {
        published: published.clone(),
        ..Default::default()
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine))
        .with_pipeline(Box::new(pipeline))
        .with_app(Box::new(NamesTheRelay));
    engine.handle(Command::Screen { display: 1 });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    assert_eq!(
        *published.lock().expect("published"),
        vec![Some(
            "rtmp://hub:1935/scene?user=remux&pass=token".to_string()
        )]
    );
}

// And one started with a destination keeps it: what the operator said wins
// over what the app would have said.
#[test]
fn a_destination_of_its_own_outranks_the_app_s() {
    let (mut engine, published) = publishing_engine(None);
    engine = engine.with_app(Box::new(NamesTheRelay));
    engine.handle(Command::Screen { display: 1 });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    assert_eq!(
        *published.lock().expect("published"),
        vec![Some(DESTINATION.to_string())]
    );
}
