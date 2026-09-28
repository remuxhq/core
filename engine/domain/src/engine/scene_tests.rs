use super::*;
use crate::layers::{Kind, Layer, Source, Transform};
use crate::scenes::Scene;

fn text(id: &str, line: &str) -> Element {
    Element {
        id: id.into(),
        x: 120,
        y: 60,
        width: 640,
        height: 160,
        visible: true,
        shader: None,
        content: ElementContent::Text { text: line.into() },
    }
}

#[test]
fn elements_are_ordered_persist_and_render_without_capture() {
    let fake = super::Wrote::default();
    let shown = fake.shown.clone();
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    assert_eq!(engine.status().scenes.len(), 1);
    let a = text("first", "Welcome");
    let b = text("second", "Hello");
    assert!(matches!(
        engine.handle(Command::SceneElementAdd { element: a.clone() }),
        Reply::Status(_)
    ));
    assert!(matches!(
        engine.handle(Command::SceneElementAdd { element: b.clone() }),
        Reply::Status(_)
    ));
    assert_eq!(engine.status().scenes[0].elements, [a.clone(), b.clone()]);
    assert_eq!(shown.lock().unwrap().last().unwrap().0, [a, b]);
    let saved = crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
    let mut restored = Engine::new();
    restored.restore(&saved);
    assert_eq!(restored.status().scenes[0].elements.len(), 2);
    assert!(matches!(
        engine.handle(Command::SceneElementRemove { id: "first".into() }),
        Reply::Status(_)
    ));
    assert_eq!(engine.status().scenes[0].elements[0].id, "second");
}

#[test]
fn captures_and_generated_layers_share_one_order_and_identity() {
    let mut engine = Engine::with_sources(Box::new(super::ThisMachine))
        .with_pipeline(Box::new(super::Wrote::default()));
    engine.handle(Command::LayerScreen {
        id: "desk".into(),
        display: 1,
    });
    engine.handle(Command::SceneElementAdd {
        element: text("label", "Hi"),
    });
    engine.handle(Command::LayerCamera {
        id: "face".into(),
        device: "MacBook Pro Camera".into(),
    });
    engine.handle(Command::Mirror { on: true });
    engine.handle(Command::LayerMirror {
        id: "face".into(),
        on: false,
    });
    assert!(
        !engine
            .status()
            .layers
            .iter()
            .find(|layer| layer.id == "face")
            .unwrap()
            .mirrored
    );
    engine.handle(Command::LayerMirror {
        id: "face".into(),
        on: true,
    });
    assert!(
        engine
            .status()
            .layers
            .iter()
            .find(|layer| layer.id == "face")
            .unwrap()
            .mirrored
    );
    assert_eq!(
        engine.status().scenes[0].ordered_ids(),
        ["desk", "label", "face"]
    );
    assert!(matches!(
        engine.handle(Command::SceneElementAdd {
            element: text("desk", "duplicate")
        }),
        Reply::Error { .. }
    ));
    engine.handle(Command::LayerMove {
        id: "label".into(),
        index: 2,
    });
    assert_eq!(
        engine.status().scenes[0].ordered_ids(),
        ["desk", "face", "label"]
    );
    engine.handle(Command::LayerVisible {
        id: "label".into(),
        on: false,
    });
    assert!(!engine.status().scenes[0].elements[0].visible);
    let saved = crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
    let mut restored = Engine::with_sources(Box::new(super::ThisMachine))
        .with_pipeline(Box::new(super::Wrote::default()));
    restored.restore(&saved);
    assert_eq!(
        restored.status().scenes[0].ordered_ids(),
        ["desk", "face", "label"]
    );
    assert!(
        restored
            .status()
            .layers
            .iter()
            .find(|layer| layer.id == "face")
            .unwrap()
            .mirrored
    );
    restored.handle(Command::LayerRemove { id: "label".into() });
    assert_eq!(restored.status().scenes[0].ordered_ids(), ["desk", "face"]);
}

#[test]
fn generated_shader_is_atomic_persistent_and_validated_on_switch() {
    let mut engine = Engine::new().with_pipeline(Box::new(super::Wrote::default()));
    engine.handle(Command::SceneElementAdd {
        element: text("title", "Before"),
    });
    assert!(matches!(
        engine.handle(Command::LayerShader {
            id: "title".into(),
            path: Some("good.frag".into())
        }),
        Reply::Status(_)
    ));
    assert_eq!(
        engine.status().scenes[0].elements[0].shader.as_deref(),
        Some("good.frag")
    );
    assert!(matches!(
        engine.handle(Command::LayerShader {
            id: "title".into(),
            path: Some("bad.frag".into())
        }),
        Reply::Error { .. }
    ));
    assert_eq!(
        engine.status().scenes[0].elements[0].shader.as_deref(),
        Some("good.frag")
    );
    let mut invalid = text("invalid", "No");
    invalid.shader = Some("bad.frag".into());
    assert!(matches!(
        engine.handle(Command::SceneElementAdd { element: invalid }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status().scenes[0].elements.len(), 1);
    engine.handle(Command::SceneCreate {
        name: "next".into(),
    });
    assert!(matches!(
        engine.handle(Command::LayerShader {
            id: "title".into(),
            path: None
        }),
        Reply::Status(_)
    ));
    engine.handle(Command::SceneSwitch {
        name: "default".into(),
    });
    assert_eq!(
        engine
            .status()
            .scenes
            .iter()
            .find(|s| s.name == "default")
            .unwrap()
            .elements[0]
            .shader
            .as_deref(),
        Some("good.frag")
    );
    let saved = crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
    assert_eq!(
        saved.scenes[0].elements[0].shader.as_deref(),
        Some("good.frag")
    );

    let mut restored = Engine::new().with_pipeline(Box::new(super::Wrote {
        refuse: Some("shader:invalid".into()),
        ..Default::default()
    }));
    restored.restore(&saved);
    assert_eq!(
        restored.status().scenes[0].elements[0].shader.as_deref(),
        Some("good.frag")
    );
    restored.handle(Command::LayerShader {
        id: "title".into(),
        path: None,
    });
    restored.handle(Command::SceneCreate { name: "bad".into() });
    restored
        .status
        .scenes
        .iter_mut()
        .find(|s| s.name == "bad")
        .unwrap()
        .elements[0]
        .shader = Some("bad.frag".into());
    restored.handle(Command::SceneSwitch {
        name: "default".into(),
    });
    let previous = restored.status.active_scene.clone();
    assert!(matches!(
        restored.handle(Command::SceneSwitch { name: "bad".into() }),
        Reply::Error { .. }
    ));
    assert_eq!(restored.status.active_scene, previous);
}

#[test]
fn timers_are_transient_per_scene_and_never_switch_automatically() {
    let fake = super::Wrote::default();
    let shown = fake.shown.clone();
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    let timer = Element {
        id: "clock".into(),
        x: 600,
        y: 400,
        width: 700,
        height: 180,
        visible: true,
        shader: None,
        content: ElementContent::Timer { seconds: 0 },
    };
    engine.handle(Command::SceneElementAdd {
        element: timer.clone(),
    });
    engine.handle(Command::SceneTimerStart { id: "clock".into() });
    assert_eq!(shown.lock().unwrap().last().unwrap().1[0].1, Duration::ZERO);
    assert_eq!(engine.status().active_scene, "default");
    engine.handle(Command::SceneElementSet {
        element: Element {
            content: ElementContent::Timer { seconds: 90 },
            ..timer.clone()
        },
    });
    engine.handle(Command::SceneTimerStart { id: "clock".into() });
    assert!(shown.lock().unwrap().last().unwrap().1[0].1 > Duration::from_secs(89));
    engine.handle(Command::SceneTimerStop { id: "clock".into() });
    assert!(engine.counting.is_empty());
    engine.handle(Command::SceneElementSet {
        element: timer.clone(),
    });
    engine.handle(Command::SceneTimerStart { id: "clock".into() });
    let saved = engine.remembered();
    assert!(crate::remembered::write(&saved)
        .unwrap()
        .contains("\"seconds\": 0"));
    assert!(!crate::remembered::write(&saved)
        .unwrap()
        .contains("deadline"));
    engine.handle(Command::SceneCreate {
        name: "other".into(),
    });
    assert!(engine.counting.is_empty());
    engine.handle(Command::SceneSwitch {
        name: "default".into(),
    });
    assert!(engine.counting.is_empty());
    let mut restarted = Engine::new();
    restarted.restore(&saved);
    assert_eq!(restarted.status().scenes[0].elements, [timer]);
    assert!(restarted.counting.is_empty());
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

fn layer(id: &str, kind: Kind, handle: &str, x: i32) -> Layer {
    Layer {
        id: id.into(),
        source: Source {
            kind,
            handle: handle.into(),
            name: id.into(),
            width: 640,
            height: 480,
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
    let fake = super::Wrote::default();
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
    let fake = super::Wrote {
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
    let fake = super::Wrote {
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
    let mut engine = Engine::with_sources(Box::new(super::ThisMachine));
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
    let mut restored = Engine::with_sources(Box::new(super::ThisMachine));
    restored.restore(&saved);
    assert_eq!(restored.status.active_scene, "camera");
    assert_eq!(restored.status.layers[0].id, "second");
    assert_eq!(restored.remembered().scenes[0].layers[0].transform.x, 17);
}

#[test]
fn shader_commands_and_scene_choices_round_trip() {
    let words = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
    assert_eq!(
        crate::cli::parse(&words("scene layer filter face effect.frag")),
        Ok(Command::LayerShader {
            id: "face".into(),
            path: Some("effect.frag".into())
        })
    );
    assert_eq!(
        crate::cli::parse(&words("scene layer filter face off")),
        Ok(Command::LayerShader {
            id: "face".into(),
            path: None
        })
    );
    assert!(crate::cli::parse(&words("scene layer filter face")).is_err());
    let mut engine = Engine::new().with_pipeline(Box::new(super::Wrote::default()));
    engine.status.layers = vec![layer("face", Kind::Camera, "cam", 0)];
    let set = engine.handle(Command::LayerShader {
        id: "face".into(),
        path: Some("layer.frag".into()),
    });
    let Reply::Status(status) = set else {
        panic!("expected status")
    };
    assert_eq!(status.layers[0].shader.as_deref(), Some("layer.frag"));
    engine.handle(Command::Shader {
        path: Some("global.frag".into()),
    });
    engine.handle(Command::SceneCreate {
        name: "second".into(),
    });
    engine.handle(Command::LayerShader {
        id: "face".into(),
        path: None,
    });
    engine.handle(Command::Shader { path: None });
    engine.handle(Command::SceneSwitch {
        name: "default".into(),
    });
    let status = engine.status();
    assert_eq!(status.shader.as_deref(), Some("global.frag"));
    assert_eq!(status.layers[0].shader.as_deref(), Some("layer.frag"));
    let saved = crate::remembered::read(&crate::remembered::write(&engine.remembered()).unwrap());
    assert_eq!(saved.scenes[0].shader.as_deref(), Some("global.frag"));
    assert_eq!(
        saved.scenes[0].layers[0].shader.as_deref(),
        Some("layer.frag")
    );
    let second = saved
        .scenes
        .iter()
        .find(|scene| scene.name == "second")
        .unwrap();
    assert_eq!(second.shader, None);
    assert_eq!(second.layers[0].shader, None);
    assert_eq!(
        crate::remembered::read("{\"layers\":[],\"mic\":\"old\"}").scenes,
        []
    );
}

#[test]
fn invalid_layer_assignment_and_scene_shader_roll_back_without_stopping_live() {
    let fake = super::Wrote {
        refuse: Some("shader:invalid".into()),
        ..Default::default()
    };
    let events = fake.scene_events.clone();
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    let old = layer("old", Kind::Camera, "cam", 0);
    engine.status.layers = vec![old.clone()];
    engine.status.on_air = true;
    let mut next = layer("new", Kind::Screen, "display", 0);
    next.shader = Some("bad.frag".into());
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
            path: Some("bad.frag".into())
        }),
        Reply::Error { .. }
    ));
    assert_eq!(engine.status.layers[0].shader, None);
    engine.status.scenes.push(Scene {
        name: "global".into(),
        layers: vec![],
        elements: vec![],
        order: vec![],
        shader: Some("bad.frag".into()),
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
    let fake = super::Wrote {
        refuse: Some("runtime:layer".into()),
        ..Default::default()
    };
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    let mut disabled = layer("disabled", Kind::Camera, "cam", 0);
    disabled.shader = Some("failed.frag".into());
    let mut healthy = layer("healthy", Kind::Screen, "1", 0);
    healthy.shader = Some("good.frag".into());
    engine.status.layers = vec![disabled, healthy];
    engine.status.shader = Some("scene.frag".into());
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status")
    };
    assert_eq!(status.layers[0].shader, None);
    assert_eq!(status.layers[1].shader.as_deref(), Some("good.frag"));
    assert_eq!(status.shader.as_deref(), Some("scene.frag"));
    assert_eq!(engine.remembered().layers[0].shader, None);
    assert_eq!(
        engine.remembered().layers[1].shader.as_deref(),
        Some("good.frag")
    );

    let fake = super::Wrote {
        refuse: Some("runtime:global".into()),
        ..Default::default()
    };
    let mut engine = Engine::new().with_pipeline(Box::new(fake));
    engine.status.shader = Some("failed.frag".into());
    engine.status.layers = vec![layer("healthy", Kind::Camera, "cam", 0)];
    engine.status.layers[0].shader = Some("good.frag".into());
    let Reply::Status(status) = engine.handle(Command::Status) else {
        panic!("status")
    };
    assert_eq!(status.shader, None);
    assert_eq!(status.layers[0].shader.as_deref(), Some("good.frag"));
    assert_eq!(engine.remembered().scenes[0].shader, None);
}

#[test]
fn restore_skips_bad_shader_paths_without_losing_sources_or_other_scenes() {
    let mut engine = Engine::with_sources(Box::new(super::ThisMachine))
        .with_pipeline(Box::new(super::Wrote::default()));
    let mut saved_layer = layer("desktop", Kind::Screen, "1", 0);
    saved_layer.shader = Some("bad.frag".into());
    let setup = crate::remembered::Remembered {
        scenes: vec![
            Scene {
                name: "default".into(),
                layers: vec![saved_layer],
                elements: vec![],
                order: vec![],
                shader: Some("bad.frag".into()),
            },
            Scene {
                name: "later".into(),
                layers: vec![],
                elements: vec![],
                order: vec![],
                shader: Some("good.frag".into()),
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
        Some("good.frag")
    );
}

#[test]
fn scene_cli_parses_names_and_shows_list() {
    let words = |items: &[&str]| {
        items
            .iter()
            .map(|item| (*item).into())
            .collect::<Vec<String>>()
    };
    assert_eq!(
        crate::cli::parse(&words(&["scene", "create", "Close Up"])),
        Ok(Command::SceneCreate {
            name: "Close Up".into()
        })
    );
    assert_eq!(
        crate::cli::parse(&words(&["scene", "switch", "Close Up"])),
        Ok(Command::SceneSwitch {
            name: "Close Up".into()
        })
    );
    assert_eq!(
        crate::cli::parse(&words(&["scene", "delete", "Close Up"])),
        Ok(Command::SceneDelete {
            name: "Close Up".into()
        })
    );
    assert_eq!(
        crate::cli::parse(&words(&["scene", "list"])),
        Ok(Command::Status)
    );
    assert!(crate::cli::parse(&words(&["scene", "create"])).is_err());
    assert!(crate::cli::render_scene_list(&Status::default()).contains("* default"));
}

#[test]
fn clone_edit_delete_and_retain_inactive_layouts() {
    let mut engine = Engine::new();
    assert!(matches!(
        engine.handle(Command::SceneCreate {
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
