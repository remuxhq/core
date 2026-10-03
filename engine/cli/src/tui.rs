//! `remux tui`: the engine on one screen, in the terminal. What it shows is decided here,
//! in plain functions of the status and the time, and only drawn by the terminal part.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use remuxd_domain::protocol::Status;

/// The screen, until `q`: the status read once a second through `read`, drawn after
/// every read and every key. It sends nothing that changes the engine.
pub fn run(read: impl Fn() -> Result<Status, String>) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let mut status = read();
    let mut read_at = Instant::now();
    let mut picked = 0;
    let outcome = loop {
        if read_at.elapsed() >= Duration::from_secs(1) {
            status = read();
            read_at = Instant::now();
        }
        let rows = status.as_ref().map_or(0, |s| s.scenes.len());
        let mut list = ListState::default().with_selected(Some(picked.min(rows.saturating_sub(1))));
        if let Err(why) = terminal.draw(|frame| draw(frame, &status, &mut list, now())) {
            break Err(why);
        }
        match event::poll(Duration::from_millis(250)) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(why) => break Err(why),
        }
        let key = match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => key.code,
            Ok(_) => continue,
            Err(why) => break Err(why),
        };
        let typed = match key {
            KeyCode::Char(c) => c,
            KeyCode::Esc => 'q',
            KeyCode::Down => 'j',
            KeyCode::Up => 'k',
            _ => continue,
        };
        if press(typed, &mut picked, rows) == Act::Quit {
            break Ok(());
        }
    };
    ratatui::restore();
    outcome
}

fn draw(frame: &mut Frame, status: &Result<Status, String>, list: &mut ListState, now: i64) {
    let [top, middle, bottom] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let Ok(status) = status else {
        frame.render_widget(
            Paragraph::new("no engine at the socket: remux daemon start")
                .style(Style::new().fg(Color::Red)),
            top,
        );
        return;
    };
    let colour = if status.on_air {
        Color::Red
    } else {
        Color::DarkGray
    };
    let line = format!("{} · scene: {}", air(status, now), status.active_scene);
    frame.render_widget(
        Paragraph::new(line)
            .style(Style::new().fg(colour).add_modifier(Modifier::BOLD))
            .block(Block::bordered().title(" remux ")),
        top,
    );
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(middle);
    let scenes: Vec<ListItem> = status
        .scenes
        .iter()
        .map(|scene| {
            let mark = if scene.name == status.active_scene {
                "▶ "
            } else {
                "  "
            };
            ListItem::new(format!("{mark}{}", scene.name))
        })
        .collect();
    frame.render_stateful_widget(
        List::new(scenes)
            .block(Block::bordered().title(" scenes "))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        left,
        list,
    );
    let destinations: Vec<ListItem> = status
        .destinations
        .iter()
        .map(|d| {
            let lamp = if d.status == "live" {
                "●"
            } else if d.armed {
                "○"
            } else {
                "·"
            };
            let armed = if d.armed { "armed" } else { "off" };
            ListItem::new(format!(
                "{lamp} {} ({}) {armed} {}",
                d.name, d.platform, d.status
            ))
        })
        .collect();
    frame.render_widget(
        List::new(destinations).block(Block::bordered().title(" destinations ")),
        right,
    );
    frame.render_widget(
        Paragraph::new("q quit · j/k move · reads only").style(Style::new().fg(Color::DarkGray)),
        bottom,
    );
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The line that says whether anybody can see you, and for how long: the live and the
/// recording, each counted from the engine's own clock, so it is right when the screen
/// opens in the middle of a live.
pub fn air(status: &Status, now: i64) -> String {
    let mut line = if status.on_air {
        format!("● ON AIR {}", clock(status.on_air_since, now))
    } else {
        "○ off air".to_string()
    };
    if status.recording {
        line.push_str(&format!(" · ● REC {}", clock(status.recording_since, now)));
    }
    line
}

/// What a key asks of the screen.
#[derive(Debug, PartialEq, Eq)]
pub enum Act {
    Stay,
    Quit,
}

/// A key on a list of `rows`: `j` and `k` move what is picked without leaving the list,
/// `q` quits. Nothing here talks to the engine: this screen only reads.
pub fn press(key: char, picked: &mut usize, rows: usize) -> Act {
    match key {
        'q' => return Act::Quit,
        'j' if *picked + 1 < rows => *picked += 1,
        'k' => *picked = picked.saturating_sub(1),
        _ => {}
    }
    Act::Stay
}

/// Seconds since `since`, as HH:MM:SS; zero when the engine has not said.
fn clock(since: Option<i64>, now: i64) -> String {
    let seconds = since.map_or(0, |since| (now - since).max(0));
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use remuxd_domain::protocol::Status;

    #[test]
    fn q_quits_and_j_k_move_without_leaving_the_list() {
        let mut picked = 0;
        assert_eq!(press('j', &mut picked, 3), Act::Stay);
        assert_eq!(picked, 1);
        press('j', &mut picked, 3);
        press('j', &mut picked, 3);
        assert_eq!(picked, 2, "the last row is as far as it goes");
        press('k', &mut picked, 3);
        press('k', &mut picked, 3);
        press('k', &mut picked, 3);
        assert_eq!(picked, 0, "and the first is as far up");
        assert_eq!(press('q', &mut picked, 3), Act::Quit);
    }

    #[test]
    fn off_the_air_says_so_with_no_clock() {
        assert_eq!(air(&Status::default(), 1_000), "○ off air");
    }

    #[test]
    fn on_the_air_counts_from_when_the_engine_says_it_started() {
        let status = Status {
            on_air: true,
            on_air_since: Some(1_000 - 65),
            ..Status::default()
        };
        assert_eq!(air(&status, 1_000), "● ON AIR 00:01:05");
    }

    #[test]
    fn a_recording_has_its_own_clock_beside_the_air() {
        let status = Status {
            recording: true,
            recording_since: Some(1_000 - 3_610),
            ..Status::default()
        };
        assert_eq!(air(&status, 1_000), "○ off air · ● REC 01:00:10");
    }
}
