//! `remux bug`: what a bug report needs, gathered once and written so a
//! person can paste it, or open it as a GitHub issue prefilled. Nothing in
//! it is a secret: a key prints stars already, and anything that looks like
//! a token in a URL or a log line is redacted here before it leaves.

/// The pieces the shell gathers; the report is pure.
pub struct Pieces {
    pub cli_version: String,
    pub os: String,
    pub arch: String,
    /// `remuxd 0.1.0 (obs 30.2.3)`, or what stood in the way of asking.
    pub engine: String,
    pub health: Vec<String>,
    pub config: String,
    pub log: Vec<String>,
}

pub const REPO: &str = "remuxhq/core";

/// Anything after `token=`, `key=`, `secret=`, `password=` up to the next
/// `&`, space or quote, and the query of a ws/wss URL, become `…`.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let lower = rest.to_ascii_lowercase();
        let hit = ["token=", "key=", "secret=", "password=", "authorization: "]
            .iter()
            .filter_map(|needle| lower.find(needle).map(|at| (at, needle.len())))
            .min();
        let Some((at, len)) = hit else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..at + len]);
        out.push('…');
        let after = &rest[at + len..];
        // A header's value runs to the end of the line; a parameter's to
        // the next separator.
        let end = if lower[at..].starts_with("authorization: ") {
            after.find('\n').unwrap_or(after.len())
        } else {
            after
                .find(|c: char| c == '&' || c.is_whitespace() || c == '"' || c == '\'' || c == ',')
                .unwrap_or(after.len())
        };
        rest = &after[end..];
    }
    // A chat wire's URL may carry a token in any name: the whole query goes.
    let mut cleaned = String::with_capacity(out.len());
    for word in out.split_inclusive(char::is_whitespace) {
        if (word.starts_with("ws://") || word.starts_with("wss://")) && word.contains('?') {
            let (base, tail) = word.split_once('?').unwrap_or((word, ""));
            cleaned.push_str(base);
            cleaned.push_str("?…");
            cleaned.push_str(tail.trim_start_matches(|c: char| !c.is_whitespace()));
        } else {
            cleaned.push_str(word);
        }
    }
    cleaned
}

/// The report, Markdown, short enough to ride in a URL.
pub fn report(pieces: &Pieces) -> String {
    let health = if pieces.health.is_empty() {
        "  ok".to_string()
    } else {
        pieces
            .health
            .iter()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let log = pieces
        .log
        .iter()
        .rev()
        .take(25)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| format!("  {}", redact(line)))
        .collect::<Vec<_>>()
        .join("\n");
    redact(&format!(
        "remux {cli} · {engine}\n{os} {arch}\n\nhealth\n{health}\n\nconfig\n{config}\n\nlog (last lines)\n{log}\n",
        cli = pieces.cli_version,
        engine = pieces.engine,
        os = pieces.os,
        arch = pieces.arch,
        config = pieces
            .config
            .lines()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The GitHub issue form, prefilled: a person reads it and presses submit.
/// Browsers and GitHub cap a URL around 8 KB, so the log is dropped first
/// when the report is long; it is still on the terminal to paste.
pub fn issue_url(report: &str, title: &str) -> String {
    let base = format!(
        "https://github.com/{REPO}/issues/new?template=bug.yml&title={}&report=",
        encode(title)
    );
    let full = format!("{base}{}", encode(report));
    if full.len() <= 7_000 {
        return full;
    }
    let short = report.split("\nlog (last lines)").next().unwrap_or(report);
    format!(
        "{base}{}",
        encode(&format!(
            "{short}\nlog: too long for a link, pasted below\n"
        ))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_a_key_and_a_wire_query_never_leave() {
        assert_eq!(
            redact("wire: ws://h/ws?token=abc&vsn=1"),
            "wire: ws://h/ws?…"
        );
        assert_eq!(
            redact("url=rtmp://x/app key=live_123 done"),
            "url=rtmp://x/app key=… done"
        );
        assert_eq!(
            redact("Authorization: Bearer x.y\nnext"),
            "Authorization: …\nnext"
        );
        assert_eq!(redact("nothing here"), "nothing here");
    }

    #[test]
    fn the_report_carries_the_versions_the_health_the_config_and_the_last_lines_redacted() {
        let pieces = Pieces {
            cli_version: "0.1.0".into(),
            os: "linux".into(),
            arch: "aarch64".into(),
            engine: "remuxd 0.1.0 (obs 30.2.3)".into(),
            health: vec!["no picture: remux screen <id>".into()],
            config: "chat.url   ws://127.0.0.1:9999?token=t  (config)".into(),
            log: (0..40).map(|n| format!("line {n} key=k{n}")).collect(),
        };
        let said = report(&pieces);
        assert!(said.starts_with("remux 0.1.0 · remuxd 0.1.0 (obs 30.2.3)\nlinux aarch64\n"));
        assert!(said.contains("  no picture: remux screen <id>"));
        assert!(said.contains("ws://127.0.0.1:9999?…"));
        assert!(
            said.contains("line 39 key=…") && !said.contains("line 14 "),
            "the last 25 lines, redacted"
        );
        assert!(!said.contains("token=t") && !said.contains("k39"));
    }

    #[test]
    fn the_issue_url_is_the_form_prefilled_and_drops_the_log_when_too_long() {
        let url = issue_url("remux 0.1.0\nhealth\n  ok\n", "remux: it broke");
        assert!(url.starts_with("https://github.com/remuxhq/core/issues/new?template=bug.yml&title=remux%3A%20it%20broke&report="));
        assert!(url.contains("remux%200.1.0%0Ahealth"));
        let long = format!("head\nlog (last lines)\n{}", "x".repeat(9_000));
        let url = issue_url(&long, "t");
        assert!(url.len() < 7_000 && url.contains("too%20long"));
    }
}
