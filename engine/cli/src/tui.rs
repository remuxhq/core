//! `remux tui`: the engine on one screen, in the terminal. What it shows is decided here,
//! in plain functions of the status and the time, and only drawn by the terminal part.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use remuxd_domain::air::plan::Plan;
use remuxd_domain::companions::State;
use remuxd_domain::picture::scenes::Scene;
use remuxd_domain::protocol::{ChatLine, Command, Destination, Hearing, Mixing, Reply, Status};

/// The screen, until `q`: the status read once a second through `ask`, drawn after every
/// read and every key. Only a question a person answered with `y` sends anything else.
pub fn run(ask: impl Fn(&Command) -> Result<Reply, String>) -> std::io::Result<()> {
    let read = || match ask(&Command::Status)? {
        Reply::Status(status) => Ok(*status),
        other => Err(format!("the engine answered {other:?}")),
    };
    let mut terminal = ratatui::init();
    // A click focuses a panel and picks its row.
    let _ = ratatui::crossterm::execute!(std::io::stdout(), event::EnableMouseCapture);
    let mut placed: Vec<(Panel, ratatui::layout::Rect)> = Vec::new();
    let mut status = read();
    let mut read_at = Instant::now();
    let mut screen = Screen {
        hidden: read_hidden(&std::fs::read_to_string(tui_toml()).unwrap_or_default()),
        ..Screen::default()
    };
    if !screen.shows(screen.focus) {
        screen.focus = screen.next_shown(screen.focus);
    }
    let mut last_said: Option<String> = None;
    // The log starts with what the engine wrote down so far; of its events, only
    // what happens from now on.
    let mut journal_seen: Option<String> = None;
    if let Ok(now) = &status {
        for line in &now.log {
            to_log(&mut screen, line.clone());
        }
        journal_seen = now.log.last().cloned();
    }
    let mut events_seen = match ask(&Command::Events {
        since: 0,
        follow: false,
    }) {
        Ok(Reply::Events { events, .. }) => events.iter().map(|e| e.seq).max().unwrap_or(0),
        _ => 0,
    };
    let mut said_at = Instant::now();
    // The genres once: they never open a device to be listed.
    if let Ok(Reply::Sources(devices)) = ask(&Command::Genres) {
        screen.genres = devices.genres;
    }
    let mut chat = Chat::default();
    let fresh = chat.read(&ask);
    chat.hold(&mut screen, fresh);
    // What a companion's start or stop came to, from the thread that ran it.
    let (told, heard) = std::sync::mpsc::channel::<String>();
    look_at_companions(&mut screen);
    let mut meters = Meters::default();
    let mut heard_at = Instant::now();
    let outcome = loop {
        if read_at.elapsed() >= Duration::from_secs(1) {
            status = read();
            if let Ok(now) = &status {
                for line in fresh_journal(journal_seen.as_deref(), &now.log) {
                    to_log(&mut screen, line.clone());
                }
                journal_seen = now.log.last().cloned();
            }
            if let Ok(Reply::Events { events, .. }) = ask(&Command::Events {
                since: events_seen,
                follow: false,
            }) {
                for line in events_for_log(&events) {
                    to_log(&mut screen, line);
                }
                events_seen = events.iter().map(|e| e.seq).max().unwrap_or(events_seen);
            }
            let fresh = chat.read(&ask);
            chat.hold(&mut screen, fresh);
            look_at_companions(&mut screen);
            screen.filters = filters_in(&remuxd_domain::config::shaders_dir());
            read_at = Instant::now();
        }
        if let Ok(said) = heard.try_recv() {
            to_log(&mut screen, format!("{} {said}", local_time()));
            screen.said = Some(said);
            look_at_companions(&mut screen);
        }
        // The meters, every turn of the loop: one line on the socket, no device opened.
        let levels = match ask(&Command::Levels) {
            Ok(Reply::Levels { hearing, mixing }) => Some((hearing, mixing)),
            _ => None,
        };
        if let (Some((hearing, mixing)), Ok(now_status)) = (&levels, &status) {
            meters.follow(
                hearing,
                mixing,
                now_status.muted,
                heard_at.elapsed().as_secs_f64(),
            );
            screen.playing = mixing.playing;
        }
        heard_at = Instant::now();
        // An answer shows for a few seconds after it changes.
        if screen.said != last_said {
            last_said.clone_from(&screen.said);
            said_at = Instant::now();
        }
        screen.said_fresh = screen.said.is_some() && said_at.elapsed() < Duration::from_secs(4);
        let rows = status.as_ref().map_or(0, |s| s.scenes.len());
        if let Err(why) = terminal.draw(|frame| {
            placed = draw(
                frame,
                &status,
                levels.as_ref(),
                &meters,
                &screen,
                &chat,
                now(),
            );
        }) {
            break Err(why);
        }
        match event::poll(Duration::from_millis(50)) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(why) => break Err(why),
        }
        let key = match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => key.code,
            Ok(Event::Mouse(mouse)) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let open = screen.picker.is_some()
                    || screen.asking.is_some()
                    || screen.showing_keys
                    || screen.typing.is_some();
                if let (false, Some((panel, row)), Ok(now_status)) =
                    (open, panel_at(&placed, mouse.column, mouse.row), &status)
                {
                    clicked(&mut screen, now_status, panel, row);
                }
                continue;
            }
            Ok(_) => continue,
            Err(why) => break Err(why),
        };
        let typed = match key {
            // `=` is `+` without the shift.
            KeyCode::Char('=') => '+',
            KeyCode::Char(c) => c,
            KeyCode::Esc => '\u{1b}',
            KeyCode::Backspace => '\u{8}',
            KeyCode::Enter => '\n',
            KeyCode::Tab => '\t',
            KeyCode::BackTab => '\u{19}',
            KeyCode::Down => 'j',
            KeyCode::Up => 'k',
            _ => continue,
        };
        let Ok(now_status) = &status else {
            if matches!(typed, 'q' | '\u{1b}') {
                break Ok(());
            }
            continue;
        };
        let hidden = screen.hidden.clone();
        let act = press(typed, &mut screen, rows, now_status);
        chat.hold(&mut screen, Vec::new());
        // The panels shown are the person's: kept for the next time.
        if screen.hidden != hidden {
            let _ = std::fs::write(tui_toml(), keep_hidden(&screen.hidden));
        }
        match act {
            Act::Stay => {}
            Act::Quit => break Ok(()),
            Act::Say(why) => screen.said = Some(why.into()),
            Act::AskMics => match ask(&Command::Mics) {
                Ok(Reply::Sources(devices)) => screen.picker = Some(mic_picker(&devices)),
                Ok(Reply::Error { message }) => screen.said = Some(message),
                Ok(other) => screen.said = Some(format!("the engine answered {other:?}")),
                Err(why) => screen.said = Some(why),
            },
            Act::CompanionSend { name, words } => {
                let said = std::env::current_exe()
                    .and_then(|me| {
                        std::process::Command::new(me)
                            .args(["companion", "send", &name, &words])
                            .output()
                    })
                    .map(|out| {
                        let text = if out.status.success() {
                            out.stdout
                        } else {
                            out.stderr
                        };
                        String::from_utf8_lossy(&text).trim().to_string()
                    })
                    .unwrap_or_else(|why| why.to_string());
                to_log(
                    &mut screen,
                    format!("{} {}", local_time(), crate::words::plain(&said)),
                );
                screen.said = Some(crate::words::plain(&said));
            }
            Act::Companion { name, start } => {
                screen.said = Some(format!(
                    "{} {name}…",
                    if start { "starting" } else { "stopping" }
                ));
                let told = told.clone();
                // By this shell's own `remux companion`, which leaves the
                // companion to the system when it exits: a companion that
                // dies is then gone, not a zombie of this screen's.
                std::thread::spawn(move || {
                    let verb = if start { "start" } else { "stop" };
                    let said = std::env::current_exe()
                        .and_then(|me| {
                            std::process::Command::new(me)
                                .args(["companion", verb, &name])
                                .output()
                        })
                        .map(|out| {
                            let text = if out.status.success() {
                                out.stdout
                            } else {
                                out.stderr
                            };
                            String::from_utf8_lossy(&text).trim().to_string()
                        })
                        .unwrap_or_else(|why| why.to_string());
                    let _ = told.send(crate::words::plain(&said));
                });
            }
            Act::Plan => match ask(&Command::Plan) {
                Ok(Reply::Plan(plan)) => screen.planned(plan),
                Ok(other) => screen.said = Some(format!("the engine answered {other:?}")),
                Err(why) => screen.said = Some(why),
            },
            Act::Send(command) => {
                // Arming during a live changes the next one: the engine picks its
                // destinations when a live starts.
                let later = now_status.on_air && matches!(command, Command::Arm { .. });
                let answered = ask(&command);
                // What the engine did is in its journal; what never reached it is not.
                if let Err(why) = &answered {
                    to_log(&mut screen, format!("{} ! {why}", local_time()));
                }
                screen.said = Some(match answered {
                    Ok(Reply::Error { message }) => message,
                    Ok(_) if later => "done: from the next live on".into(),
                    // The engine keeps no copy of a said line: the platform hands it back.
                    Ok(_) if matches!(command, Command::Say { .. }) => {
                        "sent up the wire: it shows when the chat hands it back".into()
                    }
                    Ok(_) => "done".into(),
                    Err(why) => why,
                });
                status = read();
                read_at = Instant::now();
            }
        }
    };
    let _ = ratatui::crossterm::execute!(std::io::stdout(), event::DisableMouseCapture);
    ratatui::restore();
    outcome
}

fn draw(
    frame: &mut Frame,
    status: &Result<Status, String>,
    levels: Option<&(Hearing, Mixing)>,
    meters: &Meters,
    screen: &Screen,
    chat: &Chat,
    now: i64,
) -> Vec<(Panel, ratatui::layout::Rect)> {
    let mut placed = Vec::new();
    let [top, middle, typed, bottom] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(3),
        Constraint::Length(
            if screen.typing.is_some() && screen.typing_for != Typing::Chat {
                3
            } else {
                0
            },
        ),
        // Two lines, always: the panel's keys and the screen's. A bar whose height
        // followed the panel made the whole screen jump on every tab.
        Constraint::Length(2),
    ])
    .areas(frame.area());
    let Ok(status) = status else {
        frame.render_widget(
            Paragraph::new("no engine at the socket: remux daemon start")
                .style(Style::new().fg(Color::Red)),
            top,
        );
        return placed;
    };
    let colour = if status.on_air {
        Color::Red
    } else {
        Color::DarkGray
    };
    let line = format!("{} · {}", air(status, now), scene_line(status));
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(line, Style::new().fg(colour).add_modifier(Modifier::BOLD)),
            Line::styled(out_line(status), Style::new().fg(Color::Gray)),
        ])
        .block(Block::bordered().title(" remux ")),
        top,
    );
    // What the last action came to, at the top right, for a few seconds.
    if let (Some(said), true) = (&screen.said, screen.said_fresh) {
        let inner = top.inner(ratatui::layout::Margin::new(1, 1));
        let style = if said.starts_with("no") || said.contains("error") || said.contains("refus") {
            Style::new().fg(Color::Red)
        } else {
            Style::new().fg(Color::Green)
        };
        frame.render_widget(
            Paragraph::new(Line::styled(crate::words::plain(said), style))
                .alignment(ratatui::layout::Alignment::Right),
            inner,
        );
    }
    // Three columns, each of the panels it holds that are shown; a column with none
    // gives its room to the others.
    let columns: [(&[(Panel, u16)], u16); 3] = [
        (
            &[
                (Panel::Scenes, 30),
                (Panel::Layers, 45),
                (Panel::Companions, 25),
            ],
            28,
        ),
        (&[(Panel::Destinations, 40), (Panel::Sound, 60)], 42),
        (&[(Panel::Chat, 65), (Panel::Log, 35)], 30),
    ];
    let present: Vec<(Vec<(Panel, u16)>, u16)> = columns
        .iter()
        .map(|(panels, weight)| {
            (
                panels
                    .iter()
                    .copied()
                    .filter(|(p, _)| screen.shows(*p))
                    .collect::<Vec<_>>(),
                *weight,
            )
        })
        .filter(|(panels, _)| !panels.is_empty())
        .collect();
    let areas = Layout::horizontal(present.iter().map(|(_, w)| Constraint::Fill(*w))).split(middle);
    for ((panels, _), column) in present.iter().zip(areas.iter()) {
        let rows =
            Layout::vertical(panels.iter().map(|(_, w)| Constraint::Fill(*w))).split(*column);
        for ((panel, _), area) in panels.iter().zip(rows.iter()) {
            draw_panel(frame, *panel, *area, status, levels, meters, screen, chat);
            placed.push((*panel, *area));
        }
    }
    if let (Some(line), false) = (&screen.typing, screen.typing_for == Typing::Chat) {
        frame.render_widget(
            Paragraph::new(format!("{line}▏")).block(
                Block::bordered()
                    .title(match screen.typing_for {
                        Typing::Chat => " say in the chat · enter sends · esc lets it go ",
                        Typing::Draft(_) => {
                            " the new scene's name · enter drafts it in the preview "
                        }
                        Typing::Companion(_) => " words to the companion · enter sends them ",
                    })
                    .border_style(Style::new().fg(Color::Yellow)),
            ),
            typed,
        );
    }
    // The keys: the focused panel's bright on the first line, the screen's dim on
    // the second; a line too long for the width is cut, never wrapped.
    let [own, global] = key_lines(screen.focus);
    let line = |tokens: &[&'static str], style: Style| {
        let mut spans = vec![Span::styled(
            format!(" {} ", screen.focus.word()),
            style.add_modifier(Modifier::REVERSED),
        )];
        for token in tokens {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(*token, style));
        }
        Line::from(spans)
    };
    let bright = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
    let mut lines = vec![line(own, bright)];
    let mut screen_line = vec![Span::styled(
        " controls ",
        Style::new()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::REVERSED),
    )];
    for token in global {
        screen_line.push(Span::raw("  "));
        screen_line.push(Span::styled(*token, Style::new().fg(Color::DarkGray)));
    }
    lines.push(Line::from(screen_line));
    frame.render_widget(Paragraph::new(lines), bottom);
    if let Some(lever) = &screen.asking {
        ask_about(frame, lever);
    }
    if screen.showing_keys {
        draw_keys(frame);
    }
    if let Some(picker) = &screen.picker {
        draw_picker(frame, picker);
    }
    placed
}

/// The panel drawn at a point, and the row inside its border the point is on.
pub fn panel_at(
    placed: &[(Panel, ratatui::layout::Rect)],
    x: u16,
    y: u16,
) -> Option<(Panel, Option<usize>)> {
    let (panel, area) = placed
        .iter()
        .find(|(_, a)| x >= a.x && x < a.x + a.width && y >= a.y && y < a.y + a.height)?;
    let inside = y > area.y && y + 1 < area.y + area.height;
    Some((*panel, inside.then(|| (y - area.y - 1) as usize)))
}

/// A click: the panel in focus, and in a list the row under it picked.
pub fn clicked(screen: &mut Screen, status: &Status, panel: Panel, row: Option<usize>) {
    screen.focus = panel;
    let Some(row) = row else { return };
    match panel {
        Panel::Scenes if row < status.scenes.len() && row != screen.picked => {
            screen.picked = row;
            screen.picked_layer = 0;
        }
        Panel::Layers => {
            let rows = status
                .scenes
                .get(screen.picked)
                .map_or(0, |scene| scene.ordered_ids().len());
            if row < rows {
                screen.picked_layer = row;
            }
        }
        Panel::Destinations if row < status.destinations.len() => screen.picked_destination = row,
        Panel::Companions if row < screen.companions.len() => screen.picked_companion = row,
        _ => {}
    }
}

/// One panel, drawn where it is put, its number in its title.
#[allow(clippy::too_many_arguments)]
fn draw_panel(
    frame: &mut Frame,
    panel: Panel,
    area: ratatui::layout::Rect,
    status: &Status,
    levels: Option<&(Hearing, Mixing)>,
    meters: &Meters,
    screen: &Screen,
    chat: &Chat,
) {
    let focused = screen.focus == panel;
    let block = |title: String| {
        Block::bordered()
            .title(format!(" {} {title} ", panel.number()))
            .border_style(Style::new().fg(if focused {
                Color::Yellow
            } else {
                Color::DarkGray
            }))
    };
    let picked = |row: usize, rows: usize| {
        ListState::default().with_selected(focused.then(|| row.min(rows.saturating_sub(1))))
    };
    let reversed = Style::new().add_modifier(Modifier::REVERSED);
    match panel {
        Panel::Scenes => {
            let scenes: Vec<ListItem> = status
                .scenes
                .iter()
                .map(|scene| {
                    let mark = if scene.name == status.active_scene {
                        "▶ "
                    } else if status.staged.as_deref() == Some(scene.name.as_str()) {
                        "◇ "
                    } else {
                        "  "
                    };
                    ListItem::new(format!("{mark}{}", scene.name))
                })
                .collect();
            let title = match status.trash.len() {
                0 => "scenes".to_string(),
                n => format!("scenes · {n} in the trash, U brings the last back"),
            };
            frame.render_stateful_widget(
                List::new(scenes)
                    .block(block(title))
                    .highlight_style(reversed),
                area,
                &mut picked(screen.picked, status.scenes.len()),
            );
        }
        Panel::Layers => {
            // The picked scene's, which need not be the one on the air: what a scene
            // holds is read before it goes out.
            let shown = status
                .scenes
                .get(screen.picked.min(status.scenes.len().saturating_sub(1)));
            let layers: Vec<ListItem> = shown
                .map(rows_of)
                .unwrap_or_default()
                .into_iter()
                .map(ListItem::new)
                .collect();
            let rows = layers.len();
            let title = format!("layers · {}", shown.map_or("", |scene| scene.name.as_str()));
            frame.render_stateful_widget(
                List::new(layers)
                    .block(block(title))
                    .highlight_style(reversed),
                area,
                &mut picked(screen.picked_layer, rows),
            );
        }
        Panel::Destinations => {
            let destinations: Vec<ListItem> = status
                .destinations
                .iter()
                .map(|d| {
                    ListItem::new(destination_row(d)).style(Style::new().fg(destination_colour(d)))
                })
                .collect();
            frame.render_stateful_widget(
                List::new(destinations)
                    .block(block("destinations".into()))
                    .highlight_style(reversed),
                area,
                &mut picked(screen.picked_destination, status.destinations.len()),
            );
        }
        Panel::Sound => {
            frame.render_widget(
                Paragraph::new(sound(
                    status,
                    levels,
                    meters,
                    focused.then_some(screen.picked_sound),
                    screen
                        .genre
                        .as_ref()
                        .and_then(|id| screen.genres.iter().find(|g| &g.id == id))
                        .map(|g| g.name.as_str()),
                    area.width.saturating_sub(16) as usize,
                ))
                .block(block("sound".into())),
                area,
            );
        }
        Panel::Chat => {
            let title = match (chat.reachable, screen.chat_back) {
                (false, _) => "chat · no wire".to_string(),
                (true, 0) => "chat".to_string(),
                (true, back) => format!("chat · {back} back · j to come forward"),
            };
            // A line said in the chat is typed in the chat, below what was said.
            let saying = screen
                .typing
                .as_deref()
                .filter(|_| screen.typing_for == Typing::Chat);
            let [area, typed] = Layout::vertical([
                Constraint::Min(3),
                Constraint::Length(if saying.is_some() { 3 } else { 0 }),
            ])
            .areas(area);
            if let Some(line) = saying {
                frame.render_widget(
                    Paragraph::new(format!("{line}▏")).block(
                        Block::bordered()
                            .title(" say · enter sends · esc lets it go ")
                            .border_style(Style::new().fg(Color::Yellow)),
                    ),
                    typed,
                );
            }
            let fits = area.height.saturating_sub(2) as usize;
            let width = area.width.saturating_sub(2) as usize;
            let read = chat.lines.len().saturating_sub(screen.chat_back);
            let mut rows = chat_rows(&chat.lines[..read], width);
            let shown: Vec<ListItem> = rows
                .drain(rows.len().saturating_sub(fits)..)
                .map(ListItem::new)
                .collect();
            frame.render_widget(List::new(shown).block(block(title)), area);
        }
        Panel::Companions => draw_companions(frame, screen, area, block("companions".into())),
        Panel::Log => {
            let fits = area.height.saturating_sub(2) as usize;
            let end = screen.log.len().saturating_sub(screen.log_back);
            let lines: Vec<ListItem> = if screen.log.is_empty() {
                vec![ListItem::new("nothing yet").style(Style::new().fg(Color::DarkGray))]
            } else {
                screen.log[end.saturating_sub(fits)..end]
                    .iter()
                    .map(|line| {
                        let style = log_colour(line).map_or(Style::new(), |c| Style::new().fg(c));
                        ListItem::new(crate::words::plain(line)).style(style)
                    })
                    .collect()
            };
            let title = match screen.log_back {
                0 => "log".to_string(),
                back => format!("log · {back} back · j comes forward"),
            };
            frame.render_widget(List::new(lines).block(block(title)), area);
        }
    }
}

/// A list to pick from, over the rest.
fn draw_picker(frame: &mut Frame, picker: &Picker) {
    let area = frame.area();
    let tall = (picker.rows.len() as u16 + 2).min(area.height.saturating_sub(4));
    let [_, middle, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(tall),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, middle, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(60),
        Constraint::Fill(1),
    ])
    .areas(middle);
    let rows: Vec<ListItem> = picker
        .rows
        .iter()
        .map(|(label, _)| ListItem::new(crate::words::plain(label)))
        .collect();
    frame.render_widget(ratatui::widgets::Clear, middle);
    frame.render_stateful_widget(
        List::new(rows)
            .block(
                Block::bordered()
                    .title(format!(
                        " {} · j/k · enter picks · esc closes ",
                        picker.title
                    ))
                    .border_style(Style::new().fg(Color::Yellow)),
            )
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        middle,
        &mut ListState::default().with_selected(Some(picker.at)),
    );
}

/// Every key, panel by panel, over the rest.
fn draw_keys(frame: &mut Frame) {
    let area = frame.area();
    let [_, middle, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Percentage(80),
        Constraint::Fill(1),
    ])
    .areas(area);
    let [_, middle, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(80),
        Constraint::Fill(1),
    ])
    .areas(middle);
    let width = middle.width.saturating_sub(16) as usize;
    let mut lines: Vec<Line> = Vec::new();
    let mut group = |name: String, keys: &[&str]| {
        for (n, tokens) in wrap_tokens(keys, width).into_iter().enumerate() {
            let head = if n == 0 { name.clone() } else { String::new() };
            lines.push(Line::from(vec![
                Span::styled(format!("{head:<14}"), Style::new().fg(Color::Yellow)),
                Span::raw(tokens.join("  ")),
            ]));
        }
    };
    for panel in Panel::ALL {
        group(
            format!("{} {}", panel.number(), panel.word()),
            keys_of(panel),
        );
    }
    group("everywhere".into(), GLOBAL_KEYS);
    frame.render_widget(ratatui::widgets::Clear, middle);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" every key · any key closes ")
                .border_style(Style::new().fg(Color::Yellow)),
        ),
        middle,
    );
}

/// How many lines the log keeps.
const LOG: usize = 500;

/// The journal's lines after the last one seen; all of them when that one fell
/// off its end or none was seen.
pub fn fresh_journal<'a>(seen: Option<&str>, lines: &'a [String]) -> &'a [String] {
    match seen.and_then(|seen| lines.iter().rposition(|line| line == seen)) {
        Some(at) => &lines[at + 1..],
        None => lines,
    }
}

/// The events nobody's command made, as the log says them: what the air, a
/// destination, a capture or the sound did by itself. What a command did is in
/// the journal already.
pub fn events_for_log(events: &[remuxd_domain::app::events::Numbered]) -> Vec<String> {
    use remuxd_domain::app::events::Event;
    events
        .iter()
        .filter(|n| {
            matches!(
                n.event,
                Event::LiveStarted
                    | Event::LiveEnded
                    | Event::DestinationLive { .. }
                    | Event::DestinationEnded { .. }
                    | Event::LayerStalled { .. }
                    | Event::LayerFlowing { .. }
                    | Event::SoundComplaint { .. }
                    | Event::AudioGlitch { .. }
                    | Event::Notice { .. }
                    | Event::AppReachable { .. }
                    | Event::TimerFinished { .. }
            )
        })
        .map(|n| {
            // As the CLI words it, its number left out: "HH:MM:SS #12 what".
            let said = crate::words::render_events(None, std::slice::from_ref(n));
            match said
                .split_once(" #")
                .and_then(|(at, rest)| rest.split_once(' ').map(|(_, what)| (at, what)))
            {
                Some((at, what)) => format!("{at} {what}"),
                None => said,
            }
        })
        .collect()
}

/// What changed among the companions, a line each; one that fell says where to
/// read why.
pub fn companion_changes(before: &[(String, State)], after: &[(String, State)]) -> Vec<String> {
    let kind = |state: &State| match state {
        State::Up(_) => 0,
        State::Fell(_) => 1,
        State::Down => 2,
    };
    after
        .iter()
        .filter_map(|(name, now)| {
            let was = before.iter().find(|(n, _)| n == name).map(|(_, s)| kind(s));
            if was == Some(kind(now)) || (was.is_none() && matches!(now, State::Down)) {
                return None;
            }
            Some(match now {
                State::Up(_) => format!("companion {name} up"),
                State::Fell(_) => format!("companion {name} fell: remux companion log {name}"),
                State::Down => format!("companion {name} down"),
            })
        })
        .collect()
}

/// A log line's colour: what went wrong red, a notice yellow, the rest plain.
pub fn log_colour(line: &str) -> Option<Color> {
    let wrong = [
        " ! ",
        " refused",
        " dropped",
        "stopped delivering",
        " fell",
        "glitch",
        "complaint",
    ];
    if wrong.iter().any(|w| line.contains(w)) {
        Some(Color::Red)
    } else if line.contains("notice:") {
        Some(Color::Yellow)
    } else {
        None
    }
}

/// A line into the log, stamped now, the oldest let go past its length.
fn to_log(screen: &mut Screen, line: String) {
    screen.log.push(line);
    let over = screen.log.len().saturating_sub(LOG);
    screen.log.drain(..over);
}

/// The time of day here, HH:MM:SS.
fn local_time() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as libc::time_t);
    // SAFETY: localtime_r writes into the struct given and reads only `now`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
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
        Lever::Delete(name) => (
            " delete? ",
            format!("the scene {name}, for good\n\ny: delete · any other key: no"),
            Color::Red,
        ),
        Lever::Remove { id, staged } => (
            " remove? ",
            format!(
                "the layer {id}{}, for good\n\ny: remove · any other key: no",
                if *staged {
                    " from the preview"
                } else {
                    " from the air"
                }
            ),
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
    /// A scene deleted, for good.
    Delete(String),
    /// A layer removed from the scene on the air, or from the staged one.
    Remove {
        id: String,
        staged: bool,
    },
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
            Lever::Delete(name) => Some(Command::SceneDelete { name: name.clone() }),
            Lever::Remove { id, staged } => {
                let remove = Command::LayerRemove { id: id.clone() };
                Some(if *staged {
                    Command::Staged {
                        command: Box::new(remove),
                    }
                } else {
                    remove
                })
            }
        }
    }
}

/// The filter after this one in the list, the first when it is none or one not
/// in the list, and none after the last: a key steps through and back to off.
pub fn next_filter(list: &[String], now: Option<&str>) -> Option<String> {
    match now.and_then(|now| list.iter().position(|f| f == now)) {
        Some(at) => list.get(at + 1).cloned(),
        None => list.first().cloned(),
    }
}

/// The WGSL files of a folder, by name.
pub fn filters_in(dir: &std::path::Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|e| e == "wgsl"))
                .map(|path| path.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

/// A list over the screen: the microphones, each row's label and device, and the
/// one picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub title: String,
    pub rows: Vec<(String, Option<String>)>,
    pub at: usize,
}

/// The microphones, and none.
pub fn mic_picker(devices: &remuxd_domain::protocol::Devices) -> Picker {
    Picker {
        title: "microphone".into(),
        rows: std::iter::once(("no microphone".to_string(), None))
            .chain(
                devices
                    .mics
                    .iter()
                    .map(|m| (m.name.clone(), Some(m.name.clone()))),
            )
            .collect(),
        at: 0,
    }
}

/// What a line typed is for.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Typing {
    /// A line said in the chat.
    #[default]
    Chat,
    /// A new scene's name, drafted in the preview, empty or a copy of a scene.
    Draft(Option<String>),
    /// Words to a companion that takes input.
    Companion(String),
}

/// The line typed, done: said, drafted or added.
fn typed_line(line: String, typing_for: Typing) -> Act {
    match typing_for {
        Typing::Chat => Act::Send(Command::Say {
            body: line,
            channel: None,
        }),
        Typing::Draft(from) => Act::Send(Command::SceneDraft {
            name: line.trim().to_string(),
            from,
        }),
        Typing::Companion(name) => Act::CompanionSend {
            name,
            words: line.trim().to_string(),
        },
    }
}

/// Which list the keys move in.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    #[default]
    Scenes,
    /// The picked scene's layers and elements.
    Layers,
    Destinations,
    /// The mic, the music and each audio layer.
    Sound,
    /// The chat, read back through.
    Chat,
    /// The operator's programs beside the engine.
    Companions,
    /// What the engine did and what went wrong, from any face, and this
    /// screen's own.
    Log,
}

impl Panel {
    /// Every panel, in the order of its number and of tab.
    pub const ALL: [Panel; 7] = [
        Panel::Scenes,
        Panel::Layers,
        Panel::Destinations,
        Panel::Sound,
        Panel::Chat,
        Panel::Companions,
        Panel::Log,
    ];

    /// The digit that shows or hides it.
    pub fn number(self) -> char {
        char::from(b'1' + Panel::ALL.iter().position(|p| *p == self).unwrap_or(0) as u8)
    }

    /// Its name, as the panel's title and as `tui.toml` keeps it.
    pub fn word(self) -> &'static str {
        match self {
            Panel::Scenes => "scenes",
            Panel::Layers => "layers",
            Panel::Destinations => "destinations",
            Panel::Sound => "sound",
            Panel::Chat => "chat",
            Panel::Companions => "companions",
            Panel::Log => "log",
        }
    }
}

/// The panels hidden, as `tui.toml` keeps them.
pub fn keep_hidden(hidden: &[Panel]) -> String {
    let words: Vec<String> = hidden.iter().map(|p| format!("\"{}\"", p.word())).collect();
    format!("hidden = [{}]\n", words.join(", "))
}

/// The panels a `tui.toml` hides; a word that names no panel is let go.
pub fn read_hidden(text: &str) -> Vec<Panel> {
    let Some(list) = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("hidden"))
        .and_then(|rest| rest.split_once('[').map(|(_, rest)| rest))
        .and_then(|rest| rest.split_once(']').map(|(list, _)| list))
    else {
        return Vec::new();
    };
    list.split(',')
        .map(|word| word.trim().trim_matches('"'))
        .filter_map(|word| Panel::ALL.into_iter().find(|p| p.word() == word))
        .collect()
}

/// Where the screen keeps which panels are hidden: the person's, not the engine's.
fn tui_toml() -> std::path::PathBuf {
    remuxd_domain::os::config_dir().join("tui.toml")
}

/// Each panel's keys, as the key bar and the keys pane say them.
pub fn keys_of(panel: Panel) -> &'static [&'static str] {
    match panel {
        Panel::Scenes => &[
            "[enter] preview",
            "[t] take",
            "[N] new",
            "[D] copy",
            "[X] delete",
            "[U] undelete",
            "[j/k] pick",
            "[h] hide",
        ],
        Panel::Layers => &[
            "[space] hide/show",
            "[J/K] forward/back",
            "[x] remove",
            "[f] filter",
            "[F] scene filter",
            "[j/k] pick",
            "[h] hide",
        ],
        Panel::Destinations => &["[a] arm", "[s] sandbox", "[j/k] pick", "[h] hide"],
        Panel::Sound => &[
            "[enter] choose the mic",
            "[space] on/off",
            "[+/-] volume",
            "[d] duck",
            "[s] music to the live",
            "[g] genre",
            "[j/k] pick",
            "[h] hide",
        ],
        Panel::Chat => &["[c] say", "[k] older", "[j] newer", "[h] hide"],
        Panel::Companions => &[
            "[space] start/stop",
            "[e] type a command to it",
            "[j/k] pick",
            "[h] hide",
        ],
        Panel::Log => &["[k] older", "[j] newer", "[h] hide"],
    }
}

/// The keys every panel has.
pub const GLOBAL_KEYS: &[&str] = &[
    "[tab/shift+tab] panels",
    "[1-7] go to",
    "[0] show all",
    "[L] live",
    "[S] stop",
    "[R] record",
    "[!] cut",
    "[m] mute",
    "[n] next track",
    "[p] pause",
    "[?] all keys",
    "[q] quit",
];

/// The bar's two lines, whatever the panel: its own keys, then the screen's.
pub fn key_lines(panel: Panel) -> [&'static [&'static str]; 2] {
    [keys_of(panel), GLOBAL_KEYS]
}

/// Tokens two spaces apart, onto as many lines of `width` as they need.
pub fn wrap_tokens<'a>(tokens: &[&'a str], width: usize) -> Vec<Vec<&'a str>> {
    let mut lines: Vec<Vec<&str>> = Vec::new();
    let mut used = 0;
    for token in tokens {
        let wide = token.chars().count();
        match lines.last_mut() {
            Some(line) if used + 2 + wide <= width => {
                line.push(token);
                used += 2 + wide;
            }
            _ => {
                lines.push(vec![token]);
                used = wide;
            }
        }
    }
    lines
}

/// What the screen holds between keys: the panel in focus, the picked row in each, and
/// the question open, if any.
#[derive(Debug, Default)]
pub struct Screen {
    pub focus: Panel,
    pub picked: usize,
    pub picked_layer: usize,
    pub picked_destination: usize,
    /// The mic is row 0, the music row 1, the audio layers after them.
    pub picked_sound: usize,
    /// How many lines of chat back from the last the panel ends: zero follows the chat.
    pub chat_back: usize,
    pub picked_companion: usize,
    /// Each companion and its state as last read.
    pub companions: Vec<(String, State)>,
    /// Why there is no list of companions, when there is none.
    pub companion_trouble: Option<String>,
    /// Whether the music plays, as the meters last said.
    pub playing: bool,
    pub asking: Option<Lever>,
    /// A line being typed: every key is a letter until Enter or Esc.
    pub typing: Option<String>,
    /// What the line typed is for.
    pub typing_for: Typing,
    /// The music's genres, read once, and the one last chosen here (the
    /// engine does not say which plays).
    pub genres: Vec<remuxd_domain::protocol::Named>,
    pub genre: Option<String>,
    /// The filters on offer: the WGSL files of the shaders folder.
    pub filters: Vec<String>,
    /// The log: the engine's journal and what happened by itself, and what only
    /// this screen saw; and how far back it is read.
    pub log: Vec<String>,
    pub log_back: usize,
    /// Whether every key is shown, over the rest.
    pub showing_keys: bool,
    /// A list to pick from, over the rest.
    pub picker: Option<Picker>,
    /// The panels not drawn: their keys still work once shown again.
    pub hidden: Vec<Panel>,
    /// What the last thing sent came to, and whether it is new enough to show.
    pub said: Option<String>,
    pub said_fresh: bool,
}

impl Screen {
    /// Whether this panel is drawn.
    pub fn shows(&self, panel: Panel) -> bool {
        !self.hidden.contains(&panel)
    }

    /// The panel before this one that is drawn, round from the first.
    fn previous_shown(&self, from: Panel) -> Panel {
        let at = Panel::ALL.iter().position(|p| *p == from).unwrap_or(0);
        let all = Panel::ALL.len();
        (1..=all)
            .map(|step| Panel::ALL[(at + all - step) % all])
            .find(|p| self.shows(*p))
            .unwrap_or(from)
    }

    /// The panel after this one that is drawn, round from the last.
    fn next_shown(&self, from: Panel) -> Panel {
        let at = Panel::ALL.iter().position(|p| *p == from).unwrap_or(0);
        (1..=Panel::ALL.len())
            .map(|step| Panel::ALL[(at + step) % Panel::ALL.len()])
            .find(|p| self.shows(*p))
            .unwrap_or(from)
    }

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
    /// Send nothing, and say why.
    Say(&'static str),
    /// Ask the engine for the microphones, to offer them in a list.
    AskMics,
    /// Words to a companion's standard input, by the shell's own `remux companion`.
    CompanionSend {
        name: String,
        words: String,
    },
    /// Start or stop a companion, by the shell's own `remux companion`.
    Companion {
        name: String,
        start: bool,
    },
}

/// A key. `j`/`k` move the pick without leaving the list, Enter switches to it, `q` quits.
/// `L`, `S`, `R` and `!`
/// only open a question; `y` answers it and sends, and any other key lets it go, `q`
/// included: under a question, `q` never closes the screen.
pub fn press(key: char, screen: &mut Screen, rows: usize, status: &Status) -> Act {
    if let Some(mut line) = screen.typing.take() {
        let typing_for = std::mem::take(&mut screen.typing_for);
        match key {
            '\u{1b}' => {}
            '\n' if line.trim().is_empty() => {}
            '\n' => return typed_line(line, typing_for),
            '\u{8}' => {
                line.pop();
                screen.typing = Some(line);
                screen.typing_for = typing_for;
            }
            letter => {
                line.push(letter);
                screen.typing = Some(line);
                screen.typing_for = typing_for;
            }
        }
        return Act::Stay;
    }
    if let Some(mut picker) = screen.picker.take() {
        match key {
            'j' => picker.at = (picker.at + 1).min(picker.rows.len().saturating_sub(1)),
            'k' => picker.at = picker.at.saturating_sub(1),
            '\u{1b}' | 'q' => return Act::Stay,
            '\n' => {
                return match picker.rows.get(picker.at).cloned() {
                    Some((_, device)) => Act::Send(Command::Mic { device }),
                    None => Act::Stay,
                };
            }
            _ => {}
        }
        screen.picker = Some(picker);
        return Act::Stay;
    }
    // The keys are read, not acted on: any key closes them.
    if screen.showing_keys {
        screen.showing_keys = false;
        return Act::Stay;
    }
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
    let destination = status.destinations.get(screen.picked_destination);
    let scene = status.scenes.get(screen.picked);
    let ids = scene.map(Scene::ordered_ids).unwrap_or_default();
    let layer = ids.get(screen.picked_layer).cloned();
    // A layer's verbs act on the scene on the air, or on the staged one by way of the
    // preview: a layer of a third scene with the same id would be the air's.
    let on_air = scene.is_some_and(|scene| scene.name == status.active_scene);
    let in_preview = scene.is_some_and(|scene| Some(&scene.name) == status.staged.as_ref());
    let send = |command: Command| {
        if on_air {
            Act::Send(command)
        } else {
            Act::Send(Command::Staged {
                command: Box::new(command),
            })
        }
    };
    match (key, screen.focus) {
        ('q' | '\u{1b}', _) => return Act::Quit,
        ('?', _) => screen.showing_keys = true,
        ('1'..='7', _) => {
            let panel = Panel::ALL[(key as u8 - b'1') as usize];
            screen.hidden.retain(|p| *p != panel);
            screen.focus = panel;
        }
        ('0', _) => screen.hidden.clear(),
        ('h', focus) => {
            if Panel::ALL.iter().filter(|p| screen.shows(**p)).count() == 1 {
                return Act::Say("one panel stays shown");
            }
            screen.hidden.push(focus);
            screen.focus = screen.next_shown(focus);
        }
        ('\t', focus) => screen.focus = screen.next_shown(focus),
        ('\u{19}', focus) => screen.focus = screen.previous_shown(focus),
        ('c', Panel::Chat) => {
            screen.typing_for = Typing::Chat;
            screen.typing = Some(String::new());
        }
        // The sound's everyday gestures, from any panel, at once.
        ('m', _) => return Act::Send(Command::Mute { on: !status.muted }),
        ('n', _) => return Act::Send(Command::NextTrack),
        ('p', _) => {
            return Act::Send(Command::Music {
                on: !screen.playing,
            })
        }
        ('+', focus) if focus != Panel::Sound => {
            return Act::Send(Command::MusicVolume {
                level: louder(status.faders.music, 1.0),
            })
        }
        ('-', focus) if focus != Panel::Sound => {
            return Act::Send(Command::MusicVolume {
                level: quieter(status.faders.music),
            })
        }
        ('j', Panel::Companions) if screen.picked_companion + 1 < screen.companions.len() => {
            screen.picked_companion += 1
        }
        ('k', Panel::Companions) => {
            screen.picked_companion = screen.picked_companion.saturating_sub(1)
        }
        ('e', Panel::Companions) => {
            if let Some((name, _)) = screen.companions.get(screen.picked_companion) {
                screen.typing_for = Typing::Companion(name.clone());
                screen.typing = Some(String::new());
            }
        }
        (' ', Panel::Companions) => {
            if let Some((name, state)) = screen.companions.get(screen.picked_companion) {
                return Act::Companion {
                    name: name.clone(),
                    start: !matches!(state, State::Up(_)),
                };
            }
        }
        ('k', Panel::Chat) => screen.chat_back += 1,
        ('k', Panel::Log)
            if screen.log_back + 1 < screen.log.len().max(1) || screen.log.is_empty() =>
        {
            screen.log_back += 1
        }
        ('j', Panel::Log) => screen.log_back = screen.log_back.saturating_sub(1),
        ('j', Panel::Chat) => screen.chat_back = screen.chat_back.saturating_sub(1),
        ('j', Panel::Sound) if screen.picked_sound < 1 + status.audio_layers.len() => {
            screen.picked_sound += 1
        }
        ('k', Panel::Sound) => screen.picked_sound = screen.picked_sound.saturating_sub(1),
        ('g', Panel::Sound) if screen.picked_sound == 1 && !screen.genres.is_empty() => {
            let at = screen
                .genre
                .as_ref()
                .and_then(|now| screen.genres.iter().position(|g| &g.id == now))
                .map_or(0, |at| (at + 1) % screen.genres.len());
            let id = screen.genres[at].id.clone();
            screen.genre = Some(id.clone());
            return Act::Send(Command::Genre { name: id });
        }
        (' ' | '+' | '-' | 'd' | 's', Panel::Sound) => {
            if let Some(command) = sound_verb(key, screen.picked_sound, screen.playing, status) {
                return Act::Send(command);
            }
        }
        ('j', Panel::Scenes) if screen.picked + 1 < rows => {
            screen.picked += 1;
            screen.picked_layer = 0;
        }
        ('k', Panel::Scenes) => {
            screen.picked = screen.picked.saturating_sub(1);
            screen.picked_layer = 0;
        }
        ('j', Panel::Layers) if screen.picked_layer + 1 < ids.len() => screen.picked_layer += 1,
        ('k', Panel::Layers) => screen.picked_layer = screen.picked_layer.saturating_sub(1),
        (' ' | 'J' | 'K' | 'x' | 'f' | 'F', Panel::Layers) if !on_air && !in_preview => {
            return Act::Say("enter stages this scene first")
        }
        ('f', Panel::Layers) => {
            if let (Some(id), Some(scene)) = (layer, scene) {
                let now = scene
                    .layers
                    .iter()
                    .find(|l| l.id == id)
                    .and_then(|l| l.shader.clone())
                    .or_else(|| {
                        scene
                            .elements
                            .iter()
                            .find(|e| e.id == id)
                            .and_then(|e| e.shader.clone())
                    });
                let path = next_filter(&screen.filters, now.as_deref());
                if path != now {
                    return send(Command::LayerShader { id, path });
                }
            }
        }
        ('F', Panel::Layers) => {
            if let Some(scene) = scene {
                let path = next_filter(&screen.filters, scene.shader.as_deref());
                if path != scene.shader {
                    return send(Command::Shader { path });
                }
            }
        }
        ('\n', Panel::Sound) if screen.picked_sound == 0 => return Act::AskMics,
        ('x', Panel::Layers) => {
            if let Some(id) = layer {
                screen.asking = Some(Lever::Remove {
                    id,
                    staged: !on_air,
                });
            }
        }
        ('N', Panel::Scenes) => {
            screen.typing = Some(String::new());
            screen.typing_for = Typing::Draft(None);
        }
        ('D', Panel::Scenes) => {
            if let Some(scene) = scene {
                screen.typing = Some(String::new());
                screen.typing_for = Typing::Draft(Some(scene.name.clone()));
            }
        }
        ('X', Panel::Scenes) if on_air => return Act::Say("the scene on the air is not deleted"),
        ('U', Panel::Scenes) => {
            return match status.trash.first() {
                Some(scene) => Act::Send(Command::SceneRestore {
                    name: scene.name.clone(),
                }),
                None => Act::Say("the trash is empty"),
            }
        }
        ('X', Panel::Scenes) => {
            if let Some(scene) = scene {
                screen.asking = Some(Lever::Delete(scene.name.clone()));
            }
        }
        (' ', Panel::Layers) => {
            if let (Some(id), Some(scene)) = (layer, scene) {
                let on = !shown(scene, &id);
                return send(Command::LayerVisible { id, on });
            }
        }
        ('J', Panel::Layers) if screen.picked_layer + 1 < ids.len() => {
            screen.picked_layer += 1;
            if let Some(id) = layer {
                return send(Command::LayerMove {
                    id,
                    index: screen.picked_layer,
                });
            }
        }
        ('K', Panel::Layers) if screen.picked_layer > 0 => {
            screen.picked_layer -= 1;
            if let Some(id) = layer {
                return send(Command::LayerMove {
                    id,
                    index: screen.picked_layer,
                });
            }
        }
        ('j', Panel::Destinations) if screen.picked_destination + 1 < status.destinations.len() => {
            screen.picked_destination += 1
        }
        ('k', Panel::Destinations) => {
            screen.picked_destination = screen.picked_destination.saturating_sub(1)
        }
        ('a', Panel::Destinations) => {
            if let Some(d) = destination {
                return Act::Send(Command::Arm {
                    adapter: d.id,
                    on: !d.armed,
                });
            }
        }
        ('s', Panel::Destinations) => {
            if let Some(d) = destination {
                return Act::Send(Command::Sandbox {
                    adapter: d.id,
                    on: !d.sandbox,
                });
            }
        }
        // As a studio does: Enter stages the scene, off the air, and `t` takes
        // it. The scene on the air staged is nothing staged.
        ('\n', Panel::Scenes) => {
            if let Some(scene) = status.scenes.get(screen.picked) {
                if scene.name != status.active_scene || status.staged.is_some() {
                    return Act::Send(Command::SceneStage {
                        name: scene.name.clone(),
                    });
                }
            }
        }
        ('t', _) if status.staged.is_some() => return Act::Send(Command::SceneTake),
        ('t', _) => return Act::Say("nothing staged: enter stages a scene"),
        ('L', _) if !status.on_air => return Act::Plan,
        ('S', _) if status.on_air => screen.asking = Some(Lever::Stop),
        ('R', _) => {
            screen.asking = Some(Lever::Record {
                on: !status.recording,
            })
        }
        ('!', _) => screen.asking = Some(Lever::Cut),
        _ => {}
    }
    Act::Stay
}

/// One step of the music fader: 3 dB, a gain of 10^(3/20). The fader is a linear gain,
/// and the music sits near half a percent, where a step of a percent would be 6 dB.
const STEP: f64 = 1.412_537_544_622_754;
/// Below this the fader is silence; a step louder from silence lands here.
const QUIETEST: f64 = 0.001;

/// A fader's step louder, never past its ceiling: 100% for the music, 200% for the
/// mic and the audio layers, which a quiet source needs.
fn louder(level: f64, ceiling: f64) -> f64 {
    (level * STEP).clamp(QUIETEST, ceiling)
}

/// What a key does to a row of the sound panel: space turns it off or on, `+` and `-`
/// move its fader, `d` steps its duck, and `s` on the music says whether the live hears
/// it.
fn sound_verb(key: char, row: usize, playing: bool, status: &Status) -> Option<Command> {
    use remuxd_domain::sound::audio_layers::Duck;
    Some(match (key, row) {
        (' ', 0) => Command::Mute { on: !status.muted },
        ('+', 0) => Command::Volume {
            level: louder(status.faders.mic, 2.0),
        },
        ('-', 0) => Command::Volume {
            level: quieter(status.faders.mic),
        },
        // Off is silent: off the live alone, the music still plays in the speakers and
        // on its meter, which read as a key that did nothing.
        (' ', 1) => Command::Music { on: !playing },
        ('s', 1) => Command::StreamMusic {
            on: !status.music_to_stream,
        },
        ('+', 1) => Command::MusicVolume {
            level: louder(status.faders.music, 1.0),
        },
        ('-', 1) => Command::MusicVolume {
            level: quieter(status.faders.music),
        },
        // Off, then 6 dB deeper a press, to 18, then off again.
        ('d', 1) => Command::Duck {
            db: match status.faders.duck_db {
                db if db > -3.0 => -6.0,
                db if db > -9.0 => -12.0,
                db if db > -15.0 => -18.0,
                _ => 0.0,
            },
        },
        (_, row) => {
            let layer = status.audio_layers.get(row.checked_sub(2)?)?;
            let id = layer.id.clone();
            match key {
                ' ' => Command::AudioLayerMute {
                    id,
                    on: !layer.muted,
                },
                '+' => Command::AudioLayerVolume {
                    id,
                    volume: louder(layer.volume, 2.0),
                },
                '-' => Command::AudioLayerVolume {
                    id,
                    volume: quieter(layer.volume),
                },
                'd' => Command::AudioLayerDuck {
                    id,
                    duck: match layer.duck {
                        Duck::ByKind => Duck::Off,
                        Duck::Off => Duck::On,
                        Duck::On => Duck::ByKind,
                    },
                },
                _ => return None,
            }
        }
    })
}

fn quieter(level: f64) -> f64 {
    let level = level / STEP;
    if level < QUIETEST {
        0.0
    } else {
        level
    }
}

/// The chat as far as this screen has read it.
#[derive(Debug, Default)]
pub struct Chat {
    pub reachable: bool,
    pub lines: Vec<ChatLine>,
}

impl Chat {
    /// How many lines the screen keeps; the panel shows the last that fit.
    const KEPT: usize = 200;

    /// The lines after the last one held: the engine answers only what is new.
    fn read(&mut self, ask: &impl Fn(&Command) -> Result<Reply, String>) -> Vec<ChatLine> {
        let since = self.lines.last().map_or(0, |line| line.seq);
        let Ok(Reply::Chat { reachable, lines }) = ask(&Command::Chat {
            since,
            follow: false,
        }) else {
            self.reachable = false;
            return Vec::new();
        };
        self.reachable = reachable;
        lines
    }

    /// Keeps the lines that arrived, and keeps a person reading back on the line they
    /// read: new lines below push the panel's end back by as many. Back stops at the
    /// first line held.
    pub fn hold(&mut self, screen: &mut Screen, fresh: Vec<ChatLine>) {
        if screen.chat_back > 0 {
            screen.chat_back += fresh.len();
        }
        self.lines.extend(fresh);
        let over = self.lines.len().saturating_sub(Self::KEPT);
        self.lines.drain(..over);
        screen.chat_back = screen.chat_back.min(self.lines.len().saturating_sub(1));
    }
}

/// A line of text in a column `width` cells wide: broken between words, the rest of it
/// indented by `indent`, and a word longer than the column cut where the column ends.
/// A character counts as one cell.
pub fn wrapped(text: &str, width: usize, indent: &str) -> Vec<String> {
    let width = width.max(indent.len() + 1);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let close = |out: &mut Vec<String>, line: &mut String| {
        let before = if out.is_empty() { "" } else { indent };
        out.push(format!("{before}{}", std::mem::take(line)));
    };
    for word in text.split(' ').filter(|word| !word.is_empty()) {
        let mut word: Vec<char> = word.chars().collect();
        while !word.is_empty() {
            let room = if out.is_empty() {
                width
            } else {
                width - indent.len()
            };
            let used = line.chars().count();
            let needed = if used == 0 {
                word.len()
            } else {
                used + 1 + word.len()
            };
            if needed <= room {
                if used > 0 {
                    line.push(' ');
                }
                line.extend(word.drain(..));
            } else if used > 0 {
                close(&mut out, &mut line);
            } else {
                line.extend(word.drain(..room));
                close(&mut out, &mut line);
            }
        }
    }
    if !line.is_empty() || out.is_empty() {
        close(&mut out, &mut line);
    }
    out
}

/// What leaves: the picture, its rates, and the audience across the platforms. The
/// audience needs an account (`remux login`); without one it is a dash.
pub fn out_line(status: &Status) -> String {
    let out = &status.outgoing;
    format!(
        "out {}x{} · {} fps · video {} kbps · audio {} kbps · viewers {} · peak {}",
        out.width,
        out.height,
        out.fps,
        out.video_kbps,
        out.audio_kbps,
        count(status.viewers),
        count(status.viewers_peak)
    )
}

fn count(n: Option<u32>) -> String {
    n.map_or("—".into(), |n| n.to_string())
}

/// A destination on the air is red, as the air line is; any other is grey.
pub fn destination_colour(d: &Destination) -> Color {
    if d.status == "live" {
        Color::Red
    } else {
        Color::DarkGray
    }
}

/// A destination: its lamp and state, its audience, what its platform last refused or
/// that all is well, and, with an account, whose it is and its category.
pub fn destination_row(d: &Destination) -> String {
    let lamp = if d.status == "live" {
        "●"
    } else if d.armed {
        "○"
    } else {
        "·"
    };
    let armed = if d.armed { "armed" } else { "off" };
    let sandbox = if d.sandbox { " sandbox" } else { "" };
    let trouble = d.trouble.as_deref().map(crate::words::plain);
    [
        Some(format!(
            "{lamp} {} ({}) {armed}{sandbox} {}",
            d.name, d.platform, d.status
        )),
        Some(format!("viewers {}", count(d.viewers))),
        Some(trouble.unwrap_or_else(|| "ok".into())),
        d.account.as_deref().map(crate::words::plain),
        d.category.as_deref().map(crate::words::plain),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// The chat as the CLI dresses it: the platform in its own colour (Twitch's purple,
/// YouTube's red, anything else cyan) and who said it, the words below, and a faint
/// rule between one line and the next. Every escape and control character a stranger
/// could send is taken out first.
pub fn chat_rows(lines: &[ChatLine], width: usize) -> Vec<Line<'static>> {
    let rule = Line::from(Span::styled(
        "─".repeat(width),
        Style::new().fg(Color::Indexed(238)),
    ));
    let mut rows = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            rows.push(rule.clone());
        }
        let colour = match line.platform.as_str() {
            "twitch" => Color::Indexed(141),
            "youtube" => Color::Indexed(203),
            _ => Color::Cyan,
        };
        rows.push(Line::from(vec![
            Span::styled(
                crate::words::plain(&line.platform),
                Style::new().fg(colour).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                crate::words::plain(&line.from),
                Style::new().add_modifier(Modifier::BOLD),
            ),
        ]));
        let body = crate::words::plain(&line.body);
        rows.extend(
            wrapped(&body, width.saturating_sub(2), "")
                .into_iter()
                .map(|words| Line::raw(format!("  {words}"))),
        );
    }
    rows
}

/// The scene on the air, and the one staged in the preview when there is one.
pub fn scene_line(status: &Status) -> String {
    match &status.staged {
        Some(staged) => format!("scene: {} · preview: {staged}", status.active_scene),
        None => format!("scene: {}", status.active_scene),
    }
}

/// The companions, as last read: a lamp each, green up, red fallen.
fn look_at_companions(screen: &mut Screen) {
    match crate::companion::states() {
        Ok(list) => {
            let changes = companion_changes(&screen.companions, &list);
            screen.companions = list;
            screen.companion_trouble = None;
            for change in changes {
                to_log(screen, format!("{} {change}", local_time()));
            }
        }
        Err(why) => {
            screen.companions.clear();
            screen.companion_trouble = Some(why);
        }
    }
}

fn draw_companions(frame: &mut Frame, screen: &Screen, area: ratatui::layout::Rect, block: Block) {
    let focused = screen.focus == Panel::Companions;
    let rows: Vec<ListItem> = match &screen.companion_trouble {
        Some(why) => vec![ListItem::new(why.clone()).style(Style::new().fg(Color::DarkGray))],
        None => screen
            .companions
            .iter()
            .map(|(name, state)| {
                let colour = match state {
                    State::Up(_) => Color::Green,
                    State::Fell(_) => Color::Red,
                    State::Down => Color::DarkGray,
                };
                ListItem::new(companion_row(name, *state)).style(Style::new().fg(colour))
            })
            .collect(),
    };
    let count = rows.len();
    frame.render_stateful_widget(
        List::new(rows)
            .block(block)
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        area,
        &mut ListState::default().with_selected(
            (focused && screen.companion_trouble.is_none())
                .then(|| screen.picked_companion.min(count.saturating_sub(1))),
        ),
    );
}

/// A companion: its lamp, its name, and its state; one that fell says where
/// to read why.
pub fn companion_row(name: &str, state: State) -> String {
    match state {
        State::Up(_) => format!("● {name}  up"),
        State::Fell(_) => format!("✕ {name}  fell: remux companion log {name}"),
        State::Down => format!("○ {name}  down"),
    }
}

/// Whether a scene's layer or element is shown.
fn shown(scene: &Scene, id: &str) -> bool {
    scene
        .layers
        .iter()
        .find(|layer| layer.id == id)
        .map(|layer| layer.visible)
        .or_else(|| {
            scene
                .elements
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.visible)
        })
        .unwrap_or(false)
}

/// A scene's layers and elements, back to front, one line each: what it is, what it
/// shows, and whether it is hidden.
pub fn rows_of(scene: &Scene) -> Vec<String> {
    use remuxd_domain::picture::layers::Kind;
    use remuxd_domain::picture::scenes::ElementContent;
    let hidden = |visible: bool| if visible { "" } else { " (hidden)" };
    let filtered = |shader: Option<&str>| {
        shader
            .map(|path| format!(" · filter {}", path.rsplit('/').next().unwrap_or(path)))
            .unwrap_or_default()
    };
    scene
        .ordered_ids()
        .into_iter()
        .filter_map(|id| {
            if let Some(layer) = scene.layers.iter().find(|l| l.id == id) {
                let kind = match layer.source.kind {
                    Kind::Camera => "camera",
                    Kind::Window => "window",
                    Kind::Screen => "screen",
                    Kind::Image => "image",
                };
                return Some(format!(
                    "{id} · {kind} {}{}{}",
                    layer.source.name,
                    filtered(layer.shader.as_deref()),
                    hidden(layer.visible)
                ));
            }
            let element = scene.elements.iter().find(|e| e.id == id)?;
            let what = match &element.content {
                ElementContent::Text { text } => format!("text \"{text}\""),
                ElementContent::Timer { seconds } => format!("timer {seconds} s"),
            };
            Some(format!(
                "{id} · {what}{}{}",
                filtered(element.shader.as_deref()),
                hidden(element.visible)
            ))
        })
        .collect()
}

/// The sound panel: the microphone and its gate, the gate's two levels against their
/// thresholds, the music and the mix, each a bar `width` cells wide.
fn sound(
    status: &Status,
    levels: Option<&(Hearing, Mixing)>,
    meters: &Meters,
    picked: Option<usize>,
    genre: Option<&str>,
    width: usize,
) -> Vec<Line<'static>> {
    // A row's name, reversed when it is the one the keys act on.
    let name = |name: &str, row: usize| {
        let style = if picked == Some(row) {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        Span::styled(format!("{name:<5}"), style)
    };
    let Some((hearing, mixing)) = levels else {
        return vec![Line::from("no levels from the engine")];
    };
    // The bar in its colour over a dim track of the same height; a threshold is a mark
    // across the track.
    let bar = |level_db: f64, mark: Option<f64>, colour: Color| {
        let mark = mark.map(|m| lit(m, width).min(width.saturating_sub(1)));
        let drawn = cells(level_db, width);
        let on = drawn.chars().take_while(|c| *c != ' ').count();
        let dim = Style::new().fg(Color::Indexed(238));
        let mut spans = vec![Span::styled(
            drawn.chars().take(on).collect::<String>(),
            Style::new().fg(colour),
        )];
        match mark.filter(|m| *m >= on) {
            Some(m) => spans.extend([
                Span::styled("▄".repeat(m - on), dim),
                Span::styled("│", Style::new().fg(Color::Gray)),
                Span::styled("▄".repeat(width - m - 1), dim),
            ]),
            None => spans.push(Span::styled("▄".repeat(width - on), dim)),
        }
        spans
    };
    let row = |name: &str, bar: Vec<Span<'static>>, reading: String| {
        let mut spans = vec![Span::raw(format!("{name:<6} "))];
        spans.extend(bar);
        spans.push(Span::raw(format!(" {reading}")));
        Line::from(spans)
    };
    let mic = status.mic.is_some();
    let word = signal_word(
        gate_word(hearing.gate_open, status.muted, mic),
        meters.has_signal,
    );
    let lamp = match word {
        "open" => Color::Green,
        "closed" => Color::Red,
        "no signal" => Color::Yellow,
        _ => Color::DarkGray,
    };
    let heard = meters.level;
    vec![
        Line::from(vec![
            name("mic", 0),
            Span::raw(format!(
                "  {} · {:.0}% · ",
                status.mic.as_deref().unwrap_or("none"),
                status.faders.mic * 100.0
            )),
            Span::styled(
                format!("gate {word}"),
                Style::new().fg(lamp).add_modifier(Modifier::BOLD),
            ),
        ]),
        row(
            "level",
            bar(heard, None, Color::Green),
            format!("{heard:.0} dB"),
        ),
        row(
            "voice",
            bar(meters.voice, Some(db(status.gate.full)), Color::Cyan),
            String::new(),
        ),
        row(
            "highs",
            bar(meters.highs, Some(db(status.gate.hf)), Color::Cyan),
            String::new(),
        ),
        Line::from(""),
        Line::from(vec![
            name("music", 1),
            Span::raw(format!(
                "  {}{} · {:.1}% · duck {} · {} · {}",
                status.music.as_deref().unwrap_or("nothing playing"),
                genre.map(|g| format!(" · {g}")).unwrap_or_default(),
                status.faders.music * 100.0,
                if status.faders.duck_db > -0.5 {
                    "off".to_string()
                } else {
                    format!("{:.0} dB", status.faders.duck_db)
                },
                if mixing.playing { "playing" } else { "paused" },
                if status.music_to_stream {
                    "to the live"
                } else {
                    "off the live"
                },
            )),
        ]),
        row(
            "level",
            bar(meters.music, None, Color::Magenta),
            format!("{:.0} dB", meters.music),
        ),
        row(
            "mix",
            bar(meters.mix, None, Color::Yellow),
            format!("{:.0} dB", meters.mix),
        ),
    ]
    .into_iter()
    .chain((!status.audio_layers.is_empty()).then(|| Line::from("")))
    .chain(audio_rows(status).into_iter().enumerate().map(|(i, line)| {
        let muted = status.audio_layers[i].muted;
        let style = if muted {
            Style::new().fg(Color::DarkGray)
        } else {
            Style::new()
        };
        Line::from(vec![
            name("audio", 2 + i),
            Span::styled(format!("  {line}"), style),
        ])
    }))
    .collect()
}

/// The audio layers, one line each. The engine meters none of them, so a line says what
/// the layer hears and how it is set, never a level.
pub fn audio_rows(status: &Status) -> Vec<String> {
    use remuxd_domain::sound::audio_layers::{Duck, Kind};
    status
        .audio_layers
        .iter()
        .map(|layer| {
            let source = &layer.source;
            let heard = match source.kind {
                Kind::Mic => format!("mic {}", source.device.as_deref().unwrap_or("")),
                Kind::App => format!("app {}", source.name.as_deref().unwrap_or("")),
                Kind::Screen => format!("screen {}", source.display.unwrap_or_default()),
            };
            let on = if layer.muted { "muted" } else { "on" };
            let duck = match layer.duck {
                Duck::ByKind => "auto",
                Duck::On => "on",
                Duck::Off => "off",
            };
            format!(
                "{} · {heard} · {:.0}% · {on} · duck {duck}",
                layer.id,
                layer.volume * 100.0
            )
        })
        .collect()
}

/// What each bar shows: the engine's reading, risen to at once and fallen from slowly,
/// as a meter's needle does.
#[derive(Debug)]
pub struct Meters {
    pub level: f64,
    pub voice: f64,
    pub highs: f64,
    pub music: f64,
    pub mix: f64,
    /// Whether the microphone's samples still move.
    pub signal: Signal,
    pub has_signal: bool,
    clock: f64,
}

/// Whether a device still delivers: its count of samples moving. A wireless
/// microphone that drops and comes back stops the count and starts it again.
#[derive(Debug, Default)]
pub struct Signal {
    last: u64,
    since: f64,
    seen: bool,
}

impl Signal {
    /// How long a count may stand still before it is no signal.
    const STILL: f64 = 2.0;

    /// Whether there is a signal, given the count `now` seconds in.
    pub fn heard(&mut self, samples: u64, now: f64) -> bool {
        if !self.seen || samples != self.last {
            self.last = samples;
            self.since = now;
            self.seen = true;
            return true;
        }
        now - self.since < Self::STILL
    }
}

/// The gate's word, or "no signal" for a microphone that is chosen, not muted,
/// and silent at the source.
pub fn signal_word(word: &'static str, signal: bool) -> &'static str {
    match (word, signal) {
        ("open" | "closed", false) => "no signal",
        _ => word,
    }
}

impl Default for Meters {
    fn default() -> Self {
        Meters {
            level: FLOOR_DB,
            voice: FLOOR_DB,
            highs: FLOOR_DB,
            music: FLOOR_DB,
            mix: FLOOR_DB,
            signal: Signal::default(),
            has_signal: true,
            clock: 0.0,
        }
    }
}

impl Meters {
    /// The bars `seconds` after the last reading, given this one.
    pub fn follow(&mut self, hearing: &Hearing, mixing: &Mixing, muted: bool, seconds: f64) {
        let heard = if muted { FLOOR_DB } else { hearing.level_db };
        self.level = fall(self.level, heard, seconds);
        self.voice = fall(self.voice, db(hearing.gate_levels.full), seconds);
        self.highs = fall(self.highs, db(hearing.gate_levels.hf), seconds);
        self.music = fall(self.music, mixing.music_db, seconds);
        self.mix = fall(self.mix, mixing.level_db, seconds);
        self.clock += seconds;
        self.has_signal = self.signal.heard(hearing.samples, self.clock);
    }
}

/// How fast a bar falls. 20 dB a second is the release of a peak meter (IEC 60268-10
/// asks for 20 dB in 1.7 s; a terminal reads better a little quicker).
const FALL_DB_PER_SECOND: f64 = 20.0;

/// A bar's next value: up to what is heard at once, down towards it at the meter's pace.
pub fn fall(shown: f64, heard: f64, seconds: f64) -> f64 {
    heard.max(shown - FALL_DB_PER_SECOND * seconds)
}

/// A bar `width` cells wide, half a cell tall, ending in a quarter block: the upper half
/// of the cell is the space between bars. The track behind it is the caller's.
pub fn cells(level_db: f64, width: usize) -> String {
    let halves = lit(level_db, width * 2);
    let (whole, half) = (halves / 2, halves % 2 == 1);
    let mut bar = "▄".repeat(whole);
    if whole < width {
        bar.push(if half { '▖' } else { ' ' });
        bar.push_str(&" ".repeat(width - whole - 1));
    }
    bar
}

/// The meters' floor: below it a bar is dark.
const FLOOR_DB: f64 = -60.0;

/// How many of `width` cells a level in dBFS lights, from the floor to full scale.
pub fn lit(level_db: f64, width: usize) -> usize {
    if level_db.is_nan() {
        return 0;
    }
    let share = (level_db.clamp(FLOOR_DB, 0.0) - FLOOR_DB) / -FLOOR_DB;
    (share * width as f64).round() as usize
}

/// An amplitude (the gate's levels and thresholds) in dBFS, silence on the floor.
pub fn db(amplitude: f64) -> f64 {
    if amplitude <= 0.0 {
        return FLOOR_DB;
    }
    (20.0 * amplitude.log10()).max(FLOOR_DB)
}

/// The gate's lamp: a missing or muted microphone says so before open or closed.
pub fn gate_word(open: bool, muted: bool, mic: bool) -> &'static str {
    match (open, muted, mic) {
        (_, _, false) => "no mic",
        (_, true, _) => "muted",
        (true, _, _) => "open",
        _ => "closed",
    }
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

    fn with_destinations(rows: &[(i64, bool, bool)]) -> Status {
        Status {
            destinations: rows
                .iter()
                .map(|(id, armed, sandbox)| {
                    serde_json::from_value(serde_json::json!({
                        "id": id, "name": format!("d{id}"), "platform": "twitch",
                        "status": "off", "armed": armed, "sandbox": sandbox
                    }))
                    .expect("a destination")
                })
                .collect(),
            ..Status::default()
        }
    }

    #[test]
    fn tab_moves_to_the_destinations_and_j_k_move_there_alone() {
        let status = with_destinations(&[(1, true, false), (2, false, false)]);
        let mut screen = Screen {
            focus: Panel::Layers,
            ..Screen::default()
        };
        press('\t', &mut screen, 3, &status);
        assert_eq!(screen.focus, Panel::Destinations);
        press('j', &mut screen, 3, &status);
        assert_eq!(screen.picked_destination, 1);
        assert_eq!(screen.picked, 0, "the scenes keep their own pick");
        press('j', &mut screen, 3, &status);
        assert_eq!(
            screen.picked_destination, 1,
            "two destinations, so the second is the last"
        );
        press('\t', &mut screen, 3, &status);
        assert_eq!(screen.focus, Panel::Sound);
        press('\t', &mut screen, 3, &status);
        assert_eq!(screen.focus, Panel::Chat);
        press('\t', &mut screen, 3, &status);
        assert_eq!(screen.focus, Panel::Companions);
        press('\t', &mut screen, 3, &status);
        assert_eq!(screen.focus, Panel::Log);
        press('\t', &mut screen, 3, &status);
        assert_eq!(screen.focus, Panel::Scenes);
    }

    #[test]
    fn a_arms_what_is_not_armed_and_disarms_what_is() {
        let status = with_destinations(&[(1, true, false), (2, false, false)]);
        let mut screen = Screen {
            focus: Panel::Destinations,
            ..Screen::default()
        };
        assert_eq!(
            press('a', &mut screen, 0, &status),
            Act::Send(Command::Arm {
                adapter: 1,
                on: false
            })
        );
        screen.picked_destination = 1;
        assert_eq!(
            press('a', &mut screen, 0, &status),
            Act::Send(Command::Arm {
                adapter: 2,
                on: true
            })
        );
    }

    #[test]
    fn s_on_a_destination_flips_its_sandbox() {
        let status = with_destinations(&[(7, true, false)]);
        let mut screen = Screen {
            focus: Panel::Destinations,
            ..Screen::default()
        };
        assert_eq!(
            press('s', &mut screen, 0, &status),
            Act::Send(Command::Sandbox {
                adapter: 7,
                on: true
            })
        );
    }

    #[test]
    fn a_does_nothing_while_the_scenes_have_the_focus() {
        let status = with_destinations(&[(1, false, false)]);
        let mut screen = Screen::default();
        assert_eq!(press('a', &mut screen, 0, &status), Act::Stay);
    }

    #[test]
    fn a_scene_reads_back_to_front_with_what_each_layer_is_and_what_is_hidden() {
        let layer = |id: &str, kind: &str, name: &str, visible: bool| {
            serde_json::json!({
                "id": id, "visible": visible,
                "source": { "kind": kind, "handle": "h", "name": name, "width": 1920, "height": 1080 },
                "transform": { "x": 0, "y": 0, "width": 1920, "height": 1080, "degrees": 0 }
            })
        };
        let scene: remuxd_domain::picture::scenes::Scene = serde_json::from_value(serde_json::json!({
            "name": "Starting soon",
            "layers": [layer("bg", "image", "background.png", true), layer("face", "camera", "HP", false)],
            "elements": [
                { "id": "title", "x": 0, "y": 0, "width": 10, "height": 10, "kind": "text", "text": "Hi" },
                { "id": "clock", "x": 0, "y": 0, "width": 10, "height": 10, "kind": "timer", "seconds": 180 }
            ],
            "order": ["bg", "title", "face", "clock"]
        }))
        .expect("a scene");
        assert_eq!(
            rows_of(&scene),
            vec![
                "bg · image background.png",
                "title · text \"Hi\"",
                "face · camera HP (hidden)",
                "clock · timer 180 s",
            ]
        );
    }

    #[test]
    fn a_level_lights_its_share_of_the_bar_from_minus_sixty_to_zero() {
        assert_eq!(lit(-60.0, 20), 0);
        assert_eq!(lit(-30.0, 20), 10);
        assert_eq!(lit(0.0, 20), 20);
        assert_eq!(lit(-90.0, 20), 0, "below the floor is nothing");
        assert_eq!(lit(6.0, 20), 20, "above full scale is the whole bar");
        assert_eq!(lit(f64::NAN, 20), 0);
    }

    #[test]
    fn the_gate_s_thresholds_are_amplitudes_drawn_in_db() {
        assert!((db(0.1) + 20.0).abs() < 1e-9);
        assert_eq!(db(0.0), -60.0, "silence sits on the floor");
    }

    #[test]
    fn the_gate_says_muted_over_open_and_no_mic_over_everything() {
        assert_eq!(gate_word(true, false, false), "no mic");
        assert_eq!(
            gate_word(true, true, true),
            "muted",
            "a muted mic is not live, open or not"
        );
        assert_eq!(gate_word(true, false, true), "open");
        assert_eq!(gate_word(false, false, true), "closed");
    }

    #[test]
    fn a_bar_is_half_a_cell_tall_and_ends_in_a_quarter_block() {
        assert_eq!(cells(-60.0, 4), "    ");
        assert_eq!(cells(0.0, 4), "▄▄▄▄");
        assert_eq!(cells(-30.0, 4), "▄▄  ");
        // -22.5 dB is 2.5 cells of 4: two whole and a half.
        assert_eq!(cells(-22.5, 4), "▄▄▖ ");
    }

    #[test]
    fn a_meter_rises_at_once_and_falls_at_twenty_db_a_second() {
        assert_eq!(
            fall(-40.0, -10.0, 0.05),
            -10.0,
            "a louder reading is shown at once"
        );
        assert_eq!(
            fall(-10.0, -50.0, 0.05),
            -11.0,
            "a quieter one is reached by falling"
        );
        assert_eq!(fall(-10.0, -10.5, 0.05), -10.5, "never below what is heard");
    }

    /// "Screen" on the air with `desk` shown and `face` hidden, back to front, and
    /// "BRB" beside it.
    fn on_screen() -> Status {
        let layer = |id: &str, visible: bool| {
            serde_json::json!({
                "id": id, "visible": visible,
                "source": { "kind": "screen", "handle": "1", "name": "d", "width": 1920, "height": 1080 },
                "transform": { "x": 0, "y": 0, "width": 1920, "height": 1080, "degrees": 0 }
            })
        };
        Status {
            scenes: vec![
                serde_json::from_value(serde_json::json!({
                    "name": "Screen", "layers": [layer("desk", true), layer("face", false)]
                }))
                .expect("a scene"),
                serde_json::from_value(
                    serde_json::json!({ "name": "BRB", "layers": [layer("desk", true)] }),
                )
                .expect("a scene"),
            ],
            active_scene: "Screen".into(),
            ..Status::default()
        }
    }

    #[test]
    fn tab_goes_from_the_scenes_to_their_layers_to_the_destinations() {
        let mut screen = Screen::default();
        press('\t', &mut screen, 2, &on_screen());
        assert_eq!(screen.focus, Panel::Layers);
        press('\t', &mut screen, 2, &on_screen());
        assert_eq!(screen.focus, Panel::Destinations);
    }

    #[test]
    fn space_hides_a_shown_layer_and_shows_a_hidden_one() {
        let mut screen = Screen {
            focus: Panel::Layers,
            ..Screen::default()
        };
        assert_eq!(
            press(' ', &mut screen, 2, &on_screen()),
            Act::Send(Command::LayerVisible {
                id: "desk".into(),
                on: false
            })
        );
        press('j', &mut screen, 2, &on_screen());
        assert_eq!(
            press(' ', &mut screen, 2, &on_screen()),
            Act::Send(Command::LayerVisible {
                id: "face".into(),
                on: true
            })
        );
    }

    #[test]
    fn shift_j_brings_a_layer_forward_and_shift_k_sends_it_back_the_pick_following() {
        let mut screen = Screen {
            focus: Panel::Layers,
            ..Screen::default()
        };
        assert_eq!(
            press('J', &mut screen, 2, &on_screen()),
            Act::Send(Command::LayerMove {
                id: "desk".into(),
                index: 1
            })
        );
        assert_eq!(screen.picked_layer, 1);
        assert_eq!(
            press('J', &mut screen, 2, &on_screen()),
            Act::Stay,
            "already in front"
        );
        assert_eq!(
            press('K', &mut screen, 2, &on_screen()),
            Act::Send(Command::LayerMove {
                id: "face".into(),
                index: 0
            })
        );
        assert_eq!(
            press('K', &mut screen, 2, &on_screen()),
            Act::Stay,
            "already at the back"
        );
    }

    #[test]
    fn a_layer_of_a_scene_neither_on_the_air_nor_staged_is_only_read() {
        // `desk` is in both scenes: acting on BRB's would act on the air's.
        let mut screen = Screen {
            focus: Panel::Layers,
            picked: 1,
            ..Screen::default()
        };
        assert_eq!(
            press(' ', &mut screen, 2, &on_screen()),
            Act::Say("enter stages this scene first")
        );
        assert_eq!(
            press('J', &mut screen, 2, &on_screen()),
            Act::Say("enter stages this scene first")
        );
    }

    fn music_at(level: f64) -> Status {
        let mut status = Status::default();
        status.faders.music = level;
        status
    }

    #[test]
    fn a_layer_of_the_staged_scene_is_acted_on_in_the_preview() {
        let mut status = on_screen();
        status.staged = Some("BRB".into());
        let mut screen = Screen {
            focus: Panel::Layers,
            picked: 1,
            ..Screen::default()
        };
        assert_eq!(
            press(' ', &mut screen, 2, &status),
            Act::Send(Command::Staged {
                command: Box::new(Command::LayerVisible {
                    id: "desk".into(),
                    on: false
                })
            })
        );
    }

    #[test]
    fn m_mutes_the_mic_and_unmutes_it_from_any_panel_at_once() {
        let mut screen = Screen {
            focus: Panel::Destinations,
            ..Screen::default()
        };
        assert_eq!(
            press('m', &mut screen, 0, &Status::default()),
            Act::Send(Command::Mute { on: true })
        );
        let muted = Status {
            muted: true,
            ..Status::default()
        };
        assert_eq!(
            press('m', &mut screen, 0, &muted),
            Act::Send(Command::Mute { on: false })
        );
    }

    #[test]
    fn n_is_the_next_track() {
        assert_eq!(
            press('n', &mut Screen::default(), 0, &Status::default()),
            Act::Send(Command::NextTrack)
        );
    }

    #[test]
    fn plus_and_minus_move_the_music_three_db_at_a_time() {
        let level = |key| match press(key, &mut Screen::default(), 0, &music_at(0.01)) {
            Act::Send(Command::MusicVolume { level }) => level,
            other => panic!("{other:?}"),
        };
        assert!((level('+') - 0.01 * 10f64.powf(0.15)).abs() < 1e-9);
        assert!((level('-') - 0.01 / 10f64.powf(0.15)).abs() < 1e-9);
    }

    #[test]
    fn the_music_fader_stops_at_full_and_climbs_out_of_silence() {
        let level = |key, at| match press(key, &mut Screen::default(), 0, &music_at(at)) {
            Act::Send(Command::MusicVolume { level }) => level,
            other => panic!("{other:?}"),
        };
        assert_eq!(level('+', 0.9), 1.0);
        assert_eq!(
            level('+', 0.0),
            0.001,
            "silence has no three db louder: a step out of it"
        );
        assert_eq!(
            level('-', 0.001),
            0.0,
            "below a tenth of a percent is silence"
        );
    }

    #[test]
    fn c_opens_a_line_where_every_key_is_a_letter_and_enter_says_it() {
        let mut screen = Screen {
            focus: Panel::Chat,
            ..Screen::default()
        };
        let status = Status::default();
        assert_eq!(press('c', &mut screen, 0, &status), Act::Stay);
        for key in "oi q\u{8}m".chars() {
            assert_eq!(
                press(key, &mut screen, 0, &status),
                Act::Stay,
                "{key:?} is typed, not done"
            );
        }
        assert_eq!(screen.typing.as_deref(), Some("oi m"));
        assert_eq!(
            press('\n', &mut screen, 0, &status),
            Act::Send(Command::Say {
                body: "oi m".into(),
                channel: None
            })
        );
        assert_eq!(screen.typing, None);
    }

    #[test]
    fn escape_lets_a_line_go_unsaid_and_an_empty_one_is_not_sent() {
        let mut screen = Screen {
            focus: Panel::Chat,
            ..Screen::default()
        };
        let status = Status::default();
        press('c', &mut screen, 0, &status);
        press('x', &mut screen, 0, &status);
        assert_eq!(press('\u{1b}', &mut screen, 0, &status), Act::Stay);
        assert_eq!(screen.typing, None);
        press('c', &mut screen, 0, &status);
        assert_eq!(press('\n', &mut screen, 0, &status), Act::Stay);
        assert_eq!(screen.typing, None);
    }

    #[test]
    fn c_says_in_the_chat_from_the_chat_panel_alone() {
        let mut screen = Screen::default();
        press('c', &mut screen, 0, &Status::default());
        assert_eq!(screen.typing, None, "away from the chat, c opens nothing");
        assert!(
            !GLOBAL_KEYS.iter().any(|k| k.contains("[c]")),
            "c is the chat panel's key"
        );
    }

    #[test]
    fn escape_outside_a_line_still_quits() {
        assert_eq!(
            press('\u{1b}', &mut Screen::default(), 0, &Status::default()),
            Act::Quit
        );
    }

    /// The mic at 100%, the music at 1% with no duck, and a guest on a call app at 50%.
    fn sounding() -> Status {
        let mut status = music_at(0.01);
        status.faders.mic = 1.0;
        status.audio_layers = serde_json::from_value(serde_json::json!([
            { "id": "guest", "source": { "kind": "app", "name": "Discord" },
              "volume": 0.5, "muted": false, "duck": "by-kind" }
        ]))
        .expect("audio layers");
        status
    }

    fn on_sound(row: usize) -> Screen {
        Screen {
            focus: Panel::Sound,
            picked_sound: row,
            ..Screen::default()
        }
    }

    #[test]
    fn the_sound_rows_are_the_mic_the_music_and_each_audio_layer() {
        let mut screen = on_sound(0);
        press('j', &mut screen, 0, &sounding());
        press('j', &mut screen, 0, &sounding());
        assert_eq!(screen.picked_sound, 2);
        press('j', &mut screen, 0, &sounding());
        assert_eq!(screen.picked_sound, 2, "the guest is the last row");
    }

    #[test]
    fn space_turns_the_picked_sound_off_and_on() {
        let status = sounding();
        assert_eq!(
            press(' ', &mut on_sound(0), 0, &status),
            Act::Send(Command::Mute { on: true })
        );
        let mut playing = Screen {
            playing: true,
            ..on_sound(1)
        };
        assert_eq!(
            press(' ', &mut playing, 0, &status),
            Act::Send(Command::Music { on: false }),
            "the music off is the music silent, not only off the live"
        );
        assert_eq!(
            press(' ', &mut on_sound(2), 0, &status),
            Act::Send(Command::AudioLayerMute {
                id: "guest".into(),
                on: true
            })
        );
    }

    #[test]
    fn s_on_the_music_says_whether_the_live_hears_it() {
        let mut status = sounding();
        status.music_to_stream = true;
        assert_eq!(
            press('s', &mut on_sound(1), 0, &status),
            Act::Send(Command::StreamMusic { on: false })
        );
        assert_eq!(
            press('s', &mut on_sound(0), 0, &status),
            Act::Stay,
            "only the music goes to the live or not"
        );
    }

    #[test]
    fn plus_on_the_sound_panel_moves_the_picked_fader_up_to_its_own_ceiling() {
        let mut status = sounding();
        status.faders.mic = 1.9;
        assert_eq!(
            press('+', &mut on_sound(0), 0, &status),
            Act::Send(Command::Volume { level: 2.0 })
        );
        let Act::Send(Command::AudioLayerVolume { id, volume }) =
            press('-', &mut on_sound(2), 0, &status)
        else {
            panic!("a layer's fader")
        };
        assert_eq!(id, "guest");
        assert!((volume - 0.5 / 10f64.powf(0.15)).abs() < 1e-9);
    }

    #[test]
    fn d_steps_the_music_s_duck_six_db_at_a_time_and_back_to_off() {
        let mut status = sounding();
        let duck = |status: &Status| match press('d', &mut on_sound(1), 0, status) {
            Act::Send(Command::Duck { db }) => db,
            other => panic!("{other:?}"),
        };
        for (now, next) in [(-0.0, -6.0), (-6.0, -12.0), (-12.0, -18.0), (-18.0, 0.0)] {
            status.faders.duck_db = now;
            assert_eq!(duck(&status), next);
        }
    }

    #[test]
    fn d_on_an_audio_layer_goes_auto_off_on_and_round() {
        use remuxd_domain::sound::audio_layers::Duck;
        let mut status = sounding();
        for (now, next) in [
            (Duck::ByKind, Duck::Off),
            (Duck::Off, Duck::On),
            (Duck::On, Duck::ByKind),
        ] {
            status.audio_layers[0].duck = now;
            assert_eq!(
                press('d', &mut on_sound(2), 0, &status),
                Act::Send(Command::AudioLayerDuck {
                    id: "guest".into(),
                    duck: next
                })
            );
        }
    }

    #[test]
    fn p_pauses_the_music_playing_and_plays_it_paused_from_any_panel() {
        let mut screen = Screen {
            playing: true,
            ..Screen::default()
        };
        assert_eq!(
            press('p', &mut screen, 0, &Status::default()),
            Act::Send(Command::Music { on: false })
        );
        screen.playing = false;
        assert_eq!(
            press('p', &mut screen, 0, &Status::default()),
            Act::Send(Command::Music { on: true })
        );
    }

    #[test]
    fn a_long_line_of_chat_breaks_between_words_and_its_rest_is_indented() {
        assert_eq!(
            wrapped("ana: com certeza deve ter chess de terminal", 20, "  "),
            vec!["ana: com certeza", "  deve ter chess de", "  terminal"]
        );
        assert_eq!(wrapped("curta", 20, "  "), vec!["curta"]);
    }

    #[test]
    fn a_word_longer_than_the_column_is_cut_where_the_column_ends() {
        assert_eq!(
            wrapped("KKKKKKKKKKKK", 5, "  "),
            vec!["KKKKK", "  KKK", "  KKK", "  K"]
        );
    }

    #[test]
    fn the_outgoing_line_says_the_picture_the_rates_and_the_audience() {
        let mut status = Status {
            outgoing: serde_json::from_value(serde_json::json!({
                "width": 1920, "height": 1080, "fps": 30, "video_kbps": 6182, "audio_kbps": 160
            }))
            .expect("outgoing"),
            ..Status::default()
        };
        assert_eq!(
            out_line(&status),
            "out 1920x1080 · 30 fps · video 6182 kbps · audio 160 kbps · viewers — · peak —"
        );
        status.viewers = Some(42);
        status.viewers_peak = Some(80);
        assert!(out_line(&status).ends_with("viewers 42 · peak 80"));
    }

    fn destination(extra: serde_json::Value) -> remuxd_domain::protocol::Destination {
        let mut row = serde_json::json!({
            "id": 1, "name": "twitch", "platform": "twitch", "status": "live",
            "armed": true, "sandbox": false
        });
        row.as_object_mut()
            .expect("an object")
            .extend(extra.as_object().expect("an object").clone());
        serde_json::from_value(row).expect("a destination")
    }

    #[test]
    fn a_destination_without_an_account_says_its_state_and_that_all_is_well() {
        assert_eq!(
            destination_row(&destination(serde_json::json!({}))),
            "● twitch (twitch) armed live · viewers — · ok"
        );
    }

    #[test]
    fn a_destination_with_an_account_adds_its_audience_trouble_and_category() {
        let row = destination(serde_json::json!({
            "viewers": 12, "account": "someone", "category": "Software and Game Development",
            "trouble": "youtube said 403 quotaExceeded"
        }));
        assert_eq!(
            destination_row(&row),
            "● twitch (twitch) armed live · viewers 12 · youtube said 403 quotaExceeded · \
             someone · Software and Game Development"
        );
    }

    #[test]
    fn the_chat_reads_as_the_cli_s_platform_in_its_colour_the_words_below_a_faint_rule_between() {
        let lines: Vec<ChatLine> = serde_json::from_value(serde_json::json!([
            { "seq": 1, "from": "ana", "body": "ah", "platform": "twitch", "id": "a" },
            { "seq": 2, "from": "ev\u{1b}[31mil", "body": "olá\u{1b}[2J tudo bem por aí",
              "platform": "youtube", "id": "b" }
        ]))
        .expect("chat lines");
        let rows = chat_rows(&lines, 14);
        let text: Vec<String> = rows
            .iter()
            .map(|row| row.spans.iter().map(|span| span.content.as_ref()).collect())
            .collect();
        assert_eq!(
            text,
            vec![
                "twitch  ana",
                "  ah",
                &"─".repeat(14),
                "youtube  evil",
                "  olá tudo bem",
                "  por aí"
            ]
        );
        assert_eq!(
            rows[0].spans[0].style.fg,
            Some(Color::Indexed(141)),
            "Twitch's purple"
        );
        assert_eq!(
            rows[3].spans[0].style.fg,
            Some(Color::Indexed(203)),
            "YouTube's red"
        );
        assert_eq!(
            rows[2].spans[0].style.fg,
            Some(Color::Indexed(238)),
            "the rule is faint"
        );
    }

    #[test]
    fn k_on_the_chat_goes_back_a_line_at_a_time_and_j_comes_forward_to_the_last() {
        let mut screen = Screen {
            focus: Panel::Chat,
            ..Screen::default()
        };
        press('k', &mut screen, 0, &Status::default());
        press('k', &mut screen, 0, &Status::default());
        assert_eq!(screen.chat_back, 2);
        press('j', &mut screen, 0, &Status::default());
        press('j', &mut screen, 0, &Status::default());
        press('j', &mut screen, 0, &Status::default());
        assert_eq!(
            screen.chat_back, 0,
            "the last line is as far forward as it goes"
        );
    }

    #[test]
    fn going_back_stops_at_the_first_line_held() {
        let mut chat = Chat {
            lines: chat_lines(3),
            ..Chat::default()
        };
        let mut screen = Screen {
            chat_back: 9,
            ..Screen::default()
        };
        chat.hold(&mut screen, Vec::new());
        assert_eq!(
            screen.chat_back, 2,
            "three lines: the first is two back from the last"
        );
    }

    #[test]
    fn a_line_arriving_while_reading_back_does_not_move_what_is_read() {
        let mut chat = Chat {
            lines: chat_lines(3),
            ..Chat::default()
        };
        let mut screen = Screen {
            chat_back: 1,
            ..Screen::default()
        };
        chat.hold(&mut screen, chat_lines(5).split_off(3));
        assert_eq!(
            screen.chat_back, 3,
            "two new lines below: still on the same line"
        );
        screen.chat_back = 0;
        chat.hold(&mut screen, chat_lines(6).split_off(5));
        assert_eq!(
            screen.chat_back, 0,
            "at the last line, the newest stays in view"
        );
    }

    fn chat_lines(n: u64) -> Vec<ChatLine> {
        (1..=n)
            .map(|seq| {
                serde_json::from_value(serde_json::json!({
                    "seq": seq, "from": "a", "body": "b", "platform": "twitch", "id": seq.to_string()
                }))
                .expect("a chat line")
            })
            .collect()
    }

    #[test]
    fn a_destination_on_the_air_is_red_and_one_off_it_grey() {
        assert_eq!(
            destination_colour(&destination(serde_json::json!({}))),
            Color::Red
        );
        assert_eq!(
            destination_colour(&destination(serde_json::json!({ "status": "off" }))),
            Color::DarkGray
        );
    }

    #[test]
    fn enter_stages_the_picked_scene_and_the_air_does_not_move() {
        let status = with_scenes(&["Screen", "Starting soon", "BRB"], "Screen");
        let mut screen = Screen {
            picked: 2,
            ..Screen::default()
        };
        assert_eq!(
            press('\n', &mut screen, 3, &status),
            Act::Send(Command::SceneStage { name: "BRB".into() })
        );
    }

    #[test]
    fn enter_on_the_scene_on_the_air_lets_the_staged_one_go_or_does_nothing() {
        let mut status = with_scenes(&["Screen", "BRB"], "Screen");
        assert_eq!(press('\n', &mut Screen::default(), 2, &status), Act::Stay);
        status.staged = Some("BRB".into());
        assert_eq!(
            press('\n', &mut Screen::default(), 2, &status),
            Act::Send(Command::SceneStage {
                name: "Screen".into()
            })
        );
    }

    #[test]
    fn the_top_says_the_scene_on_the_air_and_the_one_in_preview() {
        let mut status = with_scenes(&["Screen", "BRB"], "Screen");
        assert_eq!(scene_line(&status), "scene: Screen");
        status.staged = Some("BRB".into());
        assert_eq!(scene_line(&status), "scene: Screen · preview: BRB");
    }

    use remuxd_domain::companions::Record;

    fn on_companions(picked: usize) -> Screen {
        Screen {
            focus: Panel::Companions,
            picked_companion: picked,
            companions: vec![
                ("first".into(), State::Up(Record { pid: 7, since: 0 })),
                ("second".into(), State::Down),
            ],
            ..Screen::default()
        }
    }

    #[test]
    fn space_stops_a_companion_that_is_up_and_starts_one_that_is_not() {
        assert_eq!(
            press(' ', &mut on_companions(0), 0, &Status::default()),
            Act::Companion {
                name: "first".into(),
                start: false
            }
        );
        assert_eq!(
            press(' ', &mut on_companions(1), 0, &Status::default()),
            Act::Companion {
                name: "second".into(),
                start: true
            }
        );
    }

    #[test]
    fn e_opens_a_line_of_words_for_the_picked_companion_and_enter_sends_them() {
        let mut screen = on_companions(0);
        assert_eq!(press('e', &mut screen, 0, &Status::default()), Act::Stay);
        assert_eq!(screen.typing.as_deref(), Some(""));
        for key in "dvd".chars() {
            press(key, &mut screen, 0, &Status::default());
        }
        assert_eq!(
            press('\n', &mut screen, 0, &Status::default()),
            Act::CompanionSend {
                name: "first".into(),
                words: "dvd".into()
            }
        );
    }

    #[test]
    fn j_and_k_move_among_the_companions_and_stop_at_the_ends() {
        let mut screen = on_companions(0);
        press('j', &mut screen, 0, &Status::default());
        press('j', &mut screen, 0, &Status::default());
        assert_eq!(screen.picked_companion, 1);
        press('k', &mut screen, 0, &Status::default());
        press('k', &mut screen, 0, &Status::default());
        assert_eq!(screen.picked_companion, 0);
    }

    #[test]
    fn a_companion_reads_as_a_lamp_its_name_and_its_state() {
        let record = Record { pid: 7, since: 0 };
        assert_eq!(companion_row("first", State::Up(record)), "● first  up");
        assert_eq!(
            companion_row("first", State::Fell(record)),
            "✕ first  fell: remux companion log first"
        );
        assert_eq!(companion_row("first", State::Down), "○ first  down");
    }

    fn typed(screen: &mut Screen, status: &Status, words: &str) -> Act {
        for key in words.chars() {
            assert_eq!(press(key, screen, 3, status), Act::Stay, "{key:?} is typed");
        }
        press('\n', screen, 3, status)
    }

    #[test]
    fn n_names_a_new_scene_that_is_drafted_in_the_preview() {
        let status = with_scenes(&["Screen", "BRB"], "Screen");
        let mut screen = Screen::default();
        assert_eq!(press('N', &mut screen, 2, &status), Act::Stay);
        assert_eq!(
            typed(&mut screen, &status, "Keys"),
            Act::Send(Command::SceneDraft {
                name: "Keys".into(),
                from: None
            })
        );
    }

    #[test]
    fn d_drafts_a_copy_of_the_picked_scene_under_a_new_name() {
        let status = with_scenes(&["Screen", "BRB"], "Screen");
        let mut screen = Screen {
            picked: 1,
            ..Screen::default()
        };
        press('D', &mut screen, 2, &status);
        assert_eq!(
            typed(&mut screen, &status, "BRB 2"),
            Act::Send(Command::SceneDraft {
                name: "BRB 2".into(),
                from: Some("BRB".into())
            })
        );
    }

    #[test]
    fn question_mark_opens_every_key_and_any_key_closes_it() {
        let mut screen = Screen::default();
        press('?', &mut screen, 0, &Status::default());
        assert!(screen.showing_keys);
        assert_eq!(
            press('q', &mut screen, 0, &Status::default()),
            Act::Stay,
            "q closes it, it does not quit"
        );
        assert!(!screen.showing_keys);
    }

    #[test]
    fn a_digit_goes_to_its_panel_and_brings_it_back_when_hidden() {
        let mut screen = Screen::default();
        press('5', &mut screen, 0, &Status::default());
        assert_eq!(screen.focus, Panel::Chat);
        screen.hidden.push(Panel::Sound);
        press('4', &mut screen, 0, &Status::default());
        assert_eq!(screen.focus, Panel::Sound);
        assert!(
            screen.shows(Panel::Sound),
            "going to a hidden panel shows it"
        );
    }

    #[test]
    fn h_hides_the_panel_in_focus_and_the_focus_moves_on() {
        let mut screen = Screen::default();
        press('h', &mut screen, 0, &Status::default());
        assert!(!screen.shows(Panel::Scenes));
        assert_eq!(screen.focus, Panel::Layers);
    }

    #[test]
    fn zero_shows_every_panel_and_hides_none() {
        let mut screen = Screen {
            hidden: vec![Panel::Chat, Panel::Log],
            ..Screen::default()
        };
        press('0', &mut screen, 0, &Status::default());
        assert!(screen.hidden.is_empty());
        press('0', &mut screen, 0, &Status::default());
        assert!(screen.hidden.is_empty(), "zero only shows");
    }

    #[test]
    fn shift_tab_goes_back_and_both_tabs_skip_what_is_hidden() {
        let mut screen = Screen {
            hidden: vec![Panel::Layers],
            ..Screen::default()
        };
        press('\u{19}', &mut screen, 0, &Status::default());
        assert_eq!(screen.focus, Panel::Log, "back from the first is the last");
        press('\u{19}', &mut screen, 0, &Status::default());
        assert_eq!(screen.focus, Panel::Companions);
        screen.focus = Panel::Scenes;
        press('\t', &mut screen, 0, &Status::default());
        assert_eq!(
            screen.focus,
            Panel::Destinations,
            "layers hidden: tab goes past it"
        );
    }

    #[test]
    fn the_last_panel_shown_stays() {
        let mut screen = Screen {
            focus: Panel::Log,
            hidden: Panel::ALL[..6].to_vec(),
            ..Screen::default()
        };
        assert_eq!(
            press('h', &mut screen, 0, &Status::default()),
            Act::Say("one panel stays shown")
        );
        assert!(screen.shows(Panel::Log));
    }

    #[test]
    fn the_key_bar_is_two_lines_the_panel_s_and_the_screen_s_whatever_the_panel() {
        for panel in Panel::ALL {
            let [own, global] = key_lines(panel);
            assert_eq!(own, keys_of(panel));
            assert_eq!(global, GLOBAL_KEYS);
        }
        assert!(
            GLOBAL_KEYS.iter().any(|k| k.contains("[0]"))
                && GLOBAL_KEYS.iter().any(|k| k.contains("[1-7]"))
        );
        assert!(Panel::ALL
            .iter()
            .all(|p| keys_of(*p).iter().any(|k| k.contains("[h] hide"))));
    }

    #[test]
    fn the_panels_hidden_are_kept_as_words_and_read_back() {
        let hidden = vec![Panel::Chat, Panel::Log];
        let kept = keep_hidden(&hidden);
        assert_eq!(kept, "hidden = [\"chat\", \"log\"]\n");
        assert_eq!(read_hidden(&kept), hidden);
        assert_eq!(
            read_hidden("hidden = [\"nonsense\"]\n"),
            Vec::<Panel>::new()
        );
        assert_eq!(read_hidden(""), Vec::<Panel>::new());
    }

    #[test]
    fn the_key_bar_wraps_its_tokens_onto_as_many_lines_as_it_needs() {
        let tokens = ["[a] one", "[b] two", "[c] three"];
        assert_eq!(
            wrap_tokens(&tokens, 40),
            vec![vec!["[a] one", "[b] two", "[c] three"]]
        );
        assert_eq!(
            wrap_tokens(&tokens, 16),
            vec![vec!["[a] one", "[b] two"], vec!["[c] three"]]
        );
    }

    #[test]
    fn only_the_journal_s_lines_after_the_last_one_seen_are_new() {
        let lines: Vec<String> = ["12:00:01 a", "12:00:02 b", "12:00:03 c"]
            .map(String::from)
            .to_vec();
        assert_eq!(fresh_journal(Some("12:00:02 b"), &lines), &lines[2..]);
        assert_eq!(fresh_journal(None, &lines), &lines[..]);
        assert_eq!(
            fresh_journal(Some("gone"), &lines),
            &lines[..],
            "the last seen fell off: all of it"
        );
    }

    #[test]
    fn the_log_takes_the_events_no_command_made_and_leaves_the_rest_to_the_journal() {
        use remuxd_domain::app::events::{Event, Numbered};
        let at = 1_790_000_000;
        let events = vec![
            Numbered {
                seq: 1,
                at,
                event: Event::SceneSwitched { name: "BRB".into() },
            },
            Numbered {
                seq: 2,
                at,
                event: Event::LayerStalled { id: "face".into() },
            },
        ];
        assert_eq!(
            events_for_log(&events),
            vec![format!(
                "{} camera face stopped delivering",
                remuxd_domain::air::journal::clock_of(at)
            )]
        );
    }

    #[test]
    fn a_companion_that_changes_state_is_a_line_and_one_that_fell_says_where_to_look() {
        let record = Record { pid: 7, since: 0 };
        let before = vec![
            ("a".to_string(), State::Up(record)),
            ("b".to_string(), State::Down),
        ];
        let after = vec![
            ("a".to_string(), State::Fell(record)),
            ("b".to_string(), State::Up(record)),
        ];
        assert_eq!(
            companion_changes(&before, &after),
            vec![
                "companion a fell: remux companion log a".to_string(),
                "companion b up".to_string()
            ]
        );
        assert!(companion_changes(&after, &after).is_empty());
    }

    #[test]
    fn a_line_that_went_wrong_is_red_a_notice_yellow_the_rest_plain() {
        assert_eq!(
            log_colour("12:00:01 ! scene: no scene \"x\""),
            Some(Color::Red)
        );
        assert_eq!(
            log_colour("12:00:01 destination 2 dropped: timeout"),
            Some(Color::Red)
        );
        assert_eq!(
            log_colour("12:00:01 camera face stopped delivering"),
            Some(Color::Red)
        );
        assert_eq!(
            log_colour("12:00:01 companion chess fell: remux companion log chess"),
            Some(Color::Red)
        );
        assert_eq!(
            log_colour("12:00:01 notice: told the title"),
            Some(Color::Yellow)
        );
        assert_eq!(log_colour("12:00:01 scene BRB staged"), None);
    }

    #[test]
    fn k_reads_the_log_back_and_j_comes_forward() {
        let mut screen = Screen {
            focus: Panel::Log,
            ..Screen::default()
        };
        press('k', &mut screen, 0, &Status::default());
        press('k', &mut screen, 0, &Status::default());
        assert_eq!(screen.log_back, 2);
        press('j', &mut screen, 0, &Status::default());
        assert_eq!(screen.log_back, 1);
    }

    fn devices() -> remuxd_domain::protocol::Devices {
        let named = |id: &str, name: &str| remuxd_domain::protocol::Named {
            id: id.into(),
            name: name.into(),
        };
        remuxd_domain::protocol::Devices {
            cameras: vec![named("0x12", "C270 HD WEBCAM")],
            screens: vec![named("1", "VG2791R")],
            windows: vec![named("95", "Notes — today")],
            mics: vec![named("bt", "Razer BlackShark V2 Pro (BT)")],
            ..Default::default()
        }
    }

    #[test]
    fn enter_on_the_mic_asks_for_the_microphones_and_a_pick_chooses_one() {
        let mut screen = on_sound(0);
        assert_eq!(press('\n', &mut screen, 0, &sounding()), Act::AskMics);
        screen.picker = Some(mic_picker(&devices()));
        let rows: Vec<&str> = screen
            .picker
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .map(|(l, _)| l.as_str())
            .collect();
        assert_eq!(rows, ["no microphone", "Razer BlackShark V2 Pro (BT)"]);
        press('j', &mut screen, 0, &sounding());
        assert_eq!(
            press('\n', &mut screen, 0, &sounding()),
            Act::Send(Command::Mic {
                device: Some("Razer BlackShark V2 Pro (BT)".into())
            })
        );
        assert!(screen.picker.is_none());
    }

    #[test]
    fn escape_closes_a_picker_and_sends_nothing() {
        let mut screen = Screen {
            picker: Some(mic_picker(&devices())),
            ..Screen::default()
        };
        assert_eq!(
            press('\u{1b}', &mut screen, 0, &Status::default()),
            Act::Stay
        );
        assert!(screen.picker.is_none());
    }

    #[test]
    fn a_click_lands_on_the_panel_drawn_there_and_on_the_row_inside_its_border() {
        use ratatui::layout::Rect;
        let placed = vec![
            (Panel::Scenes, Rect::new(0, 4, 40, 10)),
            (Panel::Chat, Rect::new(40, 4, 40, 20)),
        ];
        assert_eq!(
            panel_at(&placed, 5, 7),
            Some((Panel::Scenes, Some(2))),
            "the third row"
        );
        assert_eq!(
            panel_at(&placed, 5, 4),
            Some((Panel::Scenes, None)),
            "the border is no row"
        );
        assert_eq!(panel_at(&placed, 50, 10), Some((Panel::Chat, Some(5))));
        assert_eq!(panel_at(&placed, 90, 10), None);
    }

    #[test]
    fn a_click_on_a_panel_focuses_it_and_picks_the_row() {
        let status = with_scenes(&["Screen", "BRB", "Old"], "Screen");
        let mut screen = Screen {
            focus: Panel::Chat,
            ..Screen::default()
        };
        clicked(&mut screen, &status, Panel::Scenes, Some(1));
        assert_eq!(screen.focus, Panel::Scenes);
        assert_eq!(screen.picked, 1);
        clicked(&mut screen, &status, Panel::Scenes, Some(9));
        assert_eq!(screen.picked, 1, "a row below the last picks nothing new");
    }

    #[test]
    fn u_brings_back_the_scene_deleted_last() {
        let mut status = with_scenes(&["Screen"], "Screen");
        assert_eq!(
            press('U', &mut Screen::default(), 1, &status),
            Act::Say("the trash is empty")
        );
        status.trash = with_scenes(&["BRB", "Old"], "BRB").scenes;
        assert_eq!(
            press('U', &mut Screen::default(), 1, &status),
            Act::Send(Command::SceneRestore { name: "BRB".into() })
        );
    }

    #[test]
    fn x_deletes_a_scene_after_a_yes_and_never_the_one_on_the_air() {
        let status = with_scenes(&["Screen", "BRB"], "Screen");
        assert_eq!(
            press('X', &mut Screen::default(), 2, &status),
            Act::Say("the scene on the air is not deleted")
        );
        let mut screen = Screen {
            picked: 1,
            ..Screen::default()
        };
        assert_eq!(press('X', &mut screen, 2, &status), Act::Stay);
        assert_eq!(
            press('y', &mut screen, 2, &status),
            Act::Send(Command::SceneDelete { name: "BRB".into() })
        );
    }

    #[test]
    fn x_removes_the_picked_layer_after_a_yes() {
        let status = on_screen();
        let mut screen = Screen {
            focus: Panel::Layers,
            ..Screen::default()
        };
        assert_eq!(press('x', &mut screen, 2, &status), Act::Stay);
        assert_eq!(
            press('y', &mut screen, 2, &status),
            Act::Send(Command::LayerRemove { id: "desk".into() })
        );
    }

    #[test]
    fn a_mic_whose_samples_stop_for_two_seconds_has_no_signal_and_gets_it_back_when_they_move() {
        let mut watch = Signal::default();
        assert!(watch.heard(100, 0.0));
        assert!(watch.heard(100, 1.5), "a second and a half is a pause");
        assert!(!watch.heard(100, 2.5), "two seconds still is no signal");
        assert!(watch.heard(180, 3.0), "samples again: back");
    }

    #[test]
    fn the_gate_says_no_signal_before_open_or_closed() {
        assert_eq!(gate_word(true, false, true), "open");
        assert_eq!(
            signal_word(gate_word(true, false, true), false),
            "no signal"
        );
        assert_eq!(
            signal_word("muted", false),
            "muted",
            "a muted mic is not a lost one"
        );
        assert_eq!(signal_word("no mic", false), "no mic");
        assert_eq!(signal_word("open", true), "open");
    }

    #[test]
    fn g_on_the_music_goes_to_the_next_genre_and_round() {
        let named = |id: &str, name: &str| remuxd_domain::protocol::Named {
            id: id.into(),
            name: name.into(),
        };
        let mut screen = Screen {
            genres: vec![named("lofi", "Lofi"), named("edm", "Edm")],
            ..on_sound(1)
        };
        let status = sounding();
        assert_eq!(
            press('g', &mut screen, 0, &status),
            Act::Send(Command::Genre {
                name: "lofi".into()
            })
        );
        assert_eq!(
            press('g', &mut screen, 0, &status),
            Act::Send(Command::Genre { name: "edm".into() })
        );
        assert_eq!(
            press('g', &mut screen, 0, &status),
            Act::Send(Command::Genre {
                name: "lofi".into()
            })
        );
        assert_eq!(screen.genre.as_deref(), Some("lofi"));
        assert_eq!(
            press('g', &mut on_sound(0), 0, &status),
            Act::Stay,
            "the mic has no genre"
        );
    }

    #[test]
    fn a_layer_with_a_filter_names_it() {
        let scene: Scene = serde_json::from_value(serde_json::json!({
            "name": "s",
            "layers": [{
                "id": "frame", "visible": true, "shader": "/f/fire.wgsl",
                "source": { "kind": "image", "handle": "h", "name": "frame.png", "width": 1920, "height": 1080 },
                "transform": { "x": 0, "y": 0, "width": 1920, "height": 1080, "degrees": 0 }
            }]
        }))
        .expect("a scene");
        assert_eq!(
            rows_of(&scene),
            ["frame · image frame.png · filter fire.wgsl"]
        );
    }

    #[test]
    fn a_filter_steps_through_the_list_and_off_after_the_last() {
        let list = vec!["/f/fire.wgsl".to_string(), "/f/water.wgsl".to_string()];
        assert_eq!(next_filter(&list, None), Some("/f/fire.wgsl".to_string()));
        assert_eq!(
            next_filter(&list, Some("/f/fire.wgsl")),
            Some("/f/water.wgsl".to_string())
        );
        assert_eq!(
            next_filter(&list, Some("/f/water.wgsl")),
            None,
            "after the last, off"
        );
        assert_eq!(
            next_filter(&list, Some("/elsewhere/x.wgsl")),
            Some("/f/fire.wgsl".to_string())
        );
        assert_eq!(next_filter(&[], None), None);
    }

    #[test]
    fn f_filters_the_picked_layer_and_shift_f_the_whole_scene_on_the_air_or_staged() {
        let mut status = on_screen();
        let mut screen = Screen {
            focus: Panel::Layers,
            filters: vec!["/f/fire.wgsl".into()],
            ..Screen::default()
        };
        assert_eq!(
            press('f', &mut screen, 2, &status),
            Act::Send(Command::LayerShader {
                id: "desk".into(),
                path: Some("/f/fire.wgsl".into())
            })
        );
        assert_eq!(
            press('F', &mut screen, 2, &status),
            Act::Send(Command::Shader {
                path: Some("/f/fire.wgsl".into())
            })
        );
        status.staged = Some("BRB".into());
        screen.picked = 1;
        assert_eq!(
            press('F', &mut screen, 2, &status),
            Act::Send(Command::Staged {
                command: Box::new(Command::Shader {
                    path: Some("/f/fire.wgsl".into())
                })
            })
        );
    }

    #[test]
    fn with_no_filters_to_offer_f_and_shift_f_change_nothing_and_send_nothing() {
        let status = on_screen();
        let mut screen = Screen {
            focus: Panel::Layers,
            ..Screen::default()
        };
        assert_eq!(press('f', &mut screen, 2, &status), Act::Stay);
        assert_eq!(press('F', &mut screen, 2, &status), Act::Stay);
    }

    #[test]
    fn filters_are_the_wgsl_files_of_the_folder_in_order() {
        let dir = std::env::temp_dir().join(format!("remux-filters-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        for name in ["water.wgsl", "fire.wgsl", "notes.txt"] {
            std::fs::write(dir.join(name), "").expect("a file");
        }
        let found = filters_in(&dir);
        assert_eq!(
            found
                .iter()
                .map(|p| p.rsplit('/').next().unwrap())
                .collect::<Vec<_>>(),
            ["fire.wgsl", "water.wgsl"]
        );
    }

    #[test]
    fn t_takes_the_staged_scene_to_the_air_from_any_panel() {
        let mut status = with_scenes(&["Screen", "BRB"], "Screen");
        let mut screen = Screen {
            focus: Panel::Sound,
            ..Screen::default()
        };
        assert_eq!(
            press('t', &mut screen, 2, &status),
            Act::Say("nothing staged: enter stages a scene")
        );
        status.staged = Some("BRB".into());
        assert_eq!(
            press('t', &mut screen, 2, &status),
            Act::Send(Command::SceneTake)
        );
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
