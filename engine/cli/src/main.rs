//! The engine, from a shell.
//!
//! One of the faces. It opens the socket, writes a line and prints what comes
//! back, and everything it knows about what the words mean is in `words`,
//! where `cargo test` can reach it. This file is the
//! part that cannot be tested without a daemon, so there is as little of it as
//! there can be.

use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::os::unix::net::UnixStream;

mod companion;
mod daemon;
mod tui;
mod words;

use remuxd_domain::protocol::{decode_reply, encode, Command, Reply};
use words::{self as cli, Format, Ink, View};

/// A failure, the way the words asked for it: on stderr for a person, as
/// the socket's error reply on stdout for a program.
fn fail(why: &str, json: bool, code: i32) -> ! {
    if json {
        println!(
            "{}",
            cli::render_json(&Reply::Error {
                message: why.into()
            })
        );
    } else {
        eprintln!("{why}");
    }
    std::process::exit(code);
}

fn main() {
    let words: Vec<String> = std::env::args().skip(1).collect();
    let (json, bare) = cli::output_mode(&words);
    // Help and the guide are answered here, with no engine.
    if let Some(said) = cli::help(&bare) {
        match said {
            Ok(text) if json => println!("{}", serde_json::json!({ "help": text })),
            Ok(text) => println!("{text}"),
            Err(why) => fail(&why, json, 2),
        }
        return;
    }
    if let Some(said) = cli::guide(&bare) {
        match said {
            Ok(text) if json => println!("{}", serde_json::json!({ "guide": text })),
            Ok(text) => print!("{text}"),
            Err(why) => fail(&why, json, 2),
        }
        return;
    }
    let ask = cli::read(&words).unwrap_or_else(|why| fail(&why, json, 2));

    let path = remuxd_domain::socket::default_path();
    let ink = Ink::for_terminal(
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").is_some(),
    );
    match (&ask.view, ask.follow) {
        (View::Reply, true) if matches!(ask.command, Some(Command::Levels)) => {
            follow_the_levels(&path, ask.format)
        }
        (View::Reply, true) if matches!(ask.command, Some(Command::Events { .. })) => {
            follow_the_events(&path, ask.format)
        }
        (View::Reply, true) => follow_the_chat(&path, ask.format, ink),
        (View::Log, true) => follow_the_log(&path, ask.format),
        _ => {}
    }
    let Some(command) = &ask.command else {
        match &ask.view {
            View::DestinationAdd {
                platform,
                name,
                url,
                key_from,
            } => keep_a_destination(platform, name, url, key_from),
            View::DestinationRemove(which) => forget_a_destination(which),
            View::Login { base } => log_in(base),
            View::Logout => log_out(),
            View::ChatKeep(url) => keep_a_chat_source(&path, url),
            View::Daemon(verb) => daemon::run_verb(verb, &path, &self::ask),
            View::Bug { open } => report_a_bug(&path, open.as_deref()),
            View::Companion(verb) => match companion::run(verb) {
                Ok(said) => {
                    println!("{said}");
                    return;
                }
                Err(why) => fail(&why, json, 1),
            },
            View::Tui => {
                if let Err(why) = tui::run(|command| self::ask(&path, command)) {
                    fail(&why.to_string(), json, 1);
                }
                return;
            }
            _ => {}
        }
        println!("{}", cli::local(&ask.view, ask.format));
        return;
    };
    if let View::Wait { until, for_secs } = &ask.view {
        wait_until(&path, until, *for_secs);
    }
    if let View::Health = &ask.view {
        report_health(&path, ask.format);
    }
    let reply = match ask_or_exit(&path, command) {
        // The answer to a search arrives on the status a moment later.
        Reply::Ok if matches!(ask.view, View::Categories { .. }) => {
            wait_for_categories(&path, &ask.view)
        }
        reply => reply,
    };
    // `remux live` at a terminal: the plan, a yes, then the live on that plan.
    // Anywhere else there is nobody to ask, and the fix is named.
    if let (View::Confirm, Reply::Plan(plan)) = (&ask.view, &reply) {
        println!(
            "{}",
            cli::show(&reply, &View::Reply, Format::Prose, ink, now())
        );
        if !plan.blockers.is_empty() {
            std::process::exit(1);
        }
        if !std::io::stdin().is_terminal() {
            eprintln!(
                "nobody to ask here: run `remux plan --json`, then `remux live --confirm <plan>`"
            );
            std::process::exit(2);
        }
        eprint!("go live? [y/N] ");
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("not going live");
            std::process::exit(1);
        }
        let live = ask_or_exit(
            &path,
            &Command::Live {
                plan: plan.fingerprint,
            },
        );
        let failed = matches!(live, Reply::Error { .. });
        println!("{}", cli::show(&live, &View::Reply, ask.format, ink, now()));
        std::process::exit(if failed { 1 } else { 0 });
    }
    let failed = matches!(reply, Reply::Error { .. });
    if let (View::ShotTo(file), Some(bytes)) = (&ask.view, cli::jpeg_bytes(&reply)) {
        if let Err(e) = std::fs::write(file, bytes) {
            eprintln!("could not write {file}: {e}");
            std::process::exit(1);
        }
        println!("{file}");
        return;
    }
    println!("{}", cli::show(&reply, &ask.view, ask.format, ink, now()));
    if failed {
        std::process::exit(1);
    }
}

/// `remux wait`: the status, every quarter second, until the condition
/// holds or the budget runs out.
fn wait_until(path: &std::path::Path, until: &remuxd_domain::wait::Until, for_secs: u64) -> ! {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(for_secs);
    loop {
        if let Reply::Status(status) = ask_or_exit(path, &Command::Status) {
            if until.met(&status) {
                println!("{}", until.words());
                std::process::exit(0);
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("still waiting for {} after {for_secs} s", until.words());
            std::process::exit(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// `remux health`: the status and the grants, joined; exit 1 when anything
/// stands in the way, so a script can ask before it plans.
fn report_health(path: &std::path::Path, format: Format) -> ! {
    use remuxd_domain::health;
    let Reply::Status(status) = ask_or_exit(path, &Command::Status) else {
        eprintln!("the engine did not answer with its status");
        std::process::exit(1);
    };
    let grants = match ask_or_exit(path, &Command::Grants) {
        Reply::Grants {
            screen,
            camera,
            microphone,
        } => health::Grants {
            screen,
            camera,
            microphone,
        },
        _ => {
            eprintln!("the engine did not answer with its grants");
            std::process::exit(1);
        }
    };
    let said = health::of(&status, &grants);
    match format {
        Format::Json => println!("{}", serde_json::to_string(&said).unwrap_or_default()),
        Format::Prose if said.ok => {
            println!("ok: remuxd {} ({}) can go live", said.version, said.motor)
        }
        Format::Prose => {
            println!("remuxd {} ({})", said.version, said.motor);
            for line in &said.trouble {
                println!("! {line}");
            }
        }
    }
    std::process::exit(if said.ok { 0 } else { 1 });
}

/// `remux destination add`: the shell writes the destinations file itself,
/// so the key goes from stdin (or a file) to a 0600 file and nowhere else.
fn keep_a_destination(platform: &str, name: &str, url: &str, key_from: &cli::KeyFrom) -> ! {
    use remuxd_domain::air::destinations;
    let key = read_key(key_from);
    let path = destinations::path();
    let mut kept = destinations::read(&path);
    match destinations::add(&mut kept, name, platform, url, &key)
        .and_then(|id| destinations::write(&path, &kept).map(|()| id))
    {
        Ok(id) => {
            println!(
                "{id} {name} ({platform}) kept in {}{}",
                path.display(),
                if key.is_empty() { ", no key yet" } else { "" }
            );
            std::process::exit(0);
        }
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(1);
        }
    }
}

/// A key comes from stdin or a file, never argv, which `ps` lists.
fn read_key(key_from: &cli::KeyFrom) -> String {
    match key_from {
        cli::KeyFrom::Stdin => {
            if std::io::stdin().is_terminal() {
                eprint!("key (not echoed): ");
            }
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            line.trim().to_string()
        }
        cli::KeyFrom::File(file) => match std::fs::read_to_string(file) {
            Ok(said) => said.trim().to_string(),
            Err(e) => {
                eprintln!("could not read {file}: {e}");
                std::process::exit(1);
            }
        },
        cli::KeyFrom::Nowhere => String::new(),
    }
}

/// `remux login`: the device code dance, here in the shell. The web shows
/// a page and a code; the person types it there, signed in; this polls
/// until they have and keeps the token in the session file. Nothing
/// crosses the socket; the engine reads the file at its next start.
fn log_in(base: &str) -> ! {
    use remuxd_domain::air::destinations::Key;
    use remuxd_domain::app::{login, session};
    let outcome = (|| -> Result<String, String> {
        let began = remux_wire::post_json(&format!("{base}/api/device"), &serde_json::json!({}))?;
        let started = login::started(began.status, &began.body)?;
        eprintln!(
            "open {} and type the code {}",
            started.verification_url, started.user_code
        );
        let every = std::time::Duration::from_secs(started.interval_secs.max(1));
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(started.expires_in_secs);
        let token = loop {
            if std::time::Instant::now() > deadline {
                return Err("the code expired before it was typed".into());
            }
            std::thread::sleep(every);
            let polled = remux_wire::post_json(
                &format!("{base}/api/token"),
                &serde_json::json!({ "device_code": started.device_code }),
            )?;
            match login::polled(polled.status, &polled.body) {
                login::Polled::Pending => {}
                login::Polled::Token(token) => break token,
                login::Polled::Denied(why) => return Err(why),
            }
        };
        let kept = session::Session {
            base: base.to_string(),
            token: Key(token),
        };
        let me = remux_wire::get_json(&format!("{base}/api/session"), &kept.token.0)?;
        let email = me
            .body
            .get("email")
            .and_then(|e| e.as_str())
            .ok_or_else(|| format!("the web said {} to the new session", me.status))?
            .to_string();
        session::write(&session::path(), &kept)?;
        Ok(email)
    })();
    match outcome {
        Ok(email) => {
            println!("signed in as {email}; restart remuxd to use it");
            std::process::exit(0);
        }
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(1);
        }
    }
}

/// `remux chat url`: where the engine reads its chat from, written to the
/// config by the shell; a running engine is told to take it up at once, a
/// live untouched.
fn keep_a_chat_source(path: &std::path::Path, url: &str) -> ! {
    use remuxd_domain::config;
    let outcome = if url == "-" {
        config::edit(&config::path(), |c| c.chat.url = None)
            .map(|()| "the chat wire is the account's again, or none".to_string())
    } else if url.starts_with("ws://") || url.starts_with("wss://") {
        config::edit(&config::path(), |c| c.chat.url = Some(url.to_string()))
            .map(|()| format!("the chat comes from {url}"))
    } else {
        Err("a chat wire is a ws:// or wss:// address".to_string())
    };
    let outcome = outcome.map(|said| match ask(path, &Command::Rewire) {
        Ok(_) => format!("{said}; the engine is opening it"),
        Err(_) => format!("{said}; the engine reads it when it starts"),
    });
    match outcome {
        Ok(said) => {
            println!("{said}");
            std::process::exit(0);
        }
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(1);
        }
    }
}

/// `remux bug`: the report, gathered from the engine when it answers and
/// from the files when it does not; `--open` hands it to the browser.
fn report_a_bug(path: &std::path::Path, open: Option<&str>) -> ! {
    use remuxd_domain::{bug, health};
    let (engine, health_lines, log) = match ask(path, &Command::Status) {
        Ok(Reply::Status(status)) => {
            let grants = match ask(path, &Command::Grants) {
                Ok(Reply::Grants {
                    screen,
                    camera,
                    microphone,
                }) => health::Grants {
                    screen,
                    camera,
                    microphone,
                },
                _ => health::Grants {
                    screen: remuxd_domain::protocol::Grant::NotAsked,
                    camera: remuxd_domain::protocol::Grant::NotAsked,
                    microphone: remuxd_domain::protocol::Grant::NotAsked,
                },
            };
            let seen = health::of(&status, &grants);
            (
                format!("remuxd {} ({})", status.version, status.motor),
                seen.trouble,
                status.log.clone(),
            )
        }
        Ok(_) => ("the engine answered something else".into(), vec![], vec![]),
        Err(why) => {
            let log = std::fs::read_to_string(remuxd_domain::log::path())
                .map(|text| text.lines().map(String::from).collect())
                .unwrap_or_default();
            (format!("the engine did not answer: {why}"), vec![], log)
        }
    };
    let pieces = bug::Pieces {
        cli_version: env!("CARGO_PKG_VERSION").into(),
        os: remuxd_domain::os::OS.name.into(),
        arch: std::env::consts::ARCH.into(),
        engine,
        health: health_lines,
        config: remuxd_domain::config::describe(),
        log,
    };
    let report = bug::report(&pieces);
    println!("{report}");
    if let Some(title) = open {
        let url = bug::issue_url(&report, &format!("remux: {title}"));
        let (program, args) = remuxd_domain::os::OS.open.split_first().expect("an opener");
        match std::process::Command::new(program)
            .args(args)
            .arg(&url)
            .status()
        {
            Ok(status) if status.success() => {
                println!("\nGitHub's form is open with this report; read it, then submit.")
            }
            _ => println!("\ncould not open a browser; the form is {url}"),
        }
    }
    std::process::exit(0);
}

fn log_out() -> ! {
    use remuxd_domain::app::session;
    if session::forget(&session::path()) {
        println!("signed out; restart remuxd");
    } else {
        println!("not signed in");
    }
    std::process::exit(0);
}

fn forget_a_destination(which: &str) -> ! {
    use remuxd_domain::air::destinations;
    let path = destinations::path();
    let mut kept = destinations::read(&path);
    if !destinations::remove(&mut kept, which) {
        eprintln!("no destination {which}");
        std::process::exit(1);
    }
    if let Err(why) = destinations::write(&path, &kept) {
        eprintln!("{why}");
        std::process::exit(1);
    }
    println!("{which} forgotten");
    std::process::exit(0);
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn ask_or_exit(path: &std::path::Path, command: &Command) -> Reply {
    let json = std::env::args().any(|w| matches!(w.as_str(), "--json" | "-j"));
    ask(path, command).unwrap_or_else(|why| fail(&why, json, 1))
}

/// The app answers a category search on the status, not on the reply: ask
/// until the status carries this query, or give up after five seconds.
fn wait_for_categories(path: &std::path::Path, view: &View) -> Reply {
    let View::Categories { adapter, query } = view else {
        return Reply::Ok;
    };
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Reply::Status(status) = ask_or_exit(path, &Command::Status) {
            if status
                .categories
                .as_ref()
                .is_some_and(|found| found.adapter == *adapter && &found.query == query)
            {
                return Reply::Status(status);
            }
        }
    }
    Reply::Error {
        message: "the app did not answer the search in five seconds".into(),
    }
}

/// `levels -f`: the meters twelve times a second, as the panel drew them;
/// one line rewritten in place on a terminal, one line each for a pipe.
fn follow_the_levels(path: &std::path::Path, format: Format) -> ! {
    let terminal = std::io::stdout().is_terminal() && format == Format::Prose;
    loop {
        let reply = ask_or_exit(path, &Command::Levels);
        let text = cli::show(&reply, &View::Reply, format, Ink::Plain, now());
        if terminal {
            print!("\r\x1b[2K{text}");
            let _ = std::io::stdout().flush();
        } else {
            println!("{text}");
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
}

/// `log -f`: the journal so far, then every new line once a second.
fn follow_the_log(path: &std::path::Path, format: Format) -> ! {
    let mut seen: Vec<String> = Vec::new();
    loop {
        if let Reply::Status(status) = ask_or_exit(path, &Command::Status) {
            let fresh = new_lines(&seen, &status.log);
            for line in &fresh {
                match format {
                    Format::Json => println!("{}", cli::json_line(line)),
                    Format::Prose => println!("{line}"),
                }
            }
            seen = status.log.clone();
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// The lines of `now` that come after the last line already seen; all of
/// them when the last seen line has scrolled out of the journal.
fn new_lines(seen: &[String], now: &[String]) -> Vec<String> {
    let Some(last) = seen.last() else {
        return now.to_vec();
    };
    match now.iter().rposition(|line| line == last) {
        Some(at) => now[at + 1..].to_vec(),
        None => now.to_vec(),
    }
}

/// One question to the engine, one answer.
fn ask(path: &std::path::Path, command: &Command) -> Result<Reply, String> {
    let stream = UnixStream::connect(path).map_err(|_| {
        format!(
            "no engine is listening on {}. Start one with `remuxd`.",
            path.display()
        )
    })?;
    let mut out = stream
        .try_clone()
        .map_err(|e| format!("the socket would not answer: {e}"))?;
    out.write_all(encode(command).as_bytes())
        .map_err(|_| "the engine went away while being asked".to_string())?;
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() || line.is_empty() {
        return Err("the engine went away without answering".into());
    }
    decode_reply(&line)
        .map_err(|why| format!("the engine said something this does not understand: {why}"))
}

/// `events -f`: what the engine still holds, then every change as the engine
/// pushes it, on one connection, until the shell says stop. Never returns.
fn follow_the_events(path: &std::path::Path, format: Format) -> ! {
    let stream = UnixStream::connect(path).unwrap_or_else(|_| {
        eprintln!(
            "no engine is listening on {}. Start one with `remuxd`.",
            path.display()
        );
        std::process::exit(1);
    });
    let mut out = stream.try_clone().unwrap_or_else(|e| {
        eprintln!("the socket would not answer: {e}");
        std::process::exit(1);
    });
    let asked = Command::Events {
        since: 0,
        follow: true,
    };
    if out.write_all(encode(&asked).as_bytes()).is_err() {
        eprintln!("the engine went away while being asked");
        std::process::exit(1);
    }
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        match decode_reply(&line) {
            Ok(reply) if format == Format::Json => {
                for line in cli::event_lines(&reply) {
                    println!("{line}");
                }
            }
            Ok(reply) => println!(
                "{}",
                cli::show(&reply, &View::Reply, format, Ink::Plain, now())
            ),
            Err(why) => {
                eprintln!("the engine said something this does not understand: {why}");
                std::process::exit(1);
            }
        }
    }
    eprintln!("the engine went away");
    std::process::exit(1);
}

/// `chat -f`: what has been said, then every line as the engine pushes it,
/// on one connection, until the shell says stop. Never returns.
fn follow_the_chat(path: &std::path::Path, format: Format, ink: Ink) -> ! {
    let stream = UnixStream::connect(path).unwrap_or_else(|_| {
        eprintln!(
            "no engine is listening on {}. Start one with `remuxd`.",
            path.display()
        );
        std::process::exit(1);
    });
    let mut out = stream.try_clone().unwrap_or_else(|e| {
        eprintln!("the socket would not answer: {e}");
        std::process::exit(1);
    });
    let asked = Command::Chat {
        since: 0,
        follow: true,
    };
    if out.write_all(encode(&asked).as_bytes()).is_err() {
        eprintln!("the engine went away while being asked");
        std::process::exit(1);
    }
    let mut shown = false;
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        match decode_reply(&line) {
            Ok(Reply::Chat { reachable, lines }) if lines.is_empty() => {
                if !reachable {
                    println!("{}", cli::render(&Reply::Chat { reachable, lines }));
                }
            }
            Ok(reply @ Reply::Chat { .. }) => {
                let text = match format {
                    Format::Json => cli::show(&reply, &View::Reply, format, ink, now()),
                    Format::Prose if shown => cli::render_more(&reply, ink),
                    Format::Prose => cli::render_with(&reply, ink),
                };
                println!("{text}");
                shown = true;
            }
            Ok(other) => println!("{}", cli::render(&other)),
            Err(why) => {
                eprintln!("the engine said something this does not understand: {why}");
                std::process::exit(1);
            }
        }
    }
    eprintln!("the engine went away");
    std::process::exit(1);
}
