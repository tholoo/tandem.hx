use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tandem::{
    agent::{AgentBackend, MockBackend, codex::CodexBackend},
    controller::Controller,
    protocol::Action,
    server, session,
};
#[derive(Parser)]
#[command(version, about = "Human-controlled coding proposals inside Helix")]
struct Args {
    #[command(subcommand)]
    command: Option<Cmd>,
}
#[derive(Subcommand)]
enum Cmd {
    /// Start/reconnect from a Steel editor and carry its JSONL connection.
    Editor {
        #[arg(default_value = ".")]
        project: PathBuf,
        #[arg(long)]
        mock: bool,
    },
    /// Start a controller and establish a persistent shadow workspace.
    Start {
        #[arg(default_value = ".")]
        project: PathBuf,
        #[arg(long)]
        session: Option<PathBuf>,
        #[arg(long)]
        mock: bool,
        #[arg(long, default_value = "codex")]
        codex: PathBuf,
    },
    /// Send a JSON action to the controller (useful for scripting/debugging).
    Send {
        #[arg(env = "TANDEM_SOCKET")]
        socket: PathBuf,
        json: String,
    },
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        None => {
            use clap::CommandFactory;
            Args::command().print_help()?;
            println!();
        }
        Some(Cmd::Start {
            project,
            session,
            mock,
            codex,
        }) => {
            let registry = session::Registry::discover(&project)?;
            let _controller = registry.controller_lock()?;
            let session = if let Some(p) = session {
                if p.is_absolute() {
                    p
                } else {
                    std::env::current_dir()?.join(p)
                }
            } else {
                registry.new_session()?
            };
            let c = Controller::create(&project, &session)?;
            let backend: Box<dyn AgentBackend> = if mock {
                Box::new(MockBackend::default())
            } else {
                Box::new(CodexBackend::new(&session, codex)?)
            };
            let socket = session.join("controller.sock");
            registry.publish(&socket)?;
            println!(
                "Tandem {}\nSession: {}\nShadow: {}\nSocket: {}\n\nRun :tandem in Steel Helix to connect.\n{}",
                env!("CARGO_PKG_VERSION"),
                session.display(),
                c.workspace.shadow.display(),
                socket.display(),
                if mock {
                    "Offline mock backend selected."
                } else {
                    "Codex backend selected. Discussion is read-only."
                }
            );
            server::serve(c, backend, &socket).await?;
        }
        Some(Cmd::Editor { project, mock }) => {
            if let Err(error) = session::editor(&project, mock).await {
                println!(
                    "{}",
                    serde_json::json!({"event":"error", "text":format!("{error:#}")})
                );
            }
        }
        Some(Cmd::Send { socket, json }) => {
            let action: Action = serde_json::from_str(&json)
                .context("expected an action such as {\"action\":\"status\"}")?;
            let response = server::request(&socket, action).await?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            if response.error.is_some() {
                std::process::exit(1)
            }
        }
    }
    Ok(())
}
