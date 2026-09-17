mod tui;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tandem::{
    agent::{AgentBackend, MockBackend, codex::CodexBackend},
    controller::Controller,
    protocol::Action,
    server,
};
#[derive(Parser)]
#[command(version, about = "Human-controlled coding proposals beside Helix")]
struct Args {
    #[command(subcommand)]
    command: Option<Cmd>,
}
#[derive(Subcommand)]
enum Cmd {
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
    /// Open the conversation TUI in a second terminal/Zellij pane.
    Chat {
        #[arg(env = "TANDEM_SOCKET")]
        socket: PathBuf,
    },
    /// Send a JSON action to the controller (useful for scripting/debugging).
    Send {
        #[arg(env = "TANDEM_SOCKET")]
        socket: PathBuf,
        json: String,
    },
    /// JSONL stdio transport for the Steel plugin. Does not control the editor.
    Bridge {
        #[arg(env = "TANDEM_SOCKET")]
        socket: PathBuf,
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
            let session = if let Some(p) = session {
                if p.is_absolute() {
                    p
                } else {
                    std::env::current_dir()?.join(p)
                }
            } else {
                let root = std::env::var_os("XDG_RUNTIME_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join("tandem");
                std::fs::create_dir_all(&root)?;
                tempfile::Builder::new()
                    .prefix("session-")
                    .tempdir_in(root)?
                    .keep()
            };
            let c = Controller::create(&project, &session)?;
            let backend: Box<dyn AgentBackend> = if mock {
                Box::new(MockBackend)
            } else {
                Box::new(CodexBackend::new(&session, codex)?)
            };
            let socket = session.join("controller.sock");
            println!(
                "Tandem {}\nSession: {}\nShadow: {}\nSocket: {}\n\nIn the assistant pane: tandem chat '{}'\nSet TANDEM_SOCKET to this socket before starting Steel Helix.\n{}",
                env!("CARGO_PKG_VERSION"),
                session.display(),
                c.workspace.shadow.display(),
                socket.display(),
                socket.display(),
                if mock {
                    "Offline mock backend selected."
                } else {
                    "Codex backend selected. Discussion is read-only."
                }
            );
            server::serve(c, backend, &socket).await?;
        }
        Some(Cmd::Chat { socket }) => tui::run(&socket).await?,
        Some(Cmd::Bridge { socket }) => server::bridge(&socket).await?,
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
