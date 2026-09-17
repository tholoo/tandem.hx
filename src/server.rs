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
                    let _ = peer.send(serde_json::to_value(Event::State { view: Box::new(c.view()?) })?);
                    peers.push(peer);
                }
                Some(incoming) = incoming.recv() => {
                    let req = incoming.request;
                    let shutdown = matches!(req.action, Action::Shutdown);
                    let cancelled = matches!(req.action, Action::Cancel);
                    let result = if req.version != VERSION {
                        Err(anyhow::anyhow!("unsupported protocol version {}", req.version))
                    } else {
                        handle(&mut c, backend.as_mut(), req.action, &events).await
                    };
                    let (text, error) = match result {
                        Ok(t) => (t, None),
                        Err(e) => (None, Some(format!("{e:#}"))),
                    };
                    let okay = error.is_none();
                    if cancelled && okay { while event_rx.try_recv().is_ok() {} }
                    let response = Response { version: VERSION, id: req.id, view: Some(c.view()?), text, error };
                    let _ = incoming.reply.send(serde_json::to_value(response)?);
                    broadcast(&mut peers, Event::State { view: Box::new(c.view()?) });
                    if shutdown && okay { backend.cancel().await; break Ok(()); }
                }
                Some(event) = event_rx.recv() => {
                    match event {
                        AgentEvent::Message(text) => broadcast(&mut peers, Event::Message { text }),
                        AgentEvent::Activity(text) => broadcast(&mut peers, Event::Activity { text }),
                        AgentEvent::Failed(text) => {
                            backend.cancel().await;
                            c.failed()?;
                            broadcast(&mut peers, Event::Error { text });
                        }
                        AgentEvent::Complete(tour) => {
                            backend.cancel().await;
                            let result = if c.session.stage == Stage::Building {
                                tour.context("backend omitted proposal tour").and_then(|t| c.complete(t))
                            } else {
                                c.busy = false;
                                if let Some(t) = tour { c.repository_tour(t) } else { Ok(()) }
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
fn start(
    c: &mut Controller,
    backend: &mut dyn AgentBackend,
    prompt: String,
    mode: WorkspaceMode,
    events: &mpsc::UnboundedSender<AgentEvent>,
) -> Result<()> {
    if mode == WorkspaceMode::ReadOnly && c.session.stage != Stage::Tour {
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
) -> Result<Option<String>> {
    match action {
        Action::Status => {}
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
        Action::Message { text } => {
            ensure!(!c.busy, "agent is busy; cancel the current turn first");
            ensure!(!text.trim().is_empty(), "empty message");
            start(c, backend, text, WorkspaceMode::ReadOnly, events)?;
        }
        Action::Plan { text } => {
            c.plan(text.clone())?;
            start(
                c,
                backend,
                format!("Discuss and summarize this plan; do not implement yet: {text}"),
                WorkspaceMode::ReadOnly,
                events,
            )?;
        }
        Action::Revise { feedback } => {
            c.plan(feedback.clone())?;
            start(
                c,
                backend,
                format!(
                    "Revise the plan using this feedback. Preserve manual edits. Wait for Begin: {feedback}"
                ),
                WorkspaceMode::ReadOnly,
                events,
            )?;
        }
        Action::Begin => {
            c.begin()?;
            start(
                c,
                backend,
                format!(
                    "BUILD is now authorized in the shadow workspace. Implement the agreed plan, run appropriate checks, and return a conceptual tour. Plan: {}",
                    c.session.plan
                ),
                WorkspaceMode::Build,
                events,
            )?;
        }
        Action::Switch { proposal } => c.switch(proposal)?,
        Action::TourNext => c.tour_move(1)?,
        Action::TourPrevious => c.tour_move(-1)?,
        Action::TourJump { index } => {
            let now = c.tour().context("no active tour")?.current_stop;
            c.tour_move(index as isize - now as isize)?;
        }
        Action::TourClose => c.close_tour()?,
        Action::Apply => c.apply()?,
        Action::NextChange => c.change_move(1)?,
        Action::PreviousChange => c.change_move(-1)?,
        Action::Revert { change } => c.revert(change)?,
        Action::Diff => {
            let p = c.proposal().context("no proposal")?;
            return Ok(Some(c.workspace.diff(&p.base, &p.tree, 3)?));
        }
    }
    Ok(None)
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
/// Stdio bridge for Steel. It transports JSON only; all editor operations stay in the plugin.
pub async fn bridge(socket: &Path) -> Result<()> {
    let stream = UnixStream::connect(socket).await?;
    let (mut read, mut write) = stream.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    tokio::select! {r=tokio::io::copy(&mut stdin,&mut write)=>{r?;},r=tokio::io::copy(&mut read,&mut stdout)=>{r?;}}
    Ok(())
}
