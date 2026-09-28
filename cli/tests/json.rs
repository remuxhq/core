use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::process::Command;

fn remux(args: &[&str], socket: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_remux"))
        .args(args)
        .env("REMUXD_SOCKET", socket)
        .output()
        .expect("run CLI")
}

#[test]
fn json_help_and_parse_errors_need_no_engine() {
    let missing = std::env::temp_dir().join(format!("remux-json-missing-{}", std::process::id()));
    let help = remux(&["help", "destination", "arm", "--json"], &missing);
    assert!(help.status.success());
    let value: serde_json::Value = serde_json::from_slice(&help.stdout).unwrap();
    assert!(value["help"].as_str().unwrap().contains("destination arm"));
    assert!(help.stderr.is_empty());

    let bad = remux(&["--json", "destination", "arm", "not-an-id"], &missing);
    assert_eq!(bad.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&bad.stdout).unwrap();
    assert_eq!(value["reply"], "error");
    assert!(value["message"].as_str().unwrap().contains("number"));
    assert!(bad.stderr.is_empty());

    let absent = remux(&["status", "--json"], &missing);
    assert_eq!(absent.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&absent.stdout).unwrap();
    assert_eq!(value["reply"], "error");
    assert!(absent.stderr.is_empty());
}

#[test]
fn guide_is_available_without_an_engine_in_text_and_json() {
    let missing = std::env::temp_dir().join(format!("remux-guide-missing-{}", std::process::id()));
    let text = remux(&["guide"], &missing);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("remux status --json"));
    assert!(text.contains("Never use these as probes"));

    let result = remux(&["--json", "guide"], &missing);
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["guide"], text);

    for args in [["help", "guide"], ["guide", "--help"]] {
        let help = remux(&args, &missing);
        assert!(help.status.success());
        assert!(String::from_utf8(help.stdout)
            .unwrap()
            .contains("Usage: remux guide"));
    }
    let bad = remux(&["guide", "unexpected", "--json"], &missing);
    assert_eq!(bad.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bad.stdout).unwrap()["reply"],
        "error"
    );
}

#[test]
fn json_reply_is_the_engine_reply_not_rendered_text() {
    let path = std::path::PathBuf::from(format!("/tmp/remux-json-{}.sock", std::process::id()));
    let listener = UnixListener::bind(&path).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 256];
        let size = stream.read(&mut request).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&request[..size]).unwrap()["cmd"],
            "status"
        );
        stream.write_all(b"{\"reply\":\"ok\"}\n").unwrap();
    });
    let result = remux(&["--json", "status"], &path);
    server.join().unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(result.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()["reply"],
        "ok"
    );
    assert!(result.stderr.is_empty());
}
