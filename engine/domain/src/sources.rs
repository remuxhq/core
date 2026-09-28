//! What can be captured, and which one was asked for.
//!
//! No Apple framework reaches this file: it is arithmetic over ids and names,
//! so `cargo test` covers all of it and the capture adapter stays thin.

use crate::protocol::Named;

/// A display, as the window server numbers it. A screen is only ever
/// identified by this, never by its position in a list: the capturer and the
/// display list order the same hardware differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub id: DisplayId,
    pub name: String,
    /// What the platform calls this monitor for good (its UUID), where it
    /// has such a thing: a number is a position in a list, and a monitor
    /// plugged in or pulled out renumbers the rest.
    pub stable: Option<String>,
}

/// A window, as the window server numbers it. Same discipline as DisplayId:
/// the id is the handle, the title is for people and changes under you.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub title: String,
    pub app: String,
}

/// The window a person meant by typing part of a name.
///
/// `remux video window <part of a title>` promises to swap the capture with no
/// dialog, so this has to answer the same way every time. Two rules, in
/// order: an exact title wins, because someone who typed the whole thing meant
/// that one; otherwise the first match in the capturer's own order, which is
/// stable while the windows are. The app name matches too, so "Brave" finds a
/// window whose title is the page it is showing.
pub fn pick<'a>(query: &str, windows: &'a [Window]) -> Option<&'a Window> {
    let query = query.to_lowercase();
    let exact = windows.iter().find(|w| w.title.to_lowercase() == query);
    exact.or_else(|| {
        windows.iter().find(|w| {
            w.title.to_lowercase().contains(&query) || w.app.to_lowercase().contains(&query)
        })
    })
}

/// A window exactly as the system described it, before anything decided
/// whether it is worth offering.
///
/// Every field is optional because every one of them is optional at the
/// boundary, and that is the whole point of this type. The objc2 bindings
/// correct their nullability from release to release: the `frameworks 0.4`
/// milestone is a list of exactly that (`MTLBuffer contents pointer is
/// nullable`, `NSSavePanel savePanel can return NULL`). When a field that used
/// to be a `String` becomes an `Option<String>`, the change has to land in a
/// tested branch here rather than as an `unwrap_or_default()` in the adapter,
/// because an `unwrap_or_default()` compiles, ships, and quietly turns every
/// window into one with no name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawWindow {
    pub id: u32,
    pub title: Option<String>,
    pub app: Option<String>,
    /// Whether the window server says this window is on a display right now.
    /// Behind another window still counts; minimised or on another Space does
    /// not.
    pub on_screen: bool,
}

/// The window a person could mean, or nothing.
///
/// A window with no title is a shadow, a menu, a status item or a drag image.
/// The desktop is full of them and none is what anybody means by "share that
/// window", so they never reach a list.
pub fn window_from(raw: RawWindow) -> Option<Window> {
    // A window that is not on a display cannot be captured: ScreenCaptureKit
    // accepts the filter and then delivers nothing at all, forever, with no
    // error: a hidden window of an application, literally titled "Window",
    // is handed over and then no frame ever comes.
    if !raw.on_screen {
        return None;
    }
    let title = raw.title.filter(|title| !title.trim().is_empty())?;
    Some(Window {
        id: WindowId(raw.id),
        title,
        // An application that will not name itself is still a window you can
        // share. Losing the window over a missing label would be the worse
        // trade of the two.
        app: raw.app.unwrap_or_default(),
    })
}

/// The camera or microphone somebody meant.
///
/// The id wins outright, because an id is unambiguous and is what a saved
/// preference holds; anything else is matched as part of the name, the way
/// `remux camera HyperX` works. The id is never the position in the list: a
/// device unplugged and plugged back in comes back somewhere else.
pub fn pick_device<'a>(query: &str, devices: &'a [Named]) -> Option<&'a Named> {
    let wanted = query.to_lowercase();
    devices
        .iter()
        .find(|device| device.id == query)
        .or_else(|| {
            devices
                .iter()
                .find(|device| device.name.to_lowercase() == wanted)
        })
        .or_else(|| {
            devices
                .iter()
                .find(|device| device.name.to_lowercase().contains(&wanted))
        })
}

/// Join what the capturer can capture with what the monitors are called.
/// The capturer's order is kept, because that is the order a person sees; the
/// name is taken from the display list, because "Screen 1" tells nobody which
/// monitor that is. A display the list does not know keeps its dull label.
pub fn screens(capturer: &[(DisplayId, &str)], monitors: &[(DisplayId, &str)]) -> Vec<Screen> {
    capturer
        .iter()
        .map(|(id, label)| Screen {
            id: *id,
            stable: None,
            name: monitors
                .iter()
                .find(|(known, _)| known == id)
                .map_or(*label, |(_, name)| *name)
                .to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ScreenCaptureKit calls them "Screen 1" and "Screen 2". The display list
    // knows them by what is written on the monitor. The two are different
    // orderings of the same hardware, which is why a screen is only ever
    // identified by its display id and never by its position in a list.
    // Characterises the fallback written with the join above, not a new step.
    #[test]
    fn a_display_the_list_does_not_know_keeps_its_dull_label() {
        let screens = screens(&[(DisplayId(9), "Screen 1")], &[]);
        assert_eq!(
            screens,
            vec![Screen {
                id: DisplayId(9),
                name: "Screen 1".into(),
                stable: None,
            }]
        );
    }

    fn windows() -> Vec<Window> {
        vec![
            Window {
                id: WindowId(1),
                title: "Brave".into(),
                app: "Brave Browser".into(),
            },
            Window {
                id: WindowId(2),
                title: "remux - Brave".into(),
                app: "Brave Browser".into(),
            },
            Window {
                id: WindowId(3),
                title: "tmux a".into(),
                app: "Ghostty".into(),
            },
        ]
    }

    #[test]
    fn a_window_is_found_by_part_of_its_title() {
        assert_eq!(pick("tmux", &windows()).map(|w| w.id), Some(WindowId(3)));
    }

    #[test]
    fn the_case_does_not_matter_because_nobody_types_it_right() {
        assert_eq!(pick("GHOSTTY", &windows()).map(|w| w.id), Some(WindowId(3)));
    }

    // The CLI promises this is deterministic and opens no dialog, so two
    // matches cannot be a coin toss: an exact title beats a substring.
    #[test]
    fn an_exact_title_wins_over_a_longer_one_that_contains_it() {
        assert_eq!(pick("Brave", &windows()).map(|w| w.id), Some(WindowId(1)));
    }

    #[test]
    fn with_no_exact_match_the_first_in_order_wins_every_time() {
        assert_eq!(pick("Browser", &windows()).map(|w| w.id), Some(WindowId(1)));
    }

    #[test]
    fn asking_for_something_that_is_not_there_finds_nothing() {
        assert!(pick("Photoshop", &windows()).is_none());
    }

    fn raw(title: Option<&str>, app: Option<&str>, on_screen: bool) -> RawWindow {
        RawWindow {
            id: 4,
            title: title.map(str::to_string),
            app: app.map(str::to_string),
            on_screen,
        }
    }

    // A window the window server is not showing cannot be captured:
    // ScreenCaptureKit takes the filter and then delivers nothing, forever,
    // with no error at all. Offering it is offering a stream that will never
    // start.
    #[test]
    fn a_window_that_is_not_on_a_display_is_not_offered() {
        assert_eq!(
            window_from(raw(Some("Window"), Some("Ghostty"), false)),
            None
        );
    }

    #[test]
    fn a_window_the_system_named_is_a_window() {
        assert_eq!(
            window_from(RawWindow {
                id: 4,
                title: Some("tmux a".into()),
                app: Some("Ghostty".into()),
                on_screen: true
            }),
            Some(Window {
                id: WindowId(4),
                title: "tmux a".into(),
                app: "Ghostty".into()
            })
        );
    }

    // The desktop is full of untitled windows: shadows, menus, status items,
    // drag images. None of them is what anybody means by "share that window".
    #[test]
    fn a_window_with_no_title_is_not_a_window_anybody_means() {
        let untitled = raw(None, Some("Ghostty"), true);
        assert_eq!(window_from(untitled), None);

        let blank = raw(Some("   "), Some("Ghostty"), true);
        assert_eq!(window_from(blank), None, "whitespace is not a name");
    }

    // The trade is deliberate and it goes the other way from the title: an
    // application that will not name itself still owns a window you can share,
    // and losing the window over a missing label is the worse of the two.
    #[test]
    fn a_window_whose_app_will_not_name_itself_is_still_shareable() {
        let anonymous = raw(Some("Untitled"), None, true);
        assert_eq!(window_from(anonymous).map(|w| w.app), Some(String::new()));
    }

    #[test]
    fn a_screen_takes_the_name_the_monitor_carries() {
        let screens = screens(
            &[(DisplayId(2), "Screen 1"), (DisplayId(7), "Screen 2")],
            &[
                (DisplayId(7), "VG2791R"),
                (DisplayId(2), "Built-in Retina Display"),
            ],
        );

        assert_eq!(
            screens,
            vec![
                Screen {
                    id: DisplayId(2),
                    name: "Built-in Retina Display".into(),
                    stable: None,
                },
                Screen {
                    id: DisplayId(7),
                    name: "VG2791R".into(),
                    stable: None,
                },
            ]
        );
    }

    fn devices() -> Vec<Named> {
        vec![
            Named {
                id: "c9bf1690".into(),
                name: "HyperX DuoCast".into(),
            },
            Named {
                id: "dae50b11".into(),
                name: "HP 430/435 FHD Webcam microphone".into(),
            },
            Named {
                id: "ab41cc1e".into(),
                name: "MacBook Pro Microphone (Built-in)".into(),
            },
        ]
    }

    #[test]
    fn a_device_id_is_the_handle_that_survives_a_reboot() {
        assert_eq!(
            pick_device("dae50b11", &devices()).map(|d| d.name.as_str()),
            Some("HP 430/435 FHD Webcam microphone")
        );
    }

    #[test]
    fn a_device_is_also_found_by_part_of_its_name_the_way_the_cli_takes_it() {
        assert_eq!(
            pick_device("hyperx", &devices()).map(|d| d.id.as_str()),
            Some("c9bf1690")
        );
    }

    // "HP 430/435 FHD Webcam microphone" contains "microphone", and so does the
    // built-in. A person who typed the whole name meant that one.
    #[test]
    fn a_whole_name_beats_a_substring_of_another() {
        assert_eq!(
            pick_device("MacBook Pro Microphone (Built-in)", &devices()).map(|d| d.id.as_str()),
            Some("ab41cc1e")
        );
    }

    #[test]
    fn asking_for_a_device_that_is_not_plugged_in_finds_nothing() {
        assert!(pick_device("Rode NT-USB", &devices()).is_none());
        assert!(pick_device("anything", &[]).is_none());
    }
}
