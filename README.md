# Tandem

Tandem keeps the human in control of a coding assistant's work, beside Helix in Zellij:

**DISCUSS → PLAN → BUILD → TOUR → REVIEW**

The agent constructs a proposal in a persistent shadow worktree. You understand it through a guided tour, apply it as uncommitted changes, edit normally, and revert individual changes when needed. Tandem never commits your project.

Tours are also useful before changing anything: ask for a tour of the repository, a subsystem, or a request's path through the code. Tours are independent objects, with an overview and ordered, revisitable code locations.

## Development

```sh
nix develop
cargo test
cargo clippy --all-targets -- -D warnings
nix build
nix run -- --help
```

Rust, Git, and Linux are required. Codex builds additionally require `codex` and working Bubblewrap/user namespaces. Existing Codex login and model/provider configuration are reused; external MCP tools, plugins, and hooks are deliberately excluded from the agent runtime because they can bypass a source-code sandbox.

## Current implementation

The controller, private shared-object Git worktree, dirty baselines, immutable proposal versions, three-way application, hunk reversion, independent tours, JSONL Unix socket protocol, and conversation TUI are implemented. The Codex app-server adapter is implemented; live runtime and experimental Steel integration validation are in progress.

Run an offline demonstration in a Git repository with an existing commit:

```sh
tandem start /path/to/project --mock
# In a second pane, using the socket printed by start:
tandem chat /path/to/session/controller.sock
```

Use `/plan add a demonstration file`, then `/begin`. After the proposal tour, `/review` applies it; `/revert` reverses the current change. `/diff` shows the proposal patch. `/revise <feedback>` prepares another plan; `/begin` explicitly authorizes its build. `/proposal 1` revisits an older version. `/quit` detaches the TUI without deleting session data.

Without `--mock`, the backend is Codex. Read-only discussion never implicitly starts a build. A local filesystem sandbox surrounds the complete Codex runtime, with write access to source granted only to the shadow worktree during BUILD.

See [architecture](docs/architecture.md) for boundaries and preservation rules.
