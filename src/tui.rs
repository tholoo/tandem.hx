use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{self, Event as KeyEvent, KeyCode, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    io::{self, Write},
    path::Path,
};
use tandem::{protocol::*, server::read_frame};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::mpsc,
};
struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
    }
}
fn action(input: &str, view: Option<&View>) -> Result<Option<Action>> {
    let (cmd, arg) = input.split_once(' ').unwrap_or((input, ""));
    Ok(Some(match cmd {
        "/quit" => return Ok(None),
        "/plan" => Action::Plan { text: arg.into() },
        "/begin" => Action::Begin,
        "/revise" => Action::Revise {
            feedback: arg.into(),
        },
        "/proposal" => Action::Switch {
            proposal: arg.parse()?,
        },
        "/next" => {
            if view.is_some_and(|v| v.tour.is_some()) {
                Action::TourNext
            } else {
                Action::NextChange
            }
        }
        "/prev" => {
            if view.is_some_and(|v| v.tour.is_some()) {
                Action::TourPrevious
            } else {
                Action::PreviousChange
            }
        }
        "/jump" => Action::TourJump {
            index: arg.parse()?,
        },
        "/close" => Action::TourClose,
        "/review" => Action::Apply,
        "/revert" => Action::Revert {
            change: if arg.is_empty() {
                view.context("no session")?.change_index
            } else {
                arg.parse()?
            },
        },
        "/diff" => Action::Diff,
        "/cancel" => Action::Cancel,
        _ if input.starts_with('/') => anyhow::bail!(
            "Unknown action. /plan <text> /begin /next /prev /jump N /close /review /revert /revise <text> /proposal N /diff /cancel /quit"
        ),
        _ => Action::Message { text: input.into() },
    }))
}
fn safe(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}
fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![];
    for line in safe(text).lines() {
        let chars: Vec<_> = line.chars().collect();
        if chars.is_empty() {
            lines.push(String::new())
        } else {
            for chunk in chars.chunks(width.max(1)) {
                lines.push(chunk.iter().collect());
            }
        }
    }
    lines
}
fn draw(view: Option<&View>, history: &[String], input: &str, scroll: usize) -> Result<()> {
    let (w, h) = terminal::size()?;
    let mut out = io::stdout();
    queue!(
        out,
        Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        SetForegroundColor(Color::Cyan)
    )?;
    let title = view
        .map(|v| {
            format!(
                "TANDEM · {:?}{}{}",
                v.stage,
                if v.busy { " · working" } else { "" },
                v.proposal
                    .map(|p| format!(" · Proposal {p}"))
                    .unwrap_or_default()
            )
        })
        .unwrap_or("TANDEM · connecting".into());
    queue!(
        out,
        Print(title.chars().take(w as usize).collect::<String>()),
        ResetColor
    )?;
    let mut lines = vec![];
    for message in history {
        lines.extend(wrapped(message, w.saturating_sub(1) as usize));
        lines.push(String::new());
    }
    let room = h.saturating_sub(5) as usize;
    let end = lines.len().saturating_sub(scroll);
    let start = end.saturating_sub(room);
    for (i, line) in lines[start..end].iter().enumerate() {
        queue!(out, cursor::MoveTo(0, i as u16 + 2), Print(line))?;
    }
    let hint = if view.is_some_and(|v| v.tour.is_some()) {
        "/next /prev /jump N · narration is in Helix · /review or /close"
    } else {
        "/plan <text> · /begin · /diff · /revert · /quit · PgUp/PgDn"
    };
    queue!(
        out,
        cursor::MoveTo(0, h.saturating_sub(2)),
        SetForegroundColor(Color::DarkGrey),
        Print(hint.chars().take(w as usize).collect::<String>()),
        ResetColor,
        cursor::MoveTo(0, h.saturating_sub(1)),
        Print("> ")
    )?;
    let tail: String = input
        .chars()
        .rev()
        .take(w.saturating_sub(3) as usize)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    queue!(out, Print(safe(&tail)))?;
    out.flush()?;
    Ok(())
}
pub async fn run(socket: &Path) -> Result<()> {
    let stream = UnixStream::connect(socket).await?;
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    terminal::enable_raw_mode()?;
    let _screen = Screen;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let (keys, mut key_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(e) = event::read() {
            if keys.send(e).is_err() {
                break;
            }
        }
    });
    let mut view: Option<View> = None;
    let mut history=vec!["Discuss freely. Source writes stay disabled until you choose /begin from PLAN. Ask for a repository tour in ordinary language.".into()];
    let mut input = String::new();
    let mut id = 0;
    let mut scroll = 0;
    loop {
        draw(view.as_ref(), &history, &input, scroll)?;
        tokio::select! {
            value = read_frame(&mut reader) => {
                let value = value?.context("controller disconnected")?;
                if value.get("event").is_some() {
                    match serde_json::from_value::<Event>(value)? {
                        Event::State { view: v } => view = Some(*v),
                        Event::Message { text } => history.push(format!("Assistant: {text}")),
                        Event::Activity { text } => history.push(text),
                        Event::Error { text } => history.push(format!("Error: {text}")),
                    }
                } else {
                    let response: Response = serde_json::from_value(value)?;
                    if let Some(v) = response.view { view = Some(v); }
                    if let Some(e) = response.error { history.push(format!("Error: {e}")); }
                    if let Some(t) = response.text { history.push(t); }
                }
            }
            Some(key) = key_rx.recv() => {
                let mut send = None;
                match key {
                    KeyEvent::Key(k) if k.kind == event::KeyEventKind::Press => match k.code {
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => send = Some(Action::Cancel),
                        KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => break,
                        KeyCode::Char(c) => input.push(c),
                        KeyCode::Backspace => { input.pop(); }
                        KeyCode::PageUp => scroll = scroll.saturating_add(10),
                        KeyCode::PageDown => scroll = scroll.saturating_sub(10),
                        KeyCode::Enter if !input.trim().is_empty() => {
                            match action(&input, view.as_ref()) {
                                Ok(None) => break,
                                Ok(a) => { history.push(format!("You: {input}")); send = a; }
                                Err(e) => history.push(e.to_string()),
                            }
                            input.clear();
                            scroll = 0;
                        }
                        _ => {}
                    },
                    KeyEvent::Paste(s) => input.push_str(&safe(&s).replace('\n', " ")),
                    _ => {}
                }
                if let Some(action) = send {
                    id += 1;
                    let mut bytes = serde_json::to_vec(&Request { version: VERSION, id, action })?;
                    bytes.push(b'\n');
                    write.write_all(&bytes).await?;
                }
            }
        }
        if history.len() > 2000 {
            history.drain(..1000);
        }
        scroll = scroll.min(
            history
                .iter()
                .map(|s| wrapped(s, terminal::size().map(|x| x.0 as usize).unwrap_or(80)).len() + 1)
                .sum::<usize>(),
        );
    }
    Ok(())
}
