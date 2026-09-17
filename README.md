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

This is a working Linux prototype, tested with Codex CLI 0.154.0 and a pinned experimental Steel Helix build. It includes dirty baselines, a persistent shared-object shadow worktree, multi-file proposal versions, conflict-safe application/reversion, revisions that preserve manual edits, independent repository/proposal tours, and a terminal conversation surface.

The live Codex loop and native Steel context/navigation/tour rendering have been exercised in isolated fixture repositories. Stable Helix is not supported; follow the [Steel setup](helix/tandem/README.md).

Run an offline demonstration in a Git repository with an existing commit:

```sh
tandem start /path/to/project --mock
# In a second pane, using the socket printed by start:
tandem chat /path/to/session/controller.sock
```

Use `/plan add a demonstration file`, then `/begin`. After the proposal tour, `/review` applies it; `/revert` reverses the current change. `/diff` shows the proposal patch. `/revise <feedback>` prepares another plan; `/begin` explicitly authorizes its build. `/proposal 1` revisits an older version. `/quit` detaches the TUI without deleting session data.

Ask “Give me a tour of this repo” in ordinary conversation. The editor displays an overview and ordered stops; Previous/Next can revisit locations. `/jump N` rewinds to a known stop and `/close` ends a repository tour without leaving DISCUSS. Questions automatically include file/cursor/selection and active-tour context.

For a two-pane Zellij session, run the controller in the background, export its `TANDEM_SOCKET`, and load [examples/zellij.kdl](examples/zellij.kdl):

```sh
tandem start /path/to/project --session /path/outside/project/session > /tmp/tandem.log 2>&1 &
export TANDEM_SOCKET=/path/outside/project/session/controller.sock
zellij --layout /path/to/tandem/examples/zellij.kdl
```

Wait for the socket to appear before launching the layout. Both panes inherit the session address. The controller stays outside the layout, and the tour card appears only when needed. `/quit` detaches the conversation; stop the controller with Ctrl-C or `tandem send "$TANDEM_SOCKET" '{"action":"shutdown"}'` when idle.

Without `--mock`, the backend is Codex. Read-only discussion never implicitly starts a build. A local filesystem sandbox surrounds the complete Codex runtime, with write access to source granted only to the shadow worktree during BUILD.

## Validation and limits

```sh
cargo test
# Needs local sockets and Bubblewrap; outside restricted build sandboxes:
cargo test --test runtime -- --ignored
# Optional: a real Steel editor, isolated configuration, no injected keys:
python3 scripts/check-helix.py /path/to/steel/hx /path/to/helix/runtime
# Optional live API check: uses existing Codex login and normal model usage:
python3 scripts/check-codex.py
```

`nix develop` and the Nix package select Bubblewrap explicitly. Outside Nix, `TANDEM_BWRAP` can select a compatible binary if a system wrapper is unsuitable. The mock backend is a labeled demo, not an implementation agent.

Current limits: a repository needs an existing commit; symlinks, submodules, and non-UTF-8 filenames are rejected. Ignored files are not imported initially. Save Helix buffers before Begin/Review/Revert, and avoid concurrent writes during those operations. Session snapshots persist, but a crash-resume UI is not implemented. Conflicting revisions leave the shadow available for inspection and the real tree untouched. Runtime files can include source and a private copy of the existing Codex login; delete the session directory when finished. No credentials or machine configuration belong in this repository.

See [architecture](docs/architecture.md) and [IPC protocol](docs/protocol.md) for the boundaries and preservation rules.
