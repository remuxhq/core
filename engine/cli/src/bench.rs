//! `remux bench export|import`: a setup packed as a JSON and the files its
//! scenes name, in one tarball, and unpacked on another machine beside what is
//! there. What a bench holds and never holds is `remuxd_domain::bench`; this is
//! the packing, the files and the conversation with the engine.

use std::path::{Path, PathBuf};
use std::process::Command as Shell;

use remuxd_domain::bench::{self, Bench, Requires};
use remuxd_domain::protocol::{Command, Reply, Status};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    Export(PathBuf),
    /// The tarball, and whether to import one made on another system.
    Import(PathBuf, bool),
}

impl Verb {
    pub fn parse(words: &[String]) -> Result<Self, String> {
        let anyway = words.iter().any(|w| w == "--anyway");
        let plain: Vec<&String> = words.iter().filter(|w| !w.starts_with("--")).collect();
        match plain.as_slice() {
            [verb, file] if *verb == "export" => Ok(Self::Export(file.into())),
            [verb, file] if *verb == "import" => Ok(Self::Import(file.into(), anyway)),
            _ => Err("bench: export <file.tar.gz>, or import <file.tar.gz> [--anyway]".into()),
        }
    }
}

type Ask<'a> = &'a mut dyn FnMut(&Command) -> Result<Reply, String>;

pub fn run(verb: &Verb, ask: Ask) -> Result<String, String> {
    match verb {
        Verb::Export(file) => export(file, crate::companion::listed().unwrap_or_default(), ask),
        Verb::Import(file, anyway) => import(
            file,
            *anyway,
            &remuxd_domain::os::config_dir().join("benches"),
            ask,
        ),
    }
}

fn status(ask: Ask) -> Result<Status, String> {
    match ask(&Command::Status)? {
        Reply::Status(status) => Ok(*status),
        Reply::Error { message } => Err(message),
        other => Err(format!("the engine answered {other:?}")),
    }
}

fn here(status: &Status) -> Requires {
    Requires {
        os: remuxd_domain::os::OS.name.into(),
        motor: status.motor.clone(),
    }
}

// A folder of our own for the packing, gone when the work is.
fn workbench() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("remux-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).map_err(|why| format!("bench: {why}"))?;
    Ok(dir)
}

fn tar(args: &[&str]) -> Result<(), String> {
    let done = Shell::new("tar")
        .args(args)
        .output()
        .map_err(|why| format!("bench: tar: {why}"))?;
    if done.status.success() {
        Ok(())
    } else {
        Err(format!(
            "bench: tar: {}",
            String::from_utf8_lossy(&done.stderr).trim()
        ))
    }
}

fn export(
    file: &Path,
    companions: Vec<remuxd_domain::companions::Companion>,
    ask: Ask,
) -> Result<String, String> {
    let status = status(ask)?;
    let mut scenes = status.scenes.clone();
    let files = bench::files(&scenes);
    let dir = workbench()?;
    for (at, path) in files.iter().enumerate() {
        std::fs::copy(path, dir.join(bench::packed(at, path))).map_err(|why| {
            let _ = std::fs::remove_dir_all(&dir);
            format!("bench: {path}, named by a scene: {why}")
        })?;
    }
    bench::relocate(&mut scenes, |path| {
        let at = files.iter().position(|f| f == path).unwrap_or_default();
        bench::packed(at, path)
    });
    let made = Bench {
        bench: bench::VERSION,
        requires: here(&status),
        scenes,
        audio_layers: status.audio_layers.clone(),
        gate: status.gate,
        faders: status.faders,
        // Where its secrets are read from is this machine's business.
        companions: companions
            .into_iter()
            .map(|c| remuxd_domain::companions::Companion {
                env_file: None,
                ..c
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&made).map_err(|why| why.to_string())?;
    std::fs::write(dir.join("bench.json"), json).map_err(|why| format!("bench: {why}"))?;
    let packed = tar(&[
        "-czf",
        &file.to_string_lossy(),
        "-C",
        &dir.to_string_lossy(),
        "bench.json",
        "files",
    ]);
    let _ = std::fs::remove_dir_all(&dir);
    packed?;
    Ok(format!(
        "{}: {} scenes, {} files, {} companions",
        file.display(),
        made.scenes.len(),
        files.len(),
        made.companions.len()
    ))
}

// Where a bench unpacks: a folder of its own under `into`, named after the
// tarball, never over another's.
fn landing(file: &Path, into: &Path) -> PathBuf {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name.trim_end_matches(".tar.gz").trim_end_matches(".tgz");
    let stem = if stem.is_empty() { "bench" } else { stem };
    let mut dir = into.join(stem);
    let mut n = 2;
    while dir.exists() {
        dir = into.join(format!("{stem}-{n}"));
        n += 1;
    }
    dir
}

fn import(file: &Path, anyway: bool, into: &Path, ask: Ask) -> Result<String, String> {
    let dir = landing(file, into);
    std::fs::create_dir_all(&dir).map_err(|why| format!("bench: {why}"))?;
    tar(&[
        "-xzf",
        &file.to_string_lossy(),
        "-C",
        &dir.to_string_lossy(),
    ])?;
    let text = std::fs::read_to_string(dir.join("bench.json"))
        .map_err(|why| format!("bench: {} holds no bench.json: {why}", file.display()))?;
    let mut made: Bench = serde_json::from_str(&text).map_err(|why| format!("bench: {why}"))?;
    if made.bench > bench::VERSION {
        return Err(format!(
            "bench: made by a newer remux (bench {}): update remux",
            made.bench
        ));
    }
    if let Some(differs) = bench::mismatch(&made.requires, &here(&status(ask)?)) {
        if !anyway {
            return Err(format!(
                "bench: {differs}: --anyway imports it all the same"
            ));
        }
    }
    bench::relocate(&mut made.scenes, |path| {
        if path.starts_with("files/") {
            dir.join(path).to_string_lossy().into_owned()
        } else {
            path.to_string()
        }
    });
    let devices = match ask(&Command::Sources)? {
        Reply::Sources(devices) => devices,
        _ => Default::default(),
    };
    let mut lines = Vec::new();
    for mut scene in made.scenes {
        let hidden = bench::hide_missing(&mut scene, &devices);
        match ask(&Command::SceneAdd {
            scene: Box::new(scene),
        })? {
            Reply::Status(status) => {
                let name = status
                    .scenes
                    .last()
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                lines.push(match hidden.is_empty() {
                    true => format!("scene {name} added"),
                    false => format!(
                        "scene {name} added; hidden, no such device here: {} (remux scene layer set --staged picks yours)",
                        hidden.join(", ")
                    ),
                });
            }
            Reply::Error { message } => lines.push(format!("a scene was refused: {message}")),
            other => return Err(format!("the engine answered {other:?}")),
        }
    }
    for companion in &made.companions {
        lines.push(format!(
            "companion {} listed, not started: {} (add it to your companions file to run it)",
            companion.name,
            companion.run.join(" ")
        ));
    }
    lines.push(format!(
        "not applied: the gate, the faders and {} audio layers, which stay yours; the files are in {}",
        made.audio_layers.len(),
        dir.display()
    ));
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use remuxd_domain::picture::layers::{Kind, Layer, Source, Transform};
    use remuxd_domain::picture::scenes::Scene;
    use remuxd_domain::protocol::{Devices, Named};

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("remux-bench-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn layer(id: &str, kind: Kind, handle: &str, name: &str) -> Layer {
        Layer {
            id: id.into(),
            source: Source {
                kind,
                handle: handle.into(),
                name: name.into(),
                width: 1920,
                height: 1080,
                stable: None,
            },
            transform: Transform::default(),
            visible: true,
            crop: None,
            shape: None,
            mirrored: false,
            shader: None,
        }
    }

    #[test]
    fn a_bench_exported_here_is_imported_there_with_its_pictures_and_without_missing_devices() {
        let mine = scratch("mine");
        std::fs::write(mine.join("bg.png"), b"a picture").unwrap();
        let picture = mine.join("bg.png").to_string_lossy().into_owned();
        let scene = Scene {
            name: "Starting soon".into(),
            layers: vec![
                layer("bg", Kind::Image, &picture, "bg.png"),
                layer("face", Kind::Camera, "0x1", "Their camera"),
            ],
            order: vec![],
            elements: vec![],
            shader: None,
        };
        let mut status = Status {
            scenes: vec![scene],
            motor: "obs 30.2.3".into(),
            ..Status::default()
        };
        let tarball = mine.join("setup.tar.gz");
        let mut engine = |command: &Command| match command {
            Command::Status => Ok(Reply::Status(Box::new(status.clone()))),
            other => Err(format!("unexpected {other:?}")),
        };
        let said = export(&tarball, vec![], &mut engine).expect("exported");
        assert!(said.contains("1 scenes, 1 files"), "{said}");

        let theirs = scratch("theirs");
        let mut added = Vec::new();
        let mut engine = |command: &Command| match command {
            Command::Status => Ok(Reply::Status(Box::new(status.clone()))),
            Command::Sources => Ok(Reply::Sources(Devices {
                cameras: vec![Named {
                    id: "0x2".into(),
                    name: "Another".into(),
                }],
                ..Devices::default()
            })),
            Command::SceneAdd { scene } => {
                added.push((**scene).clone());
                status.scenes.push((**scene).clone());
                Ok(Reply::Status(Box::new(status.clone())))
            }
            other => Err(format!("unexpected {other:?}")),
        };
        let said = import(&tarball, false, &theirs, &mut engine).expect("imported");
        assert!(said.contains("hidden, no such device here: face"), "{said}");
        let [scene] = added.as_slice() else {
            panic!("one scene: {added:?}")
        };
        let picture = &scene.layers[0].source.handle;
        assert!(picture.starts_with(&*theirs.to_string_lossy()), "{picture}");
        assert_eq!(std::fs::read(picture).unwrap(), b"a picture");
        assert!(!scene.layers[1].visible);
    }

    #[test]
    fn a_bench_from_another_system_is_refused_unless_asked() {
        let dir = scratch("other");
        let made = Bench {
            bench: bench::VERSION,
            requires: Requires {
                os: "elsewhere".into(),
                motor: "obs 30".into(),
            },
            scenes: vec![],
            audio_layers: vec![],
            gate: Default::default(),
            faders: Status::default().faders,
            companions: vec![],
        };
        std::fs::create_dir_all(dir.join("in/files")).unwrap();
        std::fs::write(
            dir.join("in/bench.json"),
            serde_json::to_string(&made).unwrap(),
        )
        .unwrap();
        let tarball = dir.join("other.tar.gz");
        tar(&[
            "-czf",
            &tarball.to_string_lossy(),
            "-C",
            &dir.join("in").to_string_lossy(),
            "bench.json",
            "files",
        ])
        .unwrap();
        let status = Status {
            motor: "obs 30".into(),
            ..Status::default()
        };
        let mut engine = |command: &Command| match command {
            Command::Status => Ok(Reply::Status(Box::new(status.clone()))),
            Command::Sources => Ok(Reply::Sources(Devices::default())),
            other => Err(format!("unexpected {other:?}")),
        };
        let refused = import(&tarball, false, &dir.join("benches"), &mut engine).unwrap_err();
        assert!(
            refused.contains("made on elsewhere") && refused.contains("--anyway"),
            "{refused}"
        );
        assert!(import(&tarball, true, &dir.join("benches"), &mut engine).is_ok());
    }

    #[test]
    fn the_words_name_the_file_and_anyway() {
        let w = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(
            Verb::parse(&w("export a.tar.gz")),
            Ok(Verb::Export("a.tar.gz".into()))
        );
        assert_eq!(
            Verb::parse(&w("import a.tar.gz --anyway")),
            Ok(Verb::Import("a.tar.gz".into(), true))
        );
        assert!(Verb::parse(&w("import")).is_err());
    }
}
