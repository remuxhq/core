//! What can be captured, read off the capture sources' own property lists:
//! displays by uuid, cameras and microphones by the ids their plugins use.
//! The domain numbers displays (`DisplayId`); the uuid each number stands
//! for is kept here, for the pipeline.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::sync::{Arc, Mutex};

use remuxd_domain::engine::{Available, Sources};
use remuxd_domain::protocol::Named;
use remuxd_domain::sources::{DisplayId, Screen, Window, WindowId};

use crate::{c, ffi};

/// The displays by the number a face uses, with the uuid libobs wants.
#[derive(Default)]
pub struct Known {
    pub displays: BTreeMap<u32, String>,
    /// The applications by name, with the bundle id `sck_audio_capture` wants.
    pub apps: BTreeMap<String, String>,
}

pub struct ObsSources {
    known: Arc<Mutex<Known>>,
}

impl ObsSources {
    pub fn new(known: Arc<Mutex<Known>>) -> Self {
        Self { known }
    }
}

/// One list property of one source type, as (name, value) pairs; the empty
/// first row the plugins put in for "none" is dropped.
pub(crate) fn list(source: &str, property: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // SAFETY: a source is created and released here; the properties are read
    // and destroyed before it goes; every string is copied out.
    unsafe {
        let made = ffi::obs_source_create(
            c(source).as_ptr(),
            c("probe").as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if made.is_null() {
            return out;
        }
        let props = ffi::obs_source_properties(made);
        if !props.is_null() {
            let p = ffi::obs_properties_get(props, c(property).as_ptr());
            if !p.is_null() {
                for i in 0..ffi::obs_property_list_item_count(p) {
                    let name = CStr::from_ptr(ffi::obs_property_list_item_name(p, i))
                        .to_string_lossy()
                        .into_owned();
                    let value = ffi::obs_property_list_item_string(p, i);
                    // A list of numbers (the windows) has no string: the number is the value.
                    let value = if value.is_null() {
                        ffi::obs_property_list_item_int(p, i).to_string()
                    } else {
                        CStr::from_ptr(value).to_string_lossy().into_owned()
                    };
                    if !value.trim().is_empty() {
                        out.push((name, value));
                    }
                }
            }
            ffi::obs_properties_destroy(props);
        }
        ffi::obs_source_release(made);
    }
    out
}

impl Sources for ObsSources {
    fn available(&self) -> Result<Available, String> {
        let table = crate::platform::screen();
        let displays = if table.portal {
            vec![(
                "the portal's pick (a dialog asks once)".to_string(),
                "portal".to_string(),
            )]
        } else {
            list(table.source, table.displays)
        };
        let mut known = self
            .known
            .lock()
            .map_err(|_| "the display list is poisoned")?;
        known.apps = table
            .apps
            .map(|apps| list(table.source, apps).into_iter().collect())
            .unwrap_or_default();
        known.displays = displays
            .iter()
            .enumerate()
            .map(|(i, (_, uuid))| (i as u32 + 1, uuid.clone()))
            .collect();
        let named = |rows: Vec<(String, String)>| {
            rows.into_iter()
                .map(|(name, id)| Named { id, name })
                .collect::<Vec<_>>()
        };
        Ok(Available {
            screens: displays
                .iter()
                .enumerate()
                .map(|(i, (name, _))| Screen {
                    id: DisplayId(i as u32 + 1),
                    name: name.split(':').next().unwrap_or(name).to_string(),
                })
                .collect(),
            // "App: title" with the window id as the value, off the same source.
            windows: if table.portal {
                Vec::new()
            } else {
                list(table.window_source, table.windows)
            }
            .into_iter()
            .filter_map(|(name, id)| {
                // macOS: "[App] title", the id as the number. Linux
                // (xcomposite): the value is "id\r\nname\r\nclass".
                let (app, title) = name
                    .strip_prefix('[')
                    .and_then(|rest| rest.split_once("] "))
                    .unwrap_or(("", &name));
                let id: u32 = id.split("\r\n").next()?.parse().ok().filter(|n| *n > 0)?;
                Some(Window {
                    id: WindowId(id),
                    title: title.to_string(),
                    app: app.to_string(),
                })
            })
            .collect(),
            cameras: if (crate::platform::TABLE.camera.probe)() {
                named(list(
                    crate::platform::TABLE.camera.source,
                    crate::platform::TABLE.camera.devices,
                ))
            } else {
                Vec::new()
            },
            mics: named(list(
                crate::platform::TABLE.mic.source,
                crate::platform::TABLE.mic.devices,
            )),
            apps: table
                .apps
                .map(|apps| {
                    list(table.source, apps)
                        .into_iter()
                        .map(|(name, _)| name)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}
