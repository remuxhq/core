//! The picture: what is behind it, what is drawn over it, and the self-view.

use super::*;
use base64::Engine as _;

/// The picture's half of the media path: what is captured and what is drawn
/// over it. See [`crate::engine::Pipeline`] for why it is a port.
pub trait Picture: Send {
    /// Point the capture at something, or at nothing. Called on every change,
    /// including while on air, because swapping the monitor mid-live is a
    /// thing people do.
    fn capture(&mut self, behind: Behind) -> Result<(), String>;
    /// Open a camera by the id the device list gave, or close the one that is
    /// open. A second capture, not a second technology: what comes out is the
    /// same kind of frame on the same clock, so the compositor has one path.
    fn camera(&mut self, device: Option<&str>) -> Result<(), String>;
    /// Put a card up, or take it down. The line is already resolved: which
    /// words a card shows is [`crate::card::Words`]'s decision, not the
    /// pipeline's.
    /// The countdown is a duration and not a deadline: the engine has no
    /// clock, and counting is something that happens between frames.
    fn show(&mut self, card: Card, line: &str, counting: Option<Duration>) -> Result<(), String>;
    fn stop(&mut self);
    /// One frame of what is going out, already small and already JPEG.
    ///
    /// `None` when there is no picture yet, which a panel draws as an empty
    /// frame rather than as an error: an engine that has not been pointed at
    /// anything is not broken.
    fn shot(&mut self, of: Framed) -> Option<(Vec<u8>, u32, u32)>;
    /// Flip the self-view left to right, or stop. A person watching themselves
    /// expects a mirror; a viewer expects the writing on their shirt to read
    /// correctly, and the engine cannot have both, so it is a switch.
    fn mirror(&mut self, on: bool);
    /// Where and how the camera sits. The whole layout, every time.
    fn layout(&mut self, _layout: crate::scene::Layout) {}
    fn flowing(&self) -> Flowing;
    fn camera_flowing(&self) -> Flowing;
    /// Where a panel can map the preview, when there is one. Read every time
    /// rather than kept: an engine that lost its region should stop claiming
    /// to have one.
    fn preview(&self) -> Option<crate::preview::Preview> {
        None
    }
    /// Whether anybody is drawing the preview, so the pipeline can stop making
    /// one for nobody. See [`Command::Watching`].
    fn previewing(&mut self, _on: bool) {}
}

impl Engine {
    /// A lease a face renews while it draws, once a second. "Off" is
    /// accepted and does nothing: a face that stopped drawing stops
    /// renewing, and the lease runs out by itself, so two windows and
    /// one closing never blinks the other. It used to be a count, and
    /// a count is state this engine holds for a face: an engine
    /// restarted under a live panel started at zero and never
    /// published again, which read as the picture freezing.
    pub(super) fn watch(&mut self, on: bool) -> Reply {
        if on {
            self.watch_lease = WATCH_LEASE_TICKS;
            self.pipeline.previewing(true);
        }
        Reply::Ok
    }

    /// Choosing what is in the picture. Each answers with the name of
    /// what it chose rather than a bare ok, because "screen 3" and
    /// "the one called VG2791R" are different amounts of confidence
    /// and a person about to go live wants the second.
    pub(super) fn choose_screen(&mut self, display: u32) -> Reply {
        match self.sources.available() {
            Ok(available) => match available.screens.iter().find(|s| s.id.0 == display) {
                Some(screen) => {
                    // Kept as an id and never as a place in the list: the
                    // capturer and the display list order the same hardware
                    // differently, so an index comes back pointing at the
                    // other monitor.
                    self.chosen_display = Some(display);
                    self.chosen_window = None;
                    self.behind(Behind::Screen(screen.id), screen.name.clone())
                }
                None => Reply::Error {
                    message: format!(
                        "no display {display}. There is {}",
                        list_of(available.screens.iter().map(|s| s.name.clone()))
                    ),
                },
            },
            Err(why) => Reply::Error { message: why },
        }
    }

    pub(super) fn choose_window(&mut self, query: String) -> Reply {
        match self.sources.available() {
            Ok(available) => match pick(&query, &available.windows) {
                Some(window) => {
                    self.chosen_window = Some(query.clone());
                    self.behind(Behind::Window(window.id), window_label(window))
                }
                None => Reply::Error {
                    message: format!("no window matches {query:?}"),
                },
            },
            Err(why) => Reply::Error { message: why },
        }
    }

    /// The camera and the microphone. `None` turns one off, which is a
    /// different thing from never having chosen one: the camera is a
    /// slot in the picture and the mic is a channel in the mix, and
    /// both are allowed to be empty.
    pub(super) fn choose_camera(&mut self, device: Option<String>) -> Reply {
        match device {
            None => match self.pipeline.camera(None) {
                Ok(()) => {
                    self.status.camera = None;
                    Reply::Status(Box::new(self.reported()))
                }
                Err(why) => Reply::Error { message: why },
            },
            Some(query) => self.open_camera(query),
        }
    }

    /// The switch that takes the screen off the live without stopping
    /// anything. Black with "No content shared" on it, never a frozen
    /// frame: the picture has to keep flowing or a viewer cannot tell a
    /// deliberate blank from a broken stream.
    pub(super) fn share(&mut self, on: bool) -> Reply {
        if on {
            Reply::Error {
                message: "choose a screen or a window to share".into(),
            }
        } else {
            self.behind(Behind::Nothing, String::new())
        }
    }

    /// The countdown always belongs to the starting card, and starting
    /// it puts that card up. Nobody presses "start the countdown"
    /// wanting to stay on the picture they are already showing.
    pub(super) fn countdown(&mut self, seconds: Option<u32>) -> Reply {
        let length = seconds.unwrap_or(DEFAULT_COUNTDOWN_SECONDS);
        self.show_counting(Card::StartingSoon, Some(Duration::from_secs(length as u64)))
    }

    pub(super) fn card_text(&mut self, which: Card, text: String) -> Reply {
        match which {
            Card::StartingSoon => self.status.words.starting = text,
            Card::BackInAMoment => self.status.words.back = text,
            // The engine's own line about itself, and the live picture
            // has no line at all.
            Card::NothingShared | Card::Live => {
                return Reply::Error {
                    message: "that card's words are not yours to change".into(),
                }
            }
        }
        // Re-show, so a card already up changes at once rather than at
        // the next time somebody happens to press it.
        match self.status.card {
            Some(up) if up == which => self.show(up),
            _ => Reply::Status(Box::new(self.reported())),
        }
    }

    /// A picture of what is going out, for a panel to draw. It asks;
    /// the engine does not push, and at one a second that is the same
    /// thing with less to go wrong.
    pub(super) fn shot(&mut self, of: Framed) -> Reply {
        match self.pipeline.shot(of) {
            Some((jpeg, width, height)) => Reply::Shot {
                jpeg: base64::engine::general_purpose::STANDARD.encode(&jpeg),
                width,
                height,
            },
            None => Reply::Error {
                message: match of {
                    Framed::Camera => "no camera is open".into(),
                    Framed::Screen => "no screen is being shared".into(),
                    Framed::Scene => "there is no picture yet".into(),
                },
            },
        }
    }

    pub(super) fn mirror(&mut self, on: bool) -> Reply {
        self.status.mirrored = on;
        self.pipeline.mirror(on);
        Reply::Status(Box::new(self.reported()))
    }

    /// The setup of now, under a name. What is behind the picture is kept
    /// as the words that chose it, so it can be found again after a restart.
    pub(super) fn scene_save(&mut self, name: String) -> Reply {
        use crate::scenes::{keep, Scene, Shown};
        if name.is_empty() {
            return Reply::Error {
                message: "a scene needs a name".into(),
            };
        }
        let shown = match (
            self.status.screen.is_some(),
            &self.chosen_window,
            self.chosen_display,
        ) {
            (true, Some(query), _) => Shown::Window {
                query: query.clone(),
            },
            (true, None, Some(display)) => Shown::Screen { display },
            _ => Shown::Nothing,
        };
        keep(
            &mut self.scenes,
            Scene {
                name: name.clone(),
                shown,
                layout: self.status.layout,
                mirrored: self.status.mirrored,
                hear: self.status.hearing_apps.clone(),
            },
        );
        self.status.scenes = self.scenes.iter().map(|s| s.name.clone()).collect();
        self.status.scene = Some(name);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn scene_switch(&mut self, name: &str) -> Reply {
        use crate::scenes::{find, Shown};
        let Some(scene) = find(&self.scenes, name).cloned() else {
            return Reply::Error {
                message: format!(
                    "no scene called {name}; there is {}",
                    list_of(self.scenes.iter().map(|s| s.name.clone()))
                ),
            };
        };
        let behind = match &scene.shown {
            Shown::Nothing => self.share(false),
            Shown::Screen { display } => self.choose_screen(*display),
            Shown::Window { query } => self.choose_window(query.clone()),
        };
        if let Reply::Error { message } = behind {
            return Reply::Error {
                message: format!("scene {name}: {message}"),
            };
        }
        self.status.layout = scene.layout;
        self.pipeline.layout(scene.layout);
        self.status.mirrored = scene.mirrored;
        self.pipeline.mirror(scene.mirrored);
        if scene.hear != self.status.hearing_apps {
            if let Reply::Error { message } = self.hear(scene.hear.clone()) {
                return Reply::Error {
                    message: format!("scene {name}: {message}"),
                };
            }
        }
        self.status.scene = Some(name.to_string());
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn scene_forget(&mut self, name: &str) -> Reply {
        if !crate::scenes::forget(&mut self.scenes, name) {
            return Reply::Error {
                message: format!("no scene called {name}"),
            };
        }
        self.status.scenes = self.scenes.iter().map(|s| s.name.clone()).collect();
        if self.status.scene.as_deref() == Some(name) {
            self.status.scene = None;
        }
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn layout(&mut self, patch: crate::scene::LayoutPatch) -> Reply {
        self.status.layout = self.status.layout.patched(patch);
        self.pipeline.layout(self.status.layout);
        Reply::Status(Box::new(self.reported()))
    }
    /// Put a card up, or take it down, and let the picture know.
    /// The card the engine puts up on its own behalf never appears in the
    /// status: the status says what the *operator* chose, and a client drawing
    /// "Back in a moment" as selected because the screen happens to be off
    /// would be lying to them.
    pub(super) fn show(&mut self, card: Card) -> Reply {
        self.show_counting(card, None)
    }

    /// Put a card up, optionally with a clock running under its words.
    pub(super) fn show_counting(&mut self, card: Card, counting: Option<Duration>) -> Reply {
        let line = self.status.words.line(card).to_string();
        if let Err(why) = self.pipeline.show(card, &line, counting) {
            return Reply::Error { message: why };
        }
        self.status.card = (card != Card::Live).then_some(card);
        self.blank_up = false;

        // An operator's card covers the engine's. Taking theirs down while
        // nothing is behind the picture has to put the engine's back, or the
        // stream goes empty at the very moment they thought they were going
        // live, and the only sign is a viewer saying "it froze".
        if card == Card::Live && self.status.screen.is_none() {
            let line = self.status.words.line(Card::NothingShared).to_string();
            if let Err(why) = self.pipeline.show(Card::NothingShared, &line, None) {
                return Reply::Error { message: why };
            }
            self.blank_up = true;
        }
        Reply::Status(Box::new(self.reported()))
    }

    /// Put something behind the picture and say so, or report why not.
    ///
    /// One door for every source, because the capture has to be told on every
    /// change including while on air: swapping the monitor mid-live is a thing
    /// people do, and it is the moment a pipeline is most likely to break.
    pub(super) fn behind(&mut self, behind: Behind, name: String) -> Reply {
        if let Err(why) = self.pipeline.capture(behind) {
            // The capture refused, so the status must not claim it happened.
            return Reply::Error { message: why };
        }
        self.status.screen = (!name.is_empty()).then_some(name);

        // Nothing behind the picture is a picture of its own, never an empty
        // one. A viewer looking at a frozen frame cannot tell a deliberate
        // blank from a stream that died, so the engine says which it is. An
        // operator's card already up outranks it: they asked for that.
        if behind == Behind::Nothing && self.status.card.is_none() {
            let line = self.status.words.line(Card::NothingShared).to_string();
            if let Err(why) = self.pipeline.show(Card::NothingShared, &line, None) {
                return Reply::Error { message: why };
            }
            self.blank_up = true;
        } else if self.blank_up {
            // Something is behind the picture again, so the engine's own card
            // comes down. Only its own: an operator's card is theirs to take
            // down.
            if let Err(why) = self.pipeline.show(Card::Live, "", None) {
                return Reply::Error { message: why };
            }
            self.blank_up = false;
        }
        Reply::Status(Box::new(self.reported()))
    }

    /// Find the camera somebody meant and actually open it.
    ///
    /// Not the shared `choose`, because a camera is the one device that can be
    /// found and still refuse: it may be in use by another application, or the
    /// grant may be missing. The status must not name a camera that never
    /// opened, so the open happens first and the name second.
    pub(super) fn open_camera(&mut self, query: String) -> Reply {
        let available = match self.sources.available() {
            Ok(available) => available,
            Err(why) => return Reply::Error { message: why },
        };
        let Some(camera) = pick_device(&query, &available.cameras) else {
            return Reply::Error {
                message: format!(
                    "no camera matches {query:?}. There is {}",
                    list_of(available.cameras.iter().map(|c| c.name.clone()))
                ),
            };
        };
        match self.pipeline.camera(Some(&camera.id)) {
            Ok(()) => {
                self.status.camera = Some(camera.name.clone());
                Reply::Status(Box::new(self.reported()))
            }
            Err(why) => Reply::Error { message: why },
        }
    }
}
