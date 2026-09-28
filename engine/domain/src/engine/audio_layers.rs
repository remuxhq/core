//! Independent audio layers: one capture per ID, separate from the legacy
//! one-mic/one-app controls. Never report a source that failed to open.
use super::*;
use crate::audio_layers::{Layer, Source};

impl Engine {
    pub(super) fn audio_layer_add(&mut self, id: String, source: Source) -> Reply {
        let layer = match Layer::new(id, source) {
            Ok(layer) => layer,
            Err(message) => return Reply::Error { message },
        };
        if self.status.audio_layers.len() >= 16 {
            return Reply::Error {
                message: "at most 16 audio layers can be open".into(),
            };
        }
        if self.status.audio_layers.iter().any(|l| l.id == layer.id) {
            return Reply::Error {
                message: format!("audio layer {:?} already exists", layer.id),
            };
        }
        if let Err(message) = self.pipeline.audio_layer_add(&layer) {
            return Reply::Error { message };
        }
        self.status.audio_layers.push(layer);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn audio_layer_remove(&mut self, id: String) -> Reply {
        let Some(index) = self.status.audio_layers.iter().position(|l| l.id == id) else {
            return Reply::Error {
                message: format!("no audio layer {id:?}"),
            };
        };
        self.pipeline.audio_layer_remove(&id);
        self.status.audio_layers.remove(index);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn audio_layer_volume(&mut self, id: String, volume: f64) -> Reply {
        if !volume.is_finite() || !(0.0..=2.0).contains(&volume) {
            return Reply::Error {
                message: "audio layer volume must be between 0 and 2".into(),
            };
        }
        let Some(layer) = self.status.audio_layers.iter_mut().find(|l| l.id == id) else {
            return Reply::Error {
                message: format!("no audio layer {id:?}"),
            };
        };
        layer.volume = volume;
        self.pipeline.audio_layer_levels(&id, volume, layer.muted);
        Reply::Status(Box::new(self.reported()))
    }

    pub(super) fn audio_layer_mute(&mut self, id: String, on: bool) -> Reply {
        let Some(layer) = self.status.audio_layers.iter_mut().find(|l| l.id == id) else {
            return Reply::Error {
                message: format!("no audio layer {id:?}"),
            };
        };
        layer.muted = on;
        self.pipeline.audio_layer_levels(&id, layer.volume, on);
        Reply::Status(Box::new(self.reported()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::*;

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
}
