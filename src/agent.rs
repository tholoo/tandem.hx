use crate::protocol::TourDraft;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;
pub mod codex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceMode {
    ReadOnly,
    Build,
    Refine,
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
    TourNavigate { tour_id: usize, index: usize },
    Complete(Option<TourDraft>),
    Failed(String),
}
/// The controller grants a workspace capability per turn. Backends cannot change stages.
pub trait AgentBackend: Send {
    fn start(&mut self, turn: Turn, events: mpsc::UnboundedSender<AgentEvent>) -> Result<()>;
    fn cancel(&mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>;
}
/// A deliberately labeled offline demonstration, never selected implicitly.
#[derive(Default)]
pub struct MockBackend {
    builds: usize,
}
impl AgentBackend for MockBackend {
    fn start(&mut self, turn: Turn, events: mpsc::UnboundedSender<AgentEvent>) -> Result<()> {
        if turn.mode == WorkspaceMode::Build
            || (turn.mode == WorkspaceMode::Refine && turn.prompt.to_lowercase().contains("change"))
        {
            let path = turn.workspace.join("tandem-example.txt");
            if self.builds == 0 {
                anyhow::ensure!(
                    !path.exists(),
                    "mock demo refuses to replace tandem-example.txt"
                );
                std::fs::write(&path, "A proposal created in Tandem's shadow workspace.\n")?;
            } else {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
                writeln!(file, "Offline revision {}.", self.builds + 1)?;
            }
            self.builds += 1;
            let tour=TourDraft{title:format!("Offline proposal {}",self.builds),overview:"This offline demo creates or extends one file. The real working tree is unchanged until /apply.".into(),stops:vec![crate::protocol::TourStop{title:"The proposed file".into(),body:"This file exists in the shadow workspace. /apply will apply it as an ordinary uncommitted file.".into(),file:"tandem-example.txt".into(),line:1,end_line:1}]};
            events.send(AgentEvent::Complete(Some(tour)))?;
        } else if turn.prompt.to_lowercase().contains("tour") {
            let files = crate::workspace::capture(&turn.workspace)?;
            let file = files
                .keys()
                .find(|p| !p.starts_with('.'))
                .ok_or_else(|| anyhow::anyhow!("no files to tour"))?
                .clone();
            let stop=crate::protocol::TourStop {title:"Explore existing code".into(),body:"Offline demonstration: this stop points into an existing repository file. Codex supplies the conceptual narrative in live sessions.".into(),file,line:1,end_line:1};
            events.send(AgentEvent::Complete(Some(TourDraft {
                title: "Repository tour (offline demo)".into(),
                overview: "A read-only tour, independent of any proposal.".into(),
                stops: vec![
                    stop.clone(),
                    crate::protocol::TourStop {
                        title: "Return to the same location".into(),
                        body: "A later stop can continue the explanation at the same location."
                            .into(),
                        ..stop
                    },
                ],
            })))?;
        } else {
            events.send(AgentEvent::Message(format!(
                "[offline mock] {}\nDiscuss the change, then use /begin to create a demo proposal.",
                turn.prompt
            )))?;
            events.send(AgentEvent::Complete(None))?;
        }
        Ok(())
    }
    fn cancel(&mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {})
    }
}
