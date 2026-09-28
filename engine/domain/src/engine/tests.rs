use super::*;
use crate::layers::{Kind, Layer, Source, Transform};
use crate::protocol::Category;
use crate::scenes::Scene;
use crate::sources::{DisplayId, WindowId};

fn engine() -> Engine {
    Engine::new()
}

#[test]
fn independent_audio_layers_are_addressed_by_id_and_remembered() {
    use crate::audio_layers::Source;
    let mut engine = engine();
    for id in ["browser", "editor"] {
        assert!(matches!(
            engine.handle(Command::AudioLayerAdd {
                id: id.into(),
                source: Source::app("Safari".into()),
            }),
            Reply::Status(_)
        ));
    }
    assert_eq!(engine.status().audio_layers.len(), 2);
    assert!(matches!(
        engine.handle(Command::AudioLayerVolume {
            id: "editor".into(),
            volume: 0.25
        }),
        Reply::Status(_)
    ));
    assert!(matches!(
        engine.handle(Command::AudioLayerMute {
            id: "browser".into(),
            on: true
        }),
        Reply::Status(_)
    ));
    assert_eq!(engine.status().audio_layers[0].volume, 1.0);
    assert!(engine.status().audio_layers[0].muted);
    assert_eq!(engine.status().audio_layers[1].volume, 0.25);
    assert!(!engine.status().audio_layers[1].muted);
    assert!(matches!(
        engine.handle(Command::AudioLayerAdd {
            id: "browser".into(),
            source: Source::mic("mic".into()),
        }),
        Reply::Error { .. }
    ));
    assert!(matches!(
        engine.handle(Command::AudioLayerVolume {
            id: "editor".into(),
            volume: f64::NAN
        }),
        Reply::Error { .. }
    ));
    let setup = engine.remembered();
    let mut restored = Engine::new();
    restored.restore(&setup);
    assert_eq!(restored.status().audio_layers, engine.status().audio_layers);
    assert!(matches!(
        engine.handle(Command::AudioLayerRemove {
            id: "browser".into()
        }),
        Reply::Status(_)
    ));
    assert_eq!(engine.status().audio_layers.len(), 1);
}

/// A machine with two monitors and three windows: enough shape for every
/// decision the engine makes about sources. The built-in is display 1 and
/// the monitor is display 3, an ordering a real machine produces and the
/// reason position is never the handle.
pub(super) struct ThisMachine;

impl Sources for ThisMachine {
    fn available(&self) -> Result<Available, String> {
        Ok(Available {
            screens: vec![
                Screen {
                    id: DisplayId(1),
                    name: "Built-in Retina Display".into(),
                    stable: Some("37D8832A".into()),
                },
                Screen {
                    id: DisplayId(3),
                    name: "VG2791R".into(),
                    stable: Some("5C1E09B4".into()),
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

#[test]
fn camera_shape_and_position_belong_to_one_named_layer() {
    let (mut engine, _, _) = machine_with_music();
    assert!(crate::layers::camera(&engine.status().layers, None).is_err());
    engine.handle(Command::LayerCamera {
        id: "left".into(),
        device: "HP".into(),
    });
    assert_eq!(
        crate::layers::camera(&engine.status().layers, None)
            .unwrap()
            .id,
        "left"
    );
    assert!(matches!(
        engine.handle(Command::CameraShape {
            shape: crate::scene::CameraShape::Circle
        }),
        Reply::Status(_)
    ));
    assert_eq!(
        engine.status().layers[0].shape,
        Some(crate::scene::CameraShape::Circle)
    );
    engine.handle(Command::LayerCamera {
        id: "right".into(),
        device: "MacBook".into(),
    });
    assert!(crate::layers::camera(&engine.status().layers, None).is_err());
    assert!(matches!(
        engine.handle(Command::CameraShape {
            shape: crate::scene::CameraShape::Rectangle
        }),
        Reply::Error { .. }
    ));
    assert!(matches!(
        engine.handle(Command::CameraPosition { at: None }),
        Reply::Error { .. }
    ));
    let before = engine.status().layers[1].transform;
    assert!(matches!(
        engine.handle(Command::LayerShape {
            id: "missing".into(),
            shape: crate::scene::CameraShape::Circle
        }),
        Reply::Error { .. }
    ));
    assert!(matches!(
        engine.handle(Command::LayerPosition {
            id: "missing".into(),
            at: None
        }),
        Reply::Error { .. }
    ));
    assert!(matches!(
        engine.handle(Command::LayerShape {
            id: "right".into(),
            shape: crate::scene::CameraShape::Circle
        }),
        Reply::Status(_)
    ));
    assert_eq!(
        engine.status().layers[0].shape,
        Some(crate::scene::CameraShape::Circle)
    );
    assert_eq!(
        engine.status().layers[1].shape,
        Some(crate::scene::CameraShape::Circle)
    );
    engine.handle(Command::LayerPosition {
        id: "right".into(),
        at: Some(crate::scene::CameraPosition { x: 1900, y: 1000 }),
    });
    assert_eq!(
        engine.status().layers[1].transform.x,
        (1920 - before.width) as i32
    );
    assert_eq!(
        engine.status().layers[1].transform.y,
        (1080 - before.height) as i32
    );
    assert_eq!(engine.status().layers[1].transform.width, before.width);
    assert_eq!(engine.status().layers[0].transform.x, 0);
    engine.handle(Command::LayerPosition {
        id: "right".into(),
        at: None,
    });
    assert_eq!(engine.status().layers[1].transform, before);
    engine.handle(Command::LayerWindow {
        id: "app".into(),
        query: "tmux".into(),
    });
    assert!(matches!(
        engine.handle(Command::LayerShape {
            id: "app".into(),
            shape: crate::scene::CameraShape::Circle
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status().layers[2].shape, None);
}

#[test]
fn display_layers_have_no_reserved_id_or_order() {
    let (mut engine, _, _) = machine_with_music();
    assert!(matches!(
        engine.handle(Command::LayerScreen {
            id: "left".into(),
            display: 99
        }),
        Reply::Error { .. }
    ));
    assert!(engine.status().layers.is_empty());
    assert!(matches!(
        engine.handle(Command::LayerScreen {
            id: "left".into(),
            display: 3
        }),
        Reply::Status(_)
    ));
    assert!(matches!(
        engine.handle(Command::LayerScreen {
            id: "right".into(),
            display: 1
        }),
        Reply::Status(_)
    ));
    assert_eq!(
        engine.status().layers[0].source.kind,
        crate::layers::Kind::Screen
    );
    assert_eq!(engine.status().layers[0].source.handle, "3");
    assert_eq!(engine.status().layers[0].source.name, "VG2791R");
    engine.handle(Command::LayerMove {
        id: "right".into(),
        index: 0,
    });
    assert_eq!(engine.status().layers[0].id, "right");
    engine.handle(Command::LayerRemove { id: "left".into() });
    assert_eq!(engine.status().layers[0].id, "right");
}

#[test]
fn layers_are_ordered_ephemeral_and_failed_capture_does_not_appear() {
    use crate::layers::{Kind, Transform};
    let (mut engine, _, _) = machine_with_music();
    engine.handle(Command::LayerCamera {
        id: "face".into(),
        device: "HP 430".into(),
    });
    engine.handle(Command::LayerWindow {
        id: "code".into(),
        query: "tmux".into(),
    });
    assert_eq!(engine.status().layers.len(), 2);
    assert_eq!(engine.status().layers[1].source.kind, Kind::Window);
    assert_eq!(engine.status().layers[1].source.handle, "10");
    assert_eq!(
        (
            engine.status().layers[0].source.width,
            engine.status().layers[0].source.height
        ),
        (1280, 720)
    );
    assert_eq!(
        engine.status().layers[1].transform,
        Transform::native((853, 479))
    );
    let transform = Transform {
        x: 100,
        y: 200,
        width: 800,
        height: 450,
        degrees: 90,
    };
    engine.handle(Command::LayerTransform {
        id: "face".into(),
        transform,
    });
    engine.handle(Command::LayerMove {
        id: "face".into(),
        index: 1,
    });
    assert_eq!(engine.status().layers[1].transform, transform);
    assert_eq!(engine.status().layers[1].id, "face");
    let crop = crate::layers::Crop {
        x: 10,
        y: 20,
        width: 600,
        height: 400,
    };
    engine.handle(Command::LayerCrop {
        id: "face".into(),
        crop: Some(crop),
    });
    assert_eq!(engine.status().layers[1].crop, Some(crop));
    assert!(matches!(
        engine.handle(Command::LayerCrop {
            id: "face".into(),
            crop: Some(crate::layers::Crop { x: 1000, ..crop })
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status().layers[1].crop, Some(crop));
    engine.handle(Command::LayerCrop {
        id: "face".into(),
        crop: None,
    });
    assert_eq!(engine.status().layers[1].crop, None);
    assert!(matches!(
        engine.handle(Command::LayerCamera {
            id: "face".into(),
            device: "HP".into()
        }),
        Reply::Error { .. }
    ));
    assert!(matches!(
        engine.handle(Command::LayerMove {
            id: "face".into(),
            index: 5
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status().layers.len(), 2);
    assert!(crate::remembered::write(&engine.remembered())
        .unwrap()
        .contains("face"));
    engine.handle(Command::LayerRemove { id: "code".into() });
    assert_eq!(engine.status().layers.len(), 1);
    engine.handle(Command::HideEverything);
    assert!(engine.status().layers.is_empty());
}

struct Refused;

impl Sources for Refused {
    fn available(&self) -> Result<Available, String> {
        Err("macOS refused: screen recording".into())
    }
}

fn unique_name(status: &Status, kinds: &[crate::layers::Kind]) -> Option<String> {
    let mut found = status
        .layers
        .iter()
        .filter(|l| kinds.contains(&l.source.kind));
    let first = found.next()?;
    found.next().is_none().then(|| first.source.name.clone())
}

fn screen_name(status: &Status) -> Option<String> {
    unique_name(
        status,
        &[crate::layers::Kind::Screen, crate::layers::Kind::Window],
    )
}

fn camera_name(status: &Status) -> Option<String> {
    unique_name(status, &[crate::layers::Kind::Camera])
}

// Setting up is the part nobody wants to do twice.
#[test]
fn what_was_chosen_comes_back_after_a_restart() {
    let (mut engine, played, _) = machine_with_music();
    engine.handle(Command::Camera {
        device: Some("HP".into()),
    });
    engine.handle(Command::Mic {
        device: Some("HyperX".into()),
    });
    engine.handle(Command::Mirror { on: true });
    engine.handle(Command::CameraShape {
        shape: crate::scene::CameraShape::Rectangle,
    });
    engine.handle(Command::CameraPosition {
        at: Some(crate::scene::CameraPosition { x: 360, y: 180 }),
    });
    engine.handle(Command::Volume { level: 1.5 });
    engine.handle(Command::Genre { name: "edm".into() });
    engine.handle(Command::SceneElementAdd {
        element: Element {
            id: "title".into(),
            x: 200,
            y: 200,
            width: 600,
            height: 100,
            visible: true,
            shader: None,
            content: ElementContent::Text {
                text: "back at nine".into(),
            },
        },
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
    assert_eq!(
        now.layers[0].shape,
        Some(crate::scene::CameraShape::Rectangle)
    );
    assert_eq!(
        (now.layers[0].transform.x, now.layers[0].transform.y),
        (360, 180)
    );
    assert_eq!(now.faders.mic, 1.5);
    assert_eq!(
        now.scenes[0].elements[0].content,
        ElementContent::Text {
            text: "back at nine".into()
        }
    );
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

// A display is put back by what the monitor is, not by the number it had:
// the VG2791R saved as display 2 is display 3 now, and a monitor that is not
// here is left out rather than swapped for the one that took its number.
#[test]
fn a_saved_display_comes_back_as_the_same_monitor_whatever_its_number() {
    let (mut engine, _, _) = machine_with_music();
    let desk = |id: &str, handle: &str, stable: &str| crate::layers::Layer {
        id: id.into(),
        source: crate::layers::Source {
            kind: crate::layers::Kind::Screen,
            handle: handle.into(),
            name: "a monitor".into(),
            width: 1920,
            height: 1080,
            stable: Some(stable.into()),
        },
        transform: crate::layers::Transform::native((1920, 1080)),
        visible: true,
        crop: None,
        shape: None,
        mirrored: false,
        shader: None,
    };
    engine.restore(&crate::remembered::Remembered {
        layers: vec![desk("desk", "2", "5C1E09B4"), desk("gone", "1", "0FF1CE00")],
        ..Default::default()
    });
    let layers = &engine.status().layers;
    assert_eq!(
        layers.len(),
        1,
        "the absent monitor is left out: {layers:?}"
    );
    assert_eq!(
        (layers[0].id.as_str(), layers[0].source.handle.as_str()),
        ("desk", "3")
    );
    assert_eq!(layers[0].source.name, "VG2791R");
    assert_eq!(layers[0].source.stable.as_deref(), Some("5C1E09B4"));
    assert_eq!(
        engine.remembered().layers[0].source.stable.as_deref(),
        Some("5C1E09B4"),
        "and it is kept by its identity again"
    );
}

// Losing a webcam is not a reason to lose the gate settings somebody spent
// an evening on.
#[test]
fn a_device_that_is_gone_does_not_take_the_rest_of_the_setup_with_it() {
    let (mut engine, _, _) = machine_with_music();
    let setup = crate::remembered::Remembered {
        layers: vec![crate::layers::Layer {
            id: "gone".into(),
            source: crate::layers::Source {
                kind: crate::layers::Kind::Camera,
                handle: "missing".into(),
                name: "a camera nobody has".into(),
                width: 1280,
                height: 720,
                stable: None,
            },
            transform: crate::layers::Transform::native((1280, 720)),
            visible: true,
            crop: None,
            shape: Some(crate::scene::CameraShape::Rectangle),
            mirrored: false,
            shader: None,
        }],
        mic: Some("HyperX".into()),
        mirrored: true,
        ..Default::default()
    };
    engine.restore(&setup);
    assert!(engine.status().layers.is_empty(), "it must not invent one");
    assert_eq!(engine.status().mic.as_deref(), Some("HyperX DuoCast"));
    assert!(engine.status().mirrored);
}

#[test]
fn restart_restores_named_window_layers_without_legacy_fields() {
    let mut engine =
        Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
    engine.handle(Command::LayerWindow {
        id: "notes".into(),
        query: "tmux".into(),
    });
    let saved = crate::remembered::write(&engine.remembered()).unwrap();
    assert!(saved.contains("Ghostty — tmux a"));
    assert!(!saved.contains("camera_position"));
    assert!(!saved.contains("\"screen\""));
    let mut next =
        Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
    next.restore(&crate::remembered::read(&saved));
    assert_eq!(next.status().layers.len(), 1);
    assert_eq!(next.status().layers[0].id, "notes");
    assert_eq!(
        next.status().layers[0].source.kind,
        crate::layers::Kind::Window
    );
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

// Both motors draw an empty scene at the full rate, so frames cannot keep
// black off the air: going live refuses what the plan says would stop it.
#[test]
fn an_empty_scene_cannot_go_live_though_it_has_frames() {
    let (mut engine, published) = publishing_engine(None);
    engine.handle(Command::Screen { display: 1 });
    let id = engine.status().layers[0].id.clone();
    engine.handle(Command::LayerVisible { id, on: false });
    assert_eq!(
        engine.handle(Command::GoLive),
        Reply::Error {
            message: "the scene is empty: nothing would be shared".into()
        }
    );
    assert!(!engine.status().on_air);
    assert!(published.lock().expect("published").is_empty());
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
fn fresh_engine_has_only_a_default_scene() {
    let engine = engine();
    assert_eq!(engine.status().scenes.len(), 1);
    assert_eq!(engine.status().active_scene, "default");
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
    assert_eq!(engine.status().active_scene, "default");
    assert!(engine.status().layers.is_empty());
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
    assert_eq!(screen_name(&status), Some("VG2791R".into()));
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
    assert!(engine.status().layers.is_empty(), "and change nothing");
}

#[test]
fn a_window_is_chosen_by_part_of_its_title() {
    let mut engine = machine();
    let Reply::Status(status) = engine.handle(Command::Window {
        query: "tmux".into(),
    }) else {
        panic!("choosing a window answers with the new status")
    };
    assert_eq!(screen_name(&status), Some("Ghostty — tmux a".into()));
}

// The CLI has always taken part of a name, and a browser window is titled
// after the page it is showing, so the application has to match too.
#[test]
fn a_window_is_found_by_the_application_that_owns_it() {
    let mut engine = machine();
    engine.handle(Command::Window {
        query: "brave".into(),
    });
    assert_eq!(
        screen_name(engine.status()),
        Some("Brave Browser — remux".into())
    );
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
        screen_name(engine.status()),
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
        screen_name(engine.status()),
        Some("Built-in Retina Display".into())
    );
    engine.handle(Command::Window {
        query: "notes".into(),
    });
    assert_eq!(
        screen_name(engine.status()),
        Some("TextEdit — notes".into())
    );
    engine.handle(Command::Screen { display: 3 });
    assert_eq!(screen_name(engine.status()), Some("VG2791R".into()));
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
    assert!(engine.status().layers.is_empty());

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
    assert_eq!(
        camera_name(engine.status()),
        Some("HP 430/435 FHD Webcam".into())
    );

    engine.handle(Command::Camera {
        device: Some("6C707041".into()),
    });
    assert_eq!(
        camera_name(engine.status()),
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
    assert!(camera_name(engine.status()).is_some());
    engine.handle(Command::Camera { device: None });
    assert!(camera_name(engine.status()).is_none());
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

#[test]
fn deleted_starter_scene_is_not_recreated_on_restore() {
    let mut engine = Engine::new();
    engine.handle(Command::SceneCreate {
        name: "Custom".into(),
    });
    engine.handle(Command::SceneDelete {
        name: "default".into(),
    });
    let saved = crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
    let mut next = Engine::new();
    next.restore(&saved);
    assert_eq!(next.status().scenes.len(), 1);
    assert_eq!(next.status().active_scene, "Custom");
}

#[test]
fn old_generated_presets_are_ignored_without_losing_custom_scenes() {
    let saved = crate::remembered::read(
        r#"{"active_scene":"BRB","scenes":[{"name":"default","layers":[]},{"name":"BRB","layers":[],"graphic":{"kind":"back-in-a-moment","text":"back"}},{"name":"My scene","layers":[],"elements":[{"id":"note","kind":"text","text":"Hi","x":2,"y":3,"width":100,"height":80}]}]}"#,
    );
    let mut engine = Engine::new();
    engine.restore(&saved);
    assert_eq!(engine.status().active_scene, "default");
    assert_eq!(engine.status().scenes.len(), 2);
    assert_eq!(engine.status().scenes[1].elements[0].id, "note");
    assert_eq!(engine.status().scenes[1].ordered_ids(), ["note"]);
}

pub(super) fn layer(id: &str, kind: Kind, handle: &str, x: i32) -> Layer {
    Layer {
        id: id.into(),
        source: Source {
            kind,
            handle: handle.into(),
            name: id.into(),
            width: 640,
            height: 480,
            stable: None,
        },
        transform: Transform {
            x,
            ..Transform::native((640, 480))
        },
        visible: true,
        crop: None,
        shape: None,
        shader: None,
        mirrored: false,
    }
}

#[test]
fn scene_switch_prepares_then_commits_and_closes_only_unshared_captures() {
    let fake = Wrote::default();
    let events = fake.scene_events.clone();
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    let before = vec![
        layer("camera", Kind::Camera, "cam", 0),
        layer("old", Kind::Screen, "1", 0),
    ];
    let after = vec![
        layer("closeup", Kind::Camera, "cam", 900),
        layer("new", Kind::Window, "2", 10),
    ];
    engine.status.layers = before.clone();
    engine.status.scenes.push(Scene {
        name: "next".into(),
        layers: after.clone(),
        elements: vec![],
        order: vec![],
        shader: None,
    });
    engine.status.on_air = true;
    engine.status.recording = true;
    assert!(matches!(
        engine.handle(Command::SceneSwitch {
            name: "next".into()
        }),
        Reply::Status(_)
    ));
    assert!(engine.status.on_air);
    assert!(engine.status.recording);
    assert_eq!(engine.status.layers, after);
    assert_eq!(engine.status.scenes[0].layers, before);
    assert_eq!(
        *events.lock().unwrap(),
        ["prepare Window:2", "commit layout", "close Screen:1"]
    );
    assert!(matches!(
        engine.handle(Command::SceneSwitch {
            name: "default".into()
        }),
        Reply::Status(_)
    ));
    assert_eq!(engine.status.layers, before);
}

#[test]
fn failed_preparation_preserves_active_layout_and_live() {
    let fake = Wrote {
        refuse: Some("capture unavailable".into()),
        ..Default::default()
    };
    let events = fake.scene_events.clone();
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    let old = layer("face", Kind::Camera, "cam", 0);
    engine.status.layers = vec![old.clone()];
    engine.status.on_air = true;
    engine.status.scenes.push(Scene {
        name: "other".into(),
        layers: vec![layer("new", Kind::Camera, "other-cam", 10)],
        elements: vec![],
        order: vec![],
        shader: None,
    });
    assert!(matches!(
        engine.handle(Command::SceneSwitch {
            name: "other".into()
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status.active_scene, "default");
    assert_eq!(engine.status.layers, [old]);
    assert!(engine.status.on_air);
    assert_eq!(
        *events.lock().unwrap(),
        ["prepare Camera:other-cam", "rollback prepared"]
    );
}

#[test]
fn second_capture_failure_rolls_back_prepared_source_before_touching_live() {
    let fake = Wrote {
        refuse: Some("scene:missing".into()),
        ..Default::default()
    };
    let events = fake.scene_events.clone();
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    let previous = layer("face", Kind::Camera, "camera", 0);
    engine.status.layers = vec![previous.clone()];
    engine.status.on_air = true;
    engine.status.scenes.push(Scene {
        name: "next".into(),
        layers: vec![
            layer("screen", Kind::Screen, "1", 0),
            layer("bad", Kind::Window, "missing", 0),
        ],
        elements: vec![],
        order: vec![],
        shader: None,
    });
    assert!(matches!(
        engine.handle(Command::SceneSwitch {
            name: "next".into()
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status.active_scene, "default");
    assert_eq!(engine.status.layers, [previous]);
    assert!(engine.status.on_air);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "prepare Screen:1",
            "prepare Window:missing",
            "rollback 1",
            "rollback prepared"
        ]
    );
}

#[test]
fn legacy_layers_restore_as_default_and_named_scenes_round_trip() {
    let old = layer("desktop", Kind::Screen, "1", 17);
    let legacy = crate::remembered::read(&serde_json::json!({"layers": [old]}).to_string());
    let mut engine = Engine::with_sources(Box::new(ThisMachine));
    engine.restore(&legacy);
    assert_eq!(engine.status.active_scene, "default");
    assert_eq!(engine.remembered().scenes[0].layers, engine.status.layers);

    let mut setup = engine.remembered();
    setup.scenes.push(Scene {
        name: "camera".into(),
        layers: vec![layer("second", Kind::Screen, "3", 88)],
        elements: vec![],
        order: vec![],
        shader: None,
    });
    setup.active_scene = "camera".into();
    let saved = crate::remembered::read(&crate::remembered::write(&setup).unwrap());
    let mut restored = Engine::with_sources(Box::new(ThisMachine));
    restored.restore(&saved);
    assert_eq!(restored.status.active_scene, "camera");
    assert_eq!(restored.status.layers[0].id, "second");
    assert_eq!(restored.remembered().scenes[0].layers[0].transform.x, 17);
}

#[test]
fn invalid_layer_assignment_and_scene_shader_roll_back_without_stopping_live() {
    let fake = Wrote {
        refuse: Some("shader:invalid".into()),
        ..Default::default()
    };
    let events = fake.scene_events.clone();
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    let old = layer("old", Kind::Camera, "cam", 0);
    engine.status.layers = vec![old.clone()];
    engine.status.on_air = true;
    let mut next = layer("new", Kind::Screen, "display", 0);
    next.shader = Some("bad.wgsl".into());
    engine.status.scenes.push(Scene {
        name: "next".into(),
        layers: vec![next],
        elements: vec![],
        order: vec![],
        shader: None,
    });
    assert!(matches!(
        engine.handle(Command::SceneSwitch {
            name: "next".into()
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status.layers, [old]);
    assert!(engine.status.on_air);
    assert_eq!(engine.status.active_scene, "default");
    assert_eq!(
        *events.lock().unwrap(),
        [
            "prepare Screen:display",
            "rollback display",
            "rollback prepared"
        ]
    );
    assert!(matches!(
        engine.handle(Command::LayerShader {
            id: "old".into(),
            path: Some("bad.wgsl".into())
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status.layers[0].shader, None);
    engine.status.scenes.push(Scene {
        name: "global".into(),
        layers: vec![],
        elements: vec![],
        order: vec![],
        shader: Some("bad.wgsl".into()),
    });
    assert!(matches!(
        engine.handle(Command::SceneSwitch {
            name: "global".into()
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status.active_scene, "default");
}

#[test]
fn runtime_failure_hides_only_the_affected_shader_in_status_and_persistence() {
    let fake = Wrote {
        refuse: Some("runtime:layer".into()),
        ..Default::default()
    };
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    let mut disabled = layer("disabled", Kind::Camera, "cam", 0);
    disabled.shader = Some("failed.wgsl".into());
    let mut healthy = layer("healthy", Kind::Screen, "1", 0);
    healthy.shader = Some("good.wgsl".into());
    engine.status.layers = vec![disabled, healthy];
    engine.status.shader = Some("scene.wgsl".into());
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status")
    };
    assert_eq!(status.layers[0].shader, None);
    assert_eq!(status.layers[1].shader.as_deref(), Some("good.wgsl"));
    assert_eq!(status.shader.as_deref(), Some("scene.wgsl"));
    assert_eq!(engine.remembered().layers[0].shader, None);
    assert_eq!(
        engine.remembered().layers[1].shader.as_deref(),
        Some("good.wgsl")
    );

    let fake = Wrote {
        refuse: Some("runtime:global".into()),
        ..Default::default()
    };
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    engine.status.shader = Some("failed.wgsl".into());
    engine.status.layers = vec![layer("healthy", Kind::Camera, "cam", 0)];
    engine.status.layers[0].shader = Some("good.wgsl".into());
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status")
    };
    assert_eq!(status.shader, None);
    assert_eq!(status.layers[0].shader.as_deref(), Some("good.wgsl"));
    assert_eq!(engine.remembered().scenes[0].shader, None);
}

#[test]
fn restore_skips_bad_shader_paths_without_losing_sources_or_other_scenes() {
    let mut engine =
        Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
    let mut saved_layer = layer("desktop", Kind::Screen, "1", 0);
    saved_layer.shader = Some("bad.wgsl".into());
    let setup = crate::remembered::Remembered {
        scenes: vec![
            Scene {
                name: "default".into(),
                layers: vec![saved_layer],
                elements: vec![],
                order: vec![],
                shader: Some("bad.wgsl".into()),
            },
            Scene {
                name: "later".into(),
                layers: vec![],
                elements: vec![],
                order: vec![],
                shader: Some("good.wgsl".into()),
            },
        ],
        ..Default::default()
    };
    engine.restore(&setup);
    assert_eq!(engine.status.layers.len(), 1);
    assert_eq!(engine.status.layers[0].shader, None);
    assert_eq!(engine.status.shader, None);
    assert_eq!(
        engine.remembered().scenes[1].shader.as_deref(),
        Some("good.wgsl")
    );
}

#[test]
fn clone_edit_delete_and_retain_inactive_layouts() {
    let mut engine = Engine::new();
    assert!(matches!(
        engine.handle(Command::SceneDuplicate {
            name: "second".into()
        }),
        Reply::Status(_)
    ));
    assert!(matches!(
        engine.handle(Command::SceneDelete {
            name: "second".into()
        }),
        Reply::Error { .. }
    ));
    assert!(matches!(
        engine.handle(Command::SceneSwitch {
            name: "default".into()
        }),
        Reply::Status(_)
    ));
    assert!(matches!(
        engine.handle(Command::SceneDelete {
            name: "second".into()
        }),
        Reply::Status(_)
    ));
    assert_eq!(engine.remembered().scenes.len(), 1);
}

#[test]
fn a_created_scene_is_empty_and_switched_to_and_a_duplicate_is_the_active_one() {
    let mut engine =
        Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
    engine.handle(Command::LayerCamera {
        id: "face".into(),
        device: "MacBook Pro Camera".into(),
    });
    engine.handle(Command::SceneElementAdd {
        element: Element {
            id: "title".into(),
            x: 10,
            y: 10,
            width: 300,
            height: 90,
            visible: true,
            shader: None,
            content: ElementContent::Text { text: "Hi".into() },
        },
    });
    let Reply::Status(copied) = engine.handle(Command::SceneDuplicate {
        name: "copy".into(),
    }) else {
        panic!("a duplicate answers with the status")
    };
    assert_eq!(copied.active_scene, "copy");
    assert_eq!(copied.layers.len(), 1, "the capture carries on");
    let copy = copied.scenes.iter().find(|s| s.name == "copy").unwrap();
    assert_eq!(copy.ordered_ids(), ["face", "title"]);

    let Reply::Status(empty) = engine.handle(Command::SceneCreate {
        name: "blank".into(),
    }) else {
        panic!("a create answers with the status")
    };
    assert_eq!(empty.active_scene, "blank");
    assert!(empty.layers.is_empty(), "nothing from the scene before");
    let blank = empty.scenes.iter().find(|s| s.name == "blank").unwrap();
    assert!(blank.elements.is_empty() && blank.shader.is_none());
    let copy = empty.scenes.iter().find(|s| s.name == "copy").unwrap();
    assert_eq!(copy.layers.len(), 1, "the scene left keeps its layers");

    for taken in ["blank", "copy", "default"] {
        assert!(matches!(
            engine.handle(Command::SceneCreate { name: taken.into() }),
            Reply::Error { .. }
        ));
        assert!(matches!(
            engine.handle(Command::SceneDuplicate { name: taken.into() }),
            Reply::Error { .. }
        ));
    }
    assert_eq!(engine.status().active_scene, "blank");
}

type Published = std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>;

/// A pipeline that writes down what it was told, so a test can ask whether
/// the capture was actually pointed somewhere rather than only whether the
/// status changed. The two disagreeing is the bug worth catching.
#[derive(Default)]
pub(super) struct Wrote {
    told: std::sync::Arc<std::sync::Mutex<Vec<Behind>>>,
    cameras: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    mics: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    played: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    levels: Faders,
    pub(super) shown: Shown,
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
    scene_events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Wrote {
    /// One that refuses what `reason` names, as `scene_transition` reads it.
    pub(super) fn refusing(reason: &str) -> Self {
        Self {
            refuse: Some(reason.into()),
            ..Default::default()
        }
    }
}

impl Picture for Wrote {
    fn scene_transition(
        &mut self,
        from: &[crate::layers::Layer],
        to: &[crate::layers::Layer],
        elements: &[Element],
        shader: Option<&str>,
    ) -> Result<(), String> {
        let plan = crate::scenes::transition(from, to);
        let mut events = self.scene_events.lock().unwrap();
        let mut prepared = Vec::new();
        for key in &plan.open {
            events.push(format!("prepare {:?}:{}", key.kind, key.handle));
            if let Some(reason) = &self.refuse {
                if (!reason.starts_with("scene:") && !reason.starts_with("shader:"))
                    || reason == &format!("scene:{}", key.handle)
                {
                    for source in prepared.iter().rev() {
                        events.push(format!("rollback {source}"));
                    }
                    events.push("rollback prepared".into());
                    return Err(reason.clone());
                }
            }
            prepared.push(key.handle.clone());
        }
        if let Some(reason) = &self.refuse {
            if reason.starts_with("shader:")
                && (shader == Some("bad.wgsl")
                    || to.iter().any(|l| l.shader.as_deref() == Some("bad.wgsl"))
                    || elements
                        .iter()
                        .any(|e| e.shader.as_deref() == Some("bad.wgsl")))
            {
                for source in prepared.iter().rev() {
                    events.push(format!("rollback {source}"));
                }
                events.push("rollback prepared".into());
                return Err(reason.clone());
            }
        }
        events.push("commit layout".into());
        for key in &plan.close {
            events.push(format!("close {:?}:{}", key.kind, key.handle));
        }
        Ok(())
    }
    fn show(
        &mut self,
        elements: &[Element],
        timers: &[(String, Duration)],
        _order: &[String],
    ) -> Result<(), String> {
        if let Some(why) = &self.refuse {
            if !why.starts_with("shader:") {
                return Err(why.clone());
            }
        }
        self.shown
            .lock()
            .expect("shown")
            .push((elements.to_vec(), timers.to_vec()));
        *self.counting.lock().expect("counting") = timers.first().map(|(_, left)| *left);
        Ok(())
    }
    fn mirror(&mut self, on: bool) {
        *self.mirrored.lock().expect("mirrored") = on;
    }
    fn layer_remove(&mut self, _id: &str) {
        self.told.lock().unwrap().push(Behind::Nothing);
    }
    fn layer_replace(
        &mut self,
        old: &crate::layers::Layer,
        new: &crate::layers::Layer,
    ) -> Result<(u32, u32), crate::engine::LayerSwapError> {
        if let Some(reason) = &self.refuse {
            return Err(crate::engine::LayerSwapError {
                reason: reason.clone(),
                restored: reason != "rollback lost",
            });
        }
        if old.source.kind == new.source.kind && old.source.handle == new.source.handle {
            return Ok((old.source.width, old.source.height));
        }
        self.layer_remove(&old.id);
        self.layer_add(new)
            .map_err(|reason| crate::engine::LayerSwapError {
                reason,
                restored: false,
            })
    }
    fn layer_add(&mut self, layer: &crate::layers::Layer) -> Result<(u32, u32), String> {
        self.refuse.clone().map_or_else(
            || {
                match layer.source.kind {
                    crate::layers::Kind::Screen => {
                        self.told
                            .lock()
                            .unwrap()
                            .push(Behind::Screen(crate::sources::DisplayId(
                                layer.source.handle.parse().unwrap(),
                            )))
                    }
                    crate::layers::Kind::Window => {
                        self.told
                            .lock()
                            .unwrap()
                            .push(Behind::Window(crate::sources::WindowId(
                                layer.source.handle.parse().unwrap(),
                            )))
                    }
                    crate::layers::Kind::Camera => self
                        .cameras
                        .lock()
                        .unwrap()
                        .push(Some(layer.source.handle.clone())),
                }
                Ok(match layer.source.kind {
                    crate::layers::Kind::Camera => (1280, 720),
                    crate::layers::Kind::Window => (853, 479),
                    crate::layers::Kind::Screen => (1920, 1080),
                })
            },
            Err,
        )
    }
    fn shader_active(&self) -> bool {
        self.refuse.as_deref() != Some("runtime:global")
    }
    fn layer_shader_active(&self, id: &str) -> bool {
        self.refuse.as_deref() != Some("runtime:layer") || id != "disabled"
    }
    fn element_shader(&mut self, _element: &Element, path: Option<&str>) -> Result<(), String> {
        if path == Some("bad.wgsl") {
            Err("invalid shader".into())
        } else {
            Ok(())
        }
    }
    fn layer_shader(
        &mut self,
        _layer: &crate::layers::Layer,
        path: Option<&str>,
    ) -> Result<(), String> {
        if path == Some("bad.wgsl") {
            Err("invalid shader".into())
        } else {
            Ok(())
        }
    }
    fn shader(&mut self, path: Option<&str>) -> Result<(), String> {
        if path == Some("bad.wgsl") {
            return Err("invalid shader".into());
        }
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        let _ = path;
        Ok(())
    }
    fn previewing(&mut self, on: bool) {
        *self.previewed.lock().expect("previewed") = Some(on);
    }
    fn layer_shot(&mut self, _id: &str) -> Option<(Vec<u8>, u32, u32)> {
        Some((vec![0xff, 0xd8, 0xff], 480, 270))
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
    fn layer_flowing(&self, id: &str) -> Flowing {
        if id.is_empty() {
            return Flowing::default();
        }
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
    fn app_audio(&mut self, app: Option<&str>) -> Result<Option<String>, String> {
        Ok(app.map(str::to_string))
    }
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
    fn levels(&mut self, levels: SoundLevels) -> Result<(), String> {
        *self.levels.lock().expect("levels") = Some((
            levels.mic,
            levels.music,
            levels.duck_db,
            levels.muted,
            levels.music_to_stream,
            levels.screen_sound,
        ));
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
            app_db: -60.0,
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
            app_samples: 0,
            app_complaint: None,
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
        scene_events: Default::default(),
    };
    (
        Engine::with_sources(Box::new(ThisMachine))
            .with_pipeline(Box::new(pipeline))
            .with_destination(Some(DESTINATION.into())),
        published,
    )
}

const DESTINATION: &str = "rtmp://hub/scene?user=remux&pass=not-a-real-key";

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
fn camera_position_changes_the_pipeline_mid_live_and_can_be_restored() {
    use crate::scene::CameraPosition;
    let pipeline = Wrote::default();
    let mut engine = Engine::with_sources(Box::new(ThisMachine))
        .with_pipeline(Box::new(pipeline))
        .with_destination(Some(DESTINATION.into()));
    engine.handle(Command::Window {
        query: "remux".into(),
    });
    engine.handle(Command::Camera {
        device: Some("HP".into()),
    });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    let at = CameraPosition { x: 300, y: 200 };
    let Reply::Status(after) = engine.handle(Command::CameraPosition { at: Some(at) }) else {
        panic!("status")
    };
    let face = after
        .layers
        .iter()
        .find(|l| l.source.kind == crate::layers::Kind::Camera)
        .unwrap();
    assert_eq!((face.transform.x, face.transform.y), (300, 200));
    assert!(after.on_air, "moving the camera must not stop the stream");
    assert_eq!(
        engine
            .remembered()
            .layers
            .iter()
            .find(|l| l.id == face.id)
            .unwrap()
            .transform,
        face.transform
    );
    for bad in [
        CameraPosition { x: 1920, y: 0 },
        CameraPosition { x: 0, y: 1080 },
    ] {
        assert!(matches!(
            engine.handle(Command::CameraPosition { at: Some(bad) }),
            Reply::Error { .. }
        ));
        let face = engine
            .status()
            .layers
            .iter()
            .find(|l| l.source.kind == crate::layers::Kind::Camera)
            .unwrap();
        assert_eq!(
            (face.transform.x, face.transform.y),
            (300, 200),
            "invalid wire command must not change the picture"
        );
    }
    engine.handle(Command::CameraPosition { at: None });
    let face = engine
        .status()
        .layers
        .iter()
        .find(|l| l.source.kind == crate::layers::Kind::Camera)
        .unwrap();
    assert_eq!((face.transform.x, face.transform.y), (0, 0));
    assert!(engine.status().on_air);
}

#[test]
fn changing_camera_shape_on_air_keeps_position_and_publication() {
    use crate::scene::{CameraPosition, CameraShape};
    let published: Published = Default::default();
    let pipeline = Wrote {
        published: published.clone(),
        ..Default::default()
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine))
        .with_pipeline(Box::new(pipeline))
        .with_destination(Some(DESTINATION.into()));
    engine.handle(Command::Window {
        query: "remux".into(),
    });
    engine.handle(Command::Camera {
        device: Some("HP".into()),
    });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    let at = CameraPosition { x: 300, y: 200 };
    engine.handle(Command::CameraPosition { at: Some(at) });
    let Reply::Status(rectangle) = engine.handle(Command::CameraShape {
        shape: CameraShape::Rectangle,
    }) else {
        panic!("status")
    };
    let face = rectangle
        .layers
        .iter()
        .find(|l| l.source.kind == crate::layers::Kind::Camera)
        .unwrap();
    assert_eq!(face.shape, Some(CameraShape::Rectangle));
    assert_eq!((face.transform.x, face.transform.y), (300, 200));
    assert!(rectangle.on_air);
    assert_eq!(
        engine
            .remembered()
            .layers
            .iter()
            .find(|l| l.id == face.id)
            .unwrap()
            .shape,
        Some(CameraShape::Rectangle)
    );
    let Reply::Status(circle) = engine.handle(Command::CameraShape {
        shape: CameraShape::Circle,
    }) else {
        panic!("status")
    };
    let face = circle
        .layers
        .iter()
        .find(|l| l.source.kind == crate::layers::Kind::Camera)
        .unwrap();
    assert_eq!(face.shape, Some(CameraShape::Circle));
    assert_eq!((face.transform.x, face.transform.y), (300, 200));
    assert!(circle.on_air);
    assert_eq!(
        published.lock().unwrap().len(),
        1,
        "shape changes do not restart publication"
    );
}

#[test]
fn a_shader_changes_the_scene_without_restarting_live_and_refusal_keeps_the_last_one() {
    let (mut engine, published) = publishing_engine(None);
    engine.handle(Command::Window {
        query: "remux".into(),
    });
    assert_eq!(engine.handle(Command::GoLive), Reply::Ok);
    let path = "/tmp/invert.wgsl".to_string();
    let Reply::Status(status) = engine.handle(Command::Shader {
        path: Some(path.clone()),
    }) else {
        panic!("shader selection answers with a status");
    };
    assert_eq!(status.shader.as_deref(), Some(path.as_str()));
    assert!(status.on_air);
    assert_eq!(published.lock().unwrap().len(), 1);
    let Reply::Status(status) = engine.handle(Command::Shader { path: None }) else {
        panic!("shader off answers with a status");
    };
    assert_eq!(status.shader, None);
    assert!(status.on_air);
    assert_eq!(published.lock().unwrap().len(), 1);
    engine.handle(Command::Shader {
        path: Some("/tmp/unsafe.wgsl".into()),
    });
    engine.handle(Command::HideEverything);
    assert_eq!(
        engine.status().shader,
        None,
        "panic removes even a shader that obscures the emergency card"
    );

    let (mut refusing, _) = publishing_engine(Some("compiler refused".into()));
    let Reply::Error { message } = refusing.handle(Command::Shader { path: Some(path) }) else {
        panic!("a broken shader must not be accepted");
    };
    assert_eq!(message, "compiler refused");
    assert_eq!(refusing.status().shader, None);
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
        scene_events: Default::default(),
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
fn a_panel_can_read_back_scene_elements() {
    let (mut engine, _) = publishing_engine(None);
    let element = Element {
        id: "label".into(),
        x: 20,
        y: 30,
        width: 300,
        height: 90,
        visible: true,
        shader: None,
        content: ElementContent::Text {
            text: "Chegando já".into(),
        },
    };
    engine.handle(Command::SceneElementAdd {
        element: element.clone(),
    });
    assert_eq!(engine.status().scenes[0].elements, [element]);
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
        scene_events: Default::default(),
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
        scene_events: Default::default(),
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
        told.lock()
            .expect("told")
            .iter()
            .copied()
            .filter(|source| *source != Behind::Nothing)
            .collect::<Vec<_>>(),
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
    assert!(status.layers.is_empty());
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
        scene_events: Default::default(),
    };
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(refusing));
    let Reply::Error { message } = engine.handle(Command::Screen { display: 3 }) else {
        panic!("a refused capture is an error")
    };
    assert!(message.contains("refused"), "{message}");
    let Reply::Error { message } = engine.handle(Command::LayerWindow {
        id: "editor".into(),
        query: "tmux".into(),
    }) else {
        panic!("a refused overlay is an error")
    };
    assert!(message.contains("refused"));
    assert!(
        engine.status().layers.is_empty(),
        "a failed capture is not in Status"
    );
    assert!(
        engine.status().layers.is_empty(),
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
type Shown = std::sync::Arc<std::sync::Mutex<Vec<(Vec<Element>, Vec<(String, Duration)>)>>>;

/// Where the faders were last put: microphone, music, duck, muted.
type Faders = std::sync::Arc<std::sync::Mutex<Option<(f64, f64, f64, bool, bool, bool)>>>;

#[test]
fn hiding_keeps_capture_preview_layout_and_id_and_pauses_screen_sound() {
    let pipeline = Wrote::default();
    let captured = pipeline.told.clone();
    let levels = pipeline.levels.clone();
    let shown = pipeline.shown.clone();
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
    engine.handle(Command::LayerScreen {
        id: "desk".into(),
        display: 1,
    });
    let original = engine.status().layers[0].clone();
    engine.handle(Command::LayerScreenSound {
        id: "desk".into(),
        on: true,
    });
    assert!(levels.lock().unwrap().unwrap().5);

    let Reply::Status(hidden) = engine.handle(Command::LayerVisible {
        id: "desk".into(),
        on: false,
    }) else {
        panic!("hide")
    };
    assert_eq!(hidden.layers[0].id, original.id);
    assert_eq!(hidden.layers[0].source, original.source);
    assert_eq!(hidden.layers[0].transform, original.transform);
    assert!(!hidden.layers[0].visible);
    assert_eq!(
        hidden.layer_flowing["desk"].captured, 3,
        "capture continues while hidden"
    );
    assert!(matches!(
        engine.handle(Command::LayerShot { id: "desk".into() }),
        Reply::Shot { .. }
    ));
    assert!(hidden.screen_sound && hidden.screen_sound_layer.as_deref() == Some("desk"));
    let said = crate::cli::render(&Reply::Status(hidden.clone()));
    assert!(
        said.contains("hidden") && said.contains("screen sound paused"),
        "{said}"
    );
    assert!(
        !levels.lock().unwrap().unwrap().5,
        "hidden display audio must leave the mix"
    );
    assert_eq!(engine.status().active_scene, "default");
    assert_eq!(
        captured.lock().unwrap().len(),
        1,
        "hide must not stop or reopen capture"
    );
    let saved = crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
    assert!(!saved.layers[0].visible);
    let mut restored =
        Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
    restored.restore(&saved);
    assert_eq!(restored.status().layers[0].id, "desk");
    assert!(!restored.status().layers[0].visible);

    engine.handle(Command::LayerVisible {
        id: "desk".into(),
        on: true,
    });
    assert!(engine.status().layers[0].visible);
    assert!(
        levels.lock().unwrap().unwrap().5,
        "show resumes requested audio"
    );
    assert_eq!(engine.status().active_scene, "default");
    assert_eq!(captured.lock().unwrap().len(), 1);
    let before = elements_shown(&shown).len();
    engine.handle(Command::LayerVisible {
        id: "desk".into(),
        on: true,
    });
    assert_eq!(
        elements_shown(&shown).len(),
        before,
        "showing an already visible layer does nothing"
    );
}

#[test]
fn ordinary_visual_verbs_reuse_one_layer_without_a_reserved_id() {
    let pipeline = Wrote::default();
    let told = pipeline.told.clone();
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
    let Reply::Status(first) = engine.handle(Command::Window {
        query: "tmux".into(),
    }) else {
        panic!("window")
    };
    let id = first.layers[0].id.clone();
    assert!(id.starts_with("source-") && !id.contains("legacy"));
    engine.handle(Command::LayerCamera {
        id: "face".into(),
        device: "HP".into(),
    });
    let transform = crate::layers::Transform {
        x: 80,
        y: 50,
        width: 600,
        height: 300,
        degrees: 15,
    };
    engine.handle(Command::LayerTransform {
        id: id.clone(),
        transform,
    });
    let crop = crate::layers::Crop {
        x: 20,
        y: 30,
        width: 400,
        height: 200,
    };
    engine.handle(Command::LayerCrop {
        id: id.clone(),
        crop: Some(crop),
    });
    let Reply::Status(switched) = engine.handle(Command::Screen { display: 3 }) else {
        panic!("screen")
    };
    assert_eq!(
        switched
            .layers
            .iter()
            .map(|layer| layer.id.as_str())
            .collect::<Vec<_>>(),
        vec![id.as_str(), "face"]
    );
    assert_eq!(switched.layers[0].transform, transform);
    assert_eq!(switched.layers[0].crop, Some(crop));
    assert_eq!(switched.layers[0].source.name, "VG2791R");
    assert_eq!(switched.layers[0].source.kind, crate::layers::Kind::Screen);
    // The fake records the stop before the new source opens, not two captures
    // with different generated IDs overlapping even briefly.
    assert_eq!(
        *told.lock().unwrap(),
        vec![
            Behind::Window(WindowId(10)),
            Behind::Nothing,
            Behind::Screen(DisplayId(3))
        ]
    );
    let Reply::Status(same) = engine.handle(Command::Screen { display: 3 }) else {
        panic!("same source")
    };
    assert_eq!(same.layers[0].id, id);
    assert_eq!(
        told.lock().unwrap().len(),
        3,
        "same source must not restart capture"
    );
}

#[test]
fn a_swap_resets_an_invalid_crop_and_only_disables_audio_for_a_window() {
    let mut engine =
        Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
    engine.handle(Command::LayerScreen {
        id: "desk".into(),
        display: 1,
    });
    engine.handle(Command::LayerCrop {
        id: "desk".into(),
        crop: Some(crate::layers::Crop {
            x: 1500,
            y: 0,
            width: 300,
            height: 200,
        }),
    });
    engine.handle(Command::LayerScreenSound {
        id: "desk".into(),
        on: true,
    });
    let Reply::Status(display) = engine.handle(Command::LayerReplaceScreen {
        id: "desk".into(),
        display: 3,
    }) else {
        panic!("display swap")
    };
    assert!(display.screen_sound);
    assert_eq!(display.screen_sound_layer.as_deref(), Some("desk"));
    let Reply::Status(window) = engine.handle(Command::Window {
        query: "notes".into(),
    }) else {
        panic!("window swap")
    };
    assert_eq!(window.layers[0].id, "desk");
    assert_eq!(window.layers[0].source.kind, crate::layers::Kind::Window);
    assert_eq!(window.layers[0].crop, None);
    assert!(!window.screen_sound);
    assert_eq!(window.screen_sound_layer, None);
    assert!(matches!(
        engine.handle(Command::LayerReplaceCamera {
            id: "desk".into(),
            device: "HP".into()
        }),
        Reply::Error { .. }
    ));
}

#[test]
fn failed_swap_keeps_the_id_or_explicitly_drops_an_unrecoverable_capture() {
    let mut engine = Engine::with_sources(Box::new(ThisMachine));
    engine.handle(Command::Screen { display: 1 });
    let original = engine.status().layers[0].clone();
    let mut engine = engine.with_pipeline(Box::new(Wrote {
        refuse: Some("new source refused".into()),
        ..Default::default()
    }));
    assert!(matches!(
        engine.handle(Command::Window {
            query: "notes".into()
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status().layers, vec![original.clone()]);
    // A missing target is validated before the pipeline sees any swap.
    assert!(matches!(
        engine.handle(Command::Window {
            query: "missing-window".into()
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status().layers, vec![original.clone()]);

    let mut engine = engine.with_pipeline(Box::new(Wrote {
        refuse: Some("rollback lost".into()),
        ..Default::default()
    }));
    assert!(matches!(
        engine.handle(Command::Window {
            query: "notes".into()
        }),
        Reply::Error { .. }
    ));
    assert!(
        engine.status().layers.is_empty(),
        "a lost capture must not remain in Status"
    );
}

#[test]
fn a_camera_alias_reuses_the_unique_camera_but_refuses_two() {
    let mut engine =
        Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
    engine.handle(Command::LayerCamera {
        id: "host".into(),
        device: "HP".into(),
    });
    let Reply::Status(switched) = engine.handle(Command::Camera {
        device: Some("MacBook".into()),
    }) else {
        panic!("camera swap")
    };
    assert_eq!(switched.layers[0].id, "host");
    assert_eq!(switched.layers[0].source.name, "MacBook Pro Camera");
    engine.handle(Command::LayerCamera {
        id: "guest".into(),
        device: "HP".into(),
    });
    assert!(
        matches!(engine.handle(Command::Camera { device: Some("HP".into()) }), Reply::Error { message } if message.contains("layer ID"))
    );
}

#[test]
fn old_source_verbs_refuse_ambiguous_layers() {
    let mut engine =
        Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(Wrote::default()));
    for id in ["left", "right"] {
        assert!(matches!(
            engine.handle(Command::LayerScreen {
                id: id.into(),
                display: 1
            }),
            Reply::Status(_)
        ));
    }
    assert!(
        matches!(engine.handle(Command::Screen { display: 3 }), Reply::Error { message } if message.contains("layer ID"))
    );
    assert!(
        matches!(engine.handle(Command::Window { query: "missing".into() }), Reply::Error { message } if message.contains("layer ID"))
    );
    assert!(
        matches!(engine.handle(Command::Share { on: false }), Reply::Error { message } if message.contains("layer ID"))
    );
    assert_eq!(engine.status().layers.len(), 2);
    assert!(
        matches!(engine.handle(Command::Shot { of: Framed::Screen }), Reply::Error { message } if message.contains("layer-shot"))
    );
    assert!(matches!(
        engine.handle(Command::LayerShot { id: "right".into() }),
        Reply::Shot { .. }
    ));
    assert!(matches!(
        engine.handle(Command::LayerShot {
            id: "missing".into()
        }),
        Reply::Error { .. }
    ));
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status")
    };
    assert_eq!(
        status
            .layer_flowing
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["left", "right"]
    );
    for id in ["first", "second"] {
        engine.handle(Command::LayerCamera {
            id: id.into(),
            device: "HP".into(),
        });
    }
    assert!(
        matches!(engine.handle(Command::Camera { device: None }), Reply::Error { message } if message.contains("layer ID"))
    );
}

#[test]
fn screen_sound_requires_one_display_and_switches_without_summing() {
    let pipeline = Wrote::default();
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
    let error = engine.handle(Command::ScreenSound { on: true });
    assert!(matches!(error, Reply::Error { .. }));
    for id in ["left", "right"] {
        engine.handle(Command::LayerScreen {
            id: id.into(),
            display: 1,
        });
    }
    let error = engine.handle(Command::ScreenSound { on: true });
    assert!(matches!(error, Reply::Error { .. }));
    engine.handle(Command::LayerScreenSound {
        id: "left".into(),
        on: true,
    });
    assert_eq!(engine.status().screen_sound_layer.as_deref(), Some("left"));
    engine.handle(Command::LayerScreenSound {
        id: "right".into(),
        on: true,
    });
    assert_eq!(engine.status().screen_sound_layer.as_deref(), Some("right"));
    engine.handle(Command::LayerRemove { id: "right".into() });
    assert!(!engine.status().screen_sound);
    assert_eq!(engine.status().screen_sound_layer, None);
}

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
    engine.handle(Command::LayerScreen {
        id: "display".into(),
        display: 1,
    });

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

#[test]
fn application_audio_is_independent_and_panic_closes_it() {
    let pipeline = Wrote::default();
    let mut engine = Engine::with_sources(Box::new(ThisMachine)).with_pipeline(Box::new(pipeline));
    assert_eq!(engine.status().app_audio, None);
    let Reply::Status(selected) = engine.handle(Command::AppAudio {
        app: Some("Safari".into()),
    }) else {
        panic!("application audio must answer with status");
    };
    assert_eq!(selected.app_audio.as_deref(), Some("Safari"));
    assert!(!selected.screen_sound);
    engine.handle(Command::LayerScreen {
        id: "display".into(),
        display: 1,
    });
    engine.handle(Command::ScreenSound { on: true });
    engine.handle(Command::AppAudioVolume { level: 0.3 });
    assert_eq!(engine.status().app_audio_volume, 0.3);
    assert!(engine.status().screen_sound);
    engine.handle(Command::AppAudio { app: None });
    assert_eq!(engine.status().app_audio, None);
    assert!(
        engine.status().screen_sound,
        "turning off the app leaves screen sound on"
    );
    engine.handle(Command::AppAudio {
        app: Some("Safari".into()),
    });
    engine.handle(Command::HideEverything);
    assert_eq!(engine.status().app_audio, None);
    assert!(!engine.status().screen_sound);
}

/// Each `show` the pipeline was given: the elements and the running timers.
type Showings = Vec<(Vec<Element>, Vec<(String, Duration)>)>;

fn elements_shown(log: &Shown) -> Showings {
    log.lock().expect("shown").clone()
}

/// What the pipeline was last told a running timer had left.
type Counting = std::sync::Arc<std::sync::Mutex<Option<Duration>>>;

fn machine_watching_elements() -> (Engine, Shown, Counting) {
    let shown: Shown = Default::default();
    let counting: Counting = Default::default();
    let pipeline = Wrote {
        told: Default::default(),
        cameras: Default::default(),
        mics: Default::default(),
        played: Default::default(),
        levels: Default::default(),
        shown: shown.clone(),
        scene_events: Default::default(),
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

#[test]
fn panic_clears_scene_and_stops_media() {
    let (mut engine, shown, _) = machine_watching_elements();
    engine.handle(Command::Screen { display: 3 });
    engine.handle(Command::HideEverything);
    assert_eq!(engine.status().active_scene, "default");
    assert!(elements_shown(&shown).last().is_some());
    assert!(engine.status().layers.is_empty());
    assert!(engine.status().muted);
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
        scene_events: Default::default(),
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
    assert_eq!(now.active_scene, "default");
    assert!(now.layers.is_empty(), "the sources are off");
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
