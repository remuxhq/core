//! `remux tui`: the engine on one screen, in the terminal. What it shows is decided here,
//! in plain functions of the status and the time, and only drawn by the terminal part.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use remuxd_domain::air::plan::Plan;
use remuxd_domain::protocol::{Command, Reply, Status};

/// The screen, until `q`: the status read once a second through `ask`, drawn after every
/// read and every key. Only a question a person answered with `y` sends anything else.
pub fn run(ask: impl Fn(&Command) -> Result<Reply, String>) -> std::io::Result<()> {
    let read = || match ask(&Command::Status)? {
        Reply::Status(status) => Ok(*status),
        other => Err(format!("the engine answered {other:?}")),
    };
    let mut terminal = ratatui::init();
    let mut status = read();
    let mut read_at = Instant::now();
    let mut screen = Screen::default();
    let mut said: Option<String> = None;
    let outcome = loop {
        if read_at.elapsed() >= Duration::from_secs(1) {
            status = read();
            read_at = Instant::now();
        }
        let rows = status.as_ref().map_or(0, |s| s.scenes.len());
        let mut list =
            ListState::default().with_selected(Some(screen.picked.min(rows.saturating_sub(1))));
        if let Err(why) =
            terminal.draw(|frame| draw(frame, &status, &screen, said.as_deref(), &mut list, now()))
        {
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
            KeyCode::Enter => '\n',
            KeyCode::Down => 'j',
            KeyCode::Up => 'k',
            _ => continue,
        };
        let Ok(now_status) = &status else {
            if typed == 'q' {
                break Ok(());
            }
            continue;
        };
        match press(typed, &mut screen, rows, now_status) {
            Act::Stay => {}
            Act::Quit => break Ok(()),
            Act::Plan => match ask(&Command::Plan) {
                Ok(Reply::Plan(plan)) => screen.planned(plan),
                Ok(other) => said = Some(format!("the engine answered {other:?}")),
                Err(why) => said = Some(why),
            },
            Act::Send(command) => {
                said = Some(match ask(&command) {
                    Ok(Reply::Error { message }) => message,
                    Ok(_) => "done".into(),
                    Err(why) => why,
                });
                status = read();
                read_at = Instant::now();
            }
        }
    };
    ratatui::restore();
    outcome
}

fn draw(
    frame: &mut Frame,
    status: &Result<Status, String>,
    screen: &Screen,
    said: Option<&str>,
    list: &mut ListState,
    now: i64,
) {
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
    let keys = "q quit · j/k move · enter switch · L live · S stop · R record · ! cut";
    let footer = said.map_or(keys.to_string(), |said| format!("{said} · {keys}"));
    frame.render_widget(
        Paragraph::new(footer).style(Style::new().fg(Color::DarkGray)),
        bottom,
    );
    if let Some(lever) = &screen.asking {
        ask_about(frame, lever);
    }
}

/// The question a lever puts, over the middle of the screen.
fn ask_about(frame: &mut Frame, lever: &Lever) {
    let (title, text, colour) = match lever {
        Lever::Live(plan) if plan.blockers.is_empty() => (
            " go live? ",
            format!(
                "{}\n\ny: go live on this plan · any other key: no",
                crate::words::render_plan(plan)
            ),
            Color::Red,
        ),
        Lever::Live(plan) => (
            " cannot go live ",
            format!("{}\n\nany key: close", crate::words::render_plan(plan)),
            Color::Yellow,
        ),
        Lever::Stop => (
            " stop the live? ",
            "y: stop · any other key: no".to_string(),
            Color::Red,
        ),
        Lever::Record { on: true } => (
            " start recording? ",
            "y: record · any other key: no".to_string(),
            Color::Yellow,
        ),
        Lever::Record { on: false } => (
            " stop recording? ",
            "y: stop · any other key: no".to_string(),
            Color::Yellow,
        ),
        Lever::Cut => (
            " cut? ",
            "removes every layer of the active scene and every audio layer, for good:\n\
             the scene comes back empty and has to be built again\n\ny: cut · any other key: no"
                .to_string(),
            Color::Red,
        ),
    };
    let area = frame.area();
    let [_, middle, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Percentage(70),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, popup, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(80),
        Constraint::Fill(1),
    ])
    .areas(middle);
    frame.render_widget(ratatui::widgets::Clear, popup);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(
                Block::bordered()
                    .title(title)
                    .border_style(Style::new().fg(colour)),
            ),
        popup,
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

/// A lever that changes what goes out, held open until a person says `y`.
#[derive(Debug, Clone, PartialEq)]
pub enum Lever {
    /// Going live on this plan; it cannot be confirmed while anything blocks it.
    Live(Plan),
    Stop,
    Record {
        on: bool,
    },
    Cut,
}

impl Lever {
    fn command(&self) -> Option<Command> {
        match self {
            Lever::Live(plan) if plan.blockers.is_empty() => Some(Command::Live {
                plan: plan.fingerprint,
            }),
            Lever::Live(_) => None,
            Lever::Stop => Some(Command::Stop),
            Lever::Record { on: true } => Some(Command::RecordStart),
            Lever::Record { on: false } => Some(Command::RecordStop),
            Lever::Cut => Some(Command::HideEverything),
        }
    }
}

/// What the screen holds between keys: the picked row, and the question open, if any.
#[derive(Debug, Default)]
pub struct Screen {
    pub picked: usize,
    pub asking: Option<Lever>,
}

impl Screen {
    /// The plan the engine answered to `L`, put to the person.
    pub fn planned(&mut self, plan: Plan) {
        self.asking = Some(Lever::Live(plan));
    }
}

/// What a key asks of the loop around the screen.
#[derive(Debug, PartialEq)]
pub enum Act {
    Stay,
    Quit,
    /// Ask the engine what going live would do.
    Plan,
    /// Send this, which a person confirmed.
    Send(Command),
}

/// A key. `j`/`k` move the pick without leaving the list, Enter switches to it, `q` quits.
/// `L`, `S`, `R` and `!`
/// only open a question; `y` answers it and sends, and any other key lets it go, `q`
/// included: under a question, `q` never closes the screen.
pub fn press(key: char, screen: &mut Screen, rows: usize, status: &Status) -> Act {
    if let Some(lever) = screen.asking.take() {
        return match (key, lever.command()) {
            ('y', Some(command)) => Act::Send(command),
            // A plan something blocks stays up on `y`: the blockers are what it says.
            ('y', None) => {
                screen.asking = Some(lever);
                Act::Stay
            }
            _ => Act::Stay,
        };
    }
    match key {
        'q' => return Act::Quit,
        'j' if screen.picked + 1 < rows => screen.picked += 1,
        'k' => screen.picked = screen.picked.saturating_sub(1),
        // Switching is the live's everyday gesture: at once, no question.
        '\n' => {
            if let Some(scene) = status.scenes.get(screen.picked) {
                if scene.name != status.active_scene {
                    return Act::Send(Command::SceneSwitch {
                        name: scene.name.clone(),
                    });
                }
            }
        }
        'L' if !status.on_air => return Act::Plan,
        'S' if status.on_air => screen.asking = Some(Lever::Stop),
        'R' => {
            screen.asking = Some(Lever::Record {
                on: !status.recording,
            })
        }
        '!' => screen.asking = Some(Lever::Cut),
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
    use remuxd_domain::air::plan::Plan;
    use remuxd_domain::protocol::{Command, Status};

    fn on_air() -> Status {
        Status {
            on_air: true,
            ..Status::default()
        }
    }

    fn plan(fingerprint: u64, blockers: &[&str]) -> Plan {
        let mut plan = Plan::of(&Status::default());
        plan.fingerprint = fingerprint;
        plan.blockers = blockers.iter().map(|b| b.to_string()).collect();
        plan
    }

    #[test]
    fn q_quits_and_j_k_move_without_leaving_the_list() {
        let mut screen = Screen::default();
        let off = Status::default();
        assert_eq!(press('j', &mut screen, 3, &off), Act::Stay);
        assert_eq!(screen.picked, 1);
        press('j', &mut screen, 3, &off);
        press('j', &mut screen, 3, &off);
        assert_eq!(screen.picked, 2, "the last row is as far as it goes");
        for _ in 0..3 {
            press('k', &mut screen, 3, &off);
        }
        assert_eq!(screen.picked, 0, "and the first is as far up");
        assert_eq!(press('q', &mut screen, 3, &off), Act::Quit);
    }

    fn with_scenes(names: &[&str], active: &str) -> Status {
        Status {
            scenes: names
                .iter()
                .map(|n| {
                    serde_json::from_value(serde_json::json!({ "name": n }))
                        .expect("a scene by its name")
                })
                .collect(),
            active_scene: active.to_string(),
            ..Status::default()
        }
    }

    #[test]
    fn enter_switches_to_the_picked_scene_at_once() {
        let status = with_scenes(&["Screen", "Starting soon", "BRB"], "Screen");
        let mut screen = Screen {
            picked: 2,
            ..Screen::default()
        };
        assert_eq!(
            press('\n', &mut screen, 3, &status),
            Act::Send(Command::SceneSwitch { name: "BRB".into() })
        );
    }

    #[test]
    fn enter_on_the_scene_already_out_does_nothing() {
        let status = with_scenes(&["Screen", "BRB"], "Screen");
        let mut screen = Screen::default();
        assert_eq!(press('\n', &mut screen, 2, &status), Act::Stay);
    }

    #[test]
    fn s_asks_before_stopping_and_only_y_stops() {
        let mut screen = Screen::default();
        assert_eq!(press('S', &mut screen, 0, &on_air()), Act::Stay);
        assert_eq!(screen.asking, Some(Lever::Stop));
        assert_eq!(
            press('y', &mut screen, 0, &on_air()),
            Act::Send(Command::Stop)
        );
        assert_eq!(screen.asking, None);
    }

    #[test]
    fn any_key_but_y_lets_the_question_go_and_q_does_not_quit_under_one() {
        let mut screen = Screen::default();
        press('S', &mut screen, 0, &on_air());
        assert_eq!(press('n', &mut screen, 0, &on_air()), Act::Stay);
        assert_eq!(screen.asking, None);
        press('S', &mut screen, 0, &on_air());
        assert_eq!(
            press('q', &mut screen, 0, &on_air()),
            Act::Stay,
            "q closes the question, not the screen"
        );
        assert_eq!(screen.asking, None);
    }

    #[test]
    fn l_asks_the_engine_for_the_plan_and_y_goes_live_on_that_plan() {
        let mut screen = Screen::default();
        let off = Status::default();
        assert_eq!(press('L', &mut screen, 0, &off), Act::Plan);
        screen.planned(plan(42, &[]));
        assert!(matches!(screen.asking, Some(Lever::Live(_))));
        assert_eq!(
            press('y', &mut screen, 0, &off),
            Act::Send(Command::Live { plan: 42 })
        );
    }

    #[test]
    fn a_plan_that_something_blocks_cannot_be_confirmed() {
        let mut screen = Screen::default();
        let off = Status::default();
        press('L', &mut screen, 0, &off);
        screen.planned(plan(42, &["no destination is armed"]));
        assert_eq!(press('y', &mut screen, 0, &off), Act::Stay);
        assert!(
            matches!(screen.asking, Some(Lever::Live(_))),
            "the blockers stay on the screen"
        );
        press('n', &mut screen, 0, &off);
        assert_eq!(screen.asking, None);
    }

    #[test]
    fn going_live_is_not_offered_on_the_air_nor_stopping_off_it() {
        let mut screen = Screen::default();
        assert_eq!(press('L', &mut screen, 0, &on_air()), Act::Stay);
        assert_eq!(press('S', &mut screen, 0, &Status::default()), Act::Stay);
        assert_eq!(screen.asking, None);
    }

    #[test]
    fn r_starts_or_stops_the_recording_after_a_yes() {
        let mut screen = Screen::default();
        let off = Status::default();
        press('R', &mut screen, 0, &off);
        assert_eq!(
            press('y', &mut screen, 0, &off),
            Act::Send(Command::RecordStart)
        );
        let recording = Status {
            recording: true,
            ..Status::default()
        };
        press('R', &mut screen, 0, &recording);
        assert_eq!(
            press('y', &mut screen, 0, &recording),
            Act::Send(Command::RecordStop)
        );
    }

    #[test]
    fn bang_is_the_panic_button_after_a_yes() {
        let mut screen = Screen::default();
        press('!', &mut screen, 0, &on_air());
        assert_eq!(screen.asking, Some(Lever::Cut));
        assert_eq!(
            press('y', &mut screen, 0, &on_air()),
            Act::Send(Command::HideEverything)
        );
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
