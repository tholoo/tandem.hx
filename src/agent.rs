use crate::protocol::TourDraft;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;
pub mod codex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceMode {
    ReadOnly,
    Build,
}
#[derive(Clone, Debug)]
pub struct Turn {
    pub prompt: String,
    pub context: String,
    pub mode: WorkspaceMode,
    pub workspace: PathBuf,
}
#[derive(Debug)]
pub enum AgentEvent {
    Message(String),
    Activity(String),
    Complete(Option<TourDraft>),
    Failed(String),
}
/// The controller grants a workspace capability per turn. Backends cannot change stages.
pub trait AgentBackend: Send {
    fn start(&mut self, turn: Turn, events: mpsc::UnboundedSender<AgentEvent>) -> Result<()>;
    fn cancel(&mut self);
}
/// A deliberately labeled offline demonstration, never selected implicitly.
#[derive(Default)]
pub struct MockBackend;
impl AgentBackend for MockBackend {
    fn start(&mut self, turn: Turn, events: mpsc::UnboundedSender<AgentEvent>) -> Result<()> {
        if turn.mode == WorkspaceMode::Build {
            let path = turn.workspace.join("tandem-example.txt");
            anyhow::ensure!(
                !path.exists(),
                "mock demo refuses to replace tandem-example.txt"
            );
            std::fs::write(path, "A proposal created in Tandem's shadow workspace.\n")?;
            let tour=TourDraft{title:"A first proposal".into(),overview:"This offline demo adds one file. The real working tree is unchanged until Review.".into(),stops:vec![crate::protocol::TourStop{title:"The proposed file".into(),body:"This file exists in the shadow workspace. Review will apply it as an ordinary uncommitted file.".into(),file:"tandem-example.txt".into(),line:1}]};
            events.send(AgentEvent::Complete(Some(tour)))?;
        } else {
            events.send(AgentEvent::Message(format!("[offline mock] {}\nSelect /plan <description>, then /begin to create a demo proposal.",turn.prompt)))?;
            events.send(AgentEvent::Complete(None))?;
        }
        Ok(())
    }
    fn cancel(&mut self) {}
}
