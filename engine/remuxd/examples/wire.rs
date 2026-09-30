//! A chat source of your own, the smallest one: serves the `line` half of
//! the wire on ws://127.0.0.1:9999 and says a line a second to whoever is
//! connected. `cargo run -p remuxd --example wire [port]`, then
//! `remux chat --url ws://127.0.0.1:9999`. See docs/wire.md.

use std::net::TcpListener;
use std::time::Duration;

use remuxd_domain::app::wire::Line;

fn main() {
    let port = std::env::args().nth(1).unwrap_or_else(|| "9999".into());
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).expect("the port");
    eprintln!("wire: ws://127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let port = port.clone();
        std::thread::spawn(move || {
            let Ok(mut socket) = tungstenite::accept(stream) else {
                return;
            };
            socket.get_ref().set_nonblocking(true).ok();
            for n in 1.. {
                // what the engine sends up: a delete, a say, printed and ignored
                if let Ok(tungstenite::Message::Text(text)) = socket.read() {
                    eprintln!("wire: got {text}");
                }
                let line = Line {
                    id: format!("m{n}"),
                    platform: "example".into(),
                    channel: "here".into(),
                    from: format!("wire {port}"),
                    body: format!("line {n}"),
                };
                let frame = serde_json::json!({ "line": line }).to_string();
                if socket.send(tungstenite::Message::Text(frame)).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }
}
