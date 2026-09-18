use crate::{
    agent::{AgentBackend, AgentEvent, Turn, WorkspaceMode},
    controller::Controller,
    protocol::*,
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{os::unix::fs::PermissionsExt, path::Path};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::mpsc,
};

type Reply = mpsc::UnboundedSender<Value>;
struct Incoming {
    request: Request,
    reply: Reply,
}
pub async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Value>> {
    let mut buf = Vec::new();
    let n = reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut buf)
        .await?;
    if n == 0 {
        return Ok(None);
    }
    ensure!(
        n <= MAX_FRAME && buf.last() == Some(&b'\n'),
        "invalid or oversized IPC frame"
    );
    Ok(Some(serde_json::from_slice(&buf)?))
}
async fn client(
    stream: UnixStream,
    tx: mpsc::UnboundedSender<Incoming>,
    joins: mpsc::UnboundedSender<Reply>,
) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let (reply, mut outgoing) = mpsc::unbounded_channel();
    joins.send(reply.clone())?;
    loop {
        tokio::select! {
            frame=read_frame(&mut reader)=> {let Some(frame)=frame? else {return Ok(())};tx.send(Incoming{request:serde_json::from_value(frame)?,reply:reply.clone()})?;}
            Some(value)=outgoing.recv()=> {let mut bytes=serde_json::to_vec(&value)?;bytes.push(b'\n');write.write_all(&bytes).await?;}
        }
    }
}
pub async fn serve(
    mut c: Controller,
    mut backend: Box<dyn AgentBackend>,
    socket: &Path,
) -> Result<()> {
    let listener = UnixListener::bind(socket)
        .context("bind controller socket; an existing session is never overwritten")?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    let (tx, mut incoming) = mpsc::unbounded_channel::<Incoming>();
    let (joins, mut joined) = mpsc::unbounded_channel();
    let (events, mut event_rx) = mpsc::unbounded_channel();
    let mut peers: Vec<Reply> = vec![];
    let mut history: Vec<String> = vec![];
    let result = async {
        loop {
            tokio::select! {
                stream = listener.accept() => {
                    let (stream, _) = stream?;
                    let tx = tx.clone();
                    let joins = joins.clone();
                    tokio::spawn(async move { let _ = client(stream, tx, joins).await; });
                }
                Some(peer) = joined.recv() => {
                    let _ = peer.send(serde_json::to_value(Event::History { lines: history.clone() })?);
                    let _ = peer.send(serde_json::to_value(Event::State { view: Box::new(c.view()?) })?);
                    peers.push(peer);
                }
                Some(incoming) = incoming.recv() => {
                    let req = incoming.request;
                    let user_text = match &req.action {
                        Action::Input { text } | Action::Message { text } => Some(text.clone()),
                        Action::Revise { feedback } => Some(feedback.clone()),
                        _ => None,
                    };
                    let shutdown = matches!(req.action, Action::Shutdown);
                    let cancelled = matches!(req.action, Action::Cancel)
                        || matches!(&req.action, Action::Input { text } if text.split_whitespace().next() == Some("/cancel"));
                    let result = if req.version != VERSION {
                        Err(anyhow::anyhow!("unsupported protocol version {}", req.version))
                    } else {
                        handle(&mut c, backend.as_mut(), req.action, &events).await
                    };
                    let (reply, error) = match result {
                        Ok(t) => (t, None),
                        Err(e) => (Payload::default(), Some(format!("{e:#}"))),
                    };
                    let okay = error.is_none();
                    if okay
                        && let Some(text) = user_text {
                            let text = format!("You: {text}");
                            remember(&mut history, &text);
                            broadcast(&mut peers, Event::Message { text });
                        }
                    if cancelled && okay { while event_rx.try_recv().is_ok() {} }
                    let response = Response { version: VERSION, id: req.id, view: Some(c.view()?), reply, error };
                    let _ = incoming.reply.send(serde_json::to_value(response)?);
                    broadcast(&mut peers, Event::State { view: Box::new(c.view()?) });
                    if shutdown && okay { backend.cancel().await; break Ok(()); }
                }
                Some(event) = event_rx.recv() => {
                    match event {
                        AgentEvent::Message(text) => {
                            let text = format!("Assistant: {text}");
                            remember(&mut history, &text);
                            broadcast(&mut peers, Event::Message { text });
                        }
                        AgentEvent::Activity(text) => broadcast(&mut peers, Event::Activity { text }),
                        AgentEvent::TourNavigate { tour_id, index } => {
                            if let Err(e) = c.tour_jump(tour_id, index) {
                                broadcast(&mut peers, Event::Error { text: e.to_string() });
                            }
                        }
                        AgentEvent::Failed(text) => {
                            backend.cancel().await;
                            c.failed()?;
                            broadcast(&mut peers, Event::Error { text });
                        }
                        AgentEvent::Complete(tour) => {
                            backend.cancel().await;
                            let result = if c.session.stage == Stage::Building {
                                c.finish_build(tour)
                            } else if c.refining() {
                                c.finish_refinement(tour)
                            } else {
                                c.busy = false;
                                if let Some(t) = tour { c.present_tour(t) } else { Ok(()) }
                            };
                            if let Err(e) = result {
                                c.failed()?;
                                broadcast(&mut peers, Event::Error { text: format!("{e:#}") });
                            }
                        }
                    }
                    broadcast(&mut peers, Event::State { view: Box::new(c.view()?) });
                }
                _ = tokio::signal::ctrl_c() => {
                    backend.cancel().await;
                    c.failed()?;
                    break Ok(());
                }
            }
        }
    }.await;
    let _ = std::fs::remove_file(socket);
    result
}
fn broadcast(peers: &mut Vec<Reply>, event: Event) {
    let value = serde_json::to_value(event).expect("event serialization");
    peers.retain(|p| p.send(value.clone()).is_ok());
}
fn remember(history: &mut Vec<String>, text: &str) {
    history.push(text.chars().take(16000).collect());
    while history.len() > 200 || history.iter().map(String::len).sum::<usize>() > 200_000 {
        history.remove(0);
    }
}
fn conversation_action(input: &str, c: &Controller) -> Result<Action> {
    let input = input.trim();
    let (command, argument) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let argument = argument.trim();
    Ok(match command {
        "/begin" => Action::Begin {
            text: argument.into(),
        },
        "/revise" => Action::Revise {
            feedback: argument.into(),
        },
        "/proposal" => Action::Switch {
            proposal: argument.parse()?,
        },
        "/next" if c.tour().is_some() => Action::TourNext,
        "/prev" if c.tour().is_some() => Action::TourPrevious,
        "/next" => Action::NextChange,
        "/prev" => Action::PreviousChange,
        "/jump" => Action::TourJump {
            index: argument.parse()?,
        },
        "/close" => Action::TourClose,
        "/apply" => Action::Apply,
        "/revert" => Action::Revert {
            change: if argument.is_empty() {
                c.session.change_index
            } else {
                argument.parse()?
            },
        },
        "/diff" => Action::Diff,
        "/peek" => Action::Peek,
        "/cancel" => Action::Cancel,
        _ if input.starts_with('/') => anyhow::bail!(
            "Unknown action. Use /begin, /apply, /proposal N, /next, /prev, /jump N, /close, /revert, /diff, /peek, or /cancel."
        ),
        _ => Action::Message { text: input.into() },
    })
}
fn start(
    c: &mut Controller,
    backend: &mut dyn AgentBackend,
    prompt: String,
    mode: WorkspaceMode,
    events: &mpsc::UnboundedSender<AgentEvent>,
) -> Result<()> {
    if mode == WorkspaceMode::ReadOnly && !c.session.pending_proposal {
        c.workspace.sync(&c.current()?)?;
    }
    let turn = Turn {
        prompt,
        context: c.context()?,
        mode,
        workspace: c.workspace.shadow.clone(),
    };
    c.busy = true;
    if let Err(e) = backend.start(turn, events.clone()) {
        c.failed()?;
        return Err(e);
    }
    Ok(())
}
async fn handle(
    c: &mut Controller,
    backend: &mut dyn AgentBackend,
    action: Action,
    events: &mpsc::UnboundedSender<AgentEvent>,
) -> Result<Payload> {
    let action = if let Action::Input { text } = action {
        conversation_action(&text, c)?
    } else {
        action
    };
    match action {
        Action::Input { .. } => unreachable!("conversation input was parsed above"),
        Action::Status => {}
        Action::EditorConnection { connected } => {
            c.editor_connected = Some(connected);
            if connected {
                c.editor = EditorContext {
                    dirty: vec!["waiting for editor context".into()],
                    ..Default::default()
                };
            }
        }
        Action::Context { mut context } => {
            // Context comes from editor APIs, including unsaved text. Limit incidental data.
            context.nearby = context.nearby.chars().take(16000).collect();
            context.selection = context.selection.chars().take(4000).collect();
            c.editor = context;
        }
        Action::Cancel => {
            backend.cancel().await;
            c.failed()?;
        }
        Action::Shutdown => {
            ensure!(!c.busy, "cancel the active turn before shutting down");
        }
        Action::Message { text } | Action::Revise { feedback: text } => {
            ensure!(!c.busy, "agent is busy; cancel the current turn first");
            ensure!(!text.trim().is_empty(), "empty message");
            let mode = if c.session.stage == Stage::Tour && c.editor.dirty.is_empty() {
                c.refine()?;
                WorkspaceMode::Refine
            } else {
                WorkspaceMode::ReadOnly
            };
            start(c, backend, text, mode, events)?;
        }
        Action::Begin { text } => {
            c.begin()?;
            start(
                c,
                backend,
                if text.trim().is_empty() {
                    "Implement the approach agreed in our conversation.".into()
                } else {
                    text
                },
                WorkspaceMode::Build,
                events,
            )?;
        }
        Action::Switch { proposal } => c.switch(proposal)?,
        Action::TourNext => c.tour_move(1)?,
        Action::TourPrevious => c.tour_move(-1)?,
        Action::TourJump { index } => {
            c.tour_jump(c.tour().context("no active tour")?.id, index)?;
        }
        Action::TourClose => c.close_tour()?,
        Action::Apply => c.apply()?,
        Action::NextChange => c.change_move(1)?,
        Action::PreviousChange => c.change_move(-1)?,
        Action::Revert { change } => c.revert(change)?,
        Action::Diff => {
            return Ok(Payload {
                text: Some(c.diff()?),
                ..Payload::default()
            });
        }
        Action::Peek => return Ok(Payload {
            comparison: Some(c.peek()?),
            ..Payload::default()
        }),
    }
    Ok(Payload::default())
}
/// Single request client for CLI tools and tests. Events are intentionally skipped here.
pub async fn request(socket: &Path, action: Action) -> Result<Response> {
    let mut stream = UnixStream::connect(socket).await?;
    let mut bytes = serde_json::to_vec(&Request {
        version: VERSION,
        id: 1,
        action,
    })?;
    bytes.push(b'\n');
    stream.write_all(&bytes).await?;
    let mut reader = BufReader::new(stream);
    loop {
        let value = read_frame(&mut reader)
            .await?
            .context("controller disconnected")?;
        if value.get("id").is_some() {
            return Ok(serde_json::from_value(value)?);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Capture(Option<Turn>);
    impl AgentBackend for Capture {
        fn start(&mut self, turn: Turn, _: mpsc::UnboundedSender<AgentEvent>) -> Result<()> {
            self.0 = Some(turn);
            Ok(())
        }
        fn cancel(
            &mut self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            Box::pin(async {})
        }
    }

    #[tokio::test]
    async fn begin_forwards_inline_request_to_build_backend() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        crate::workspace::git(&repo, &["init", "-q"]).unwrap();
        crate::workspace::git(
            &repo,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "fixture",
            ],
        )
        .unwrap();
        let mut c = Controller::create(&repo, &root.path().join("session")).unwrap();
        let mut backend = Capture::default();
        let (events, _) = mpsc::unbounded_channel();
        let request = "Combine search and search_case_insensitive into one function with a sensitivity argument.";
        handle(
            &mut c,
            &mut backend,
            Action::Input {
                text: format!("/begin {request}"),
            },
            &events,
        )
        .await
        .unwrap();
        let turn = backend.0.as_ref().unwrap();
        assert_eq!(turn.mode, WorkspaceMode::Build);
        assert_eq!(turn.prompt, request);
        c.failed().unwrap();
        handle(
            &mut c,
            &mut backend,
            Action::Input {
                text: "/begin".into(),
            },
            &events,
        )
        .await
        .unwrap();
        assert_eq!(
            backend.0.unwrap().prompt,
            "Implement the approach agreed in our conversation."
        );
    }
}
