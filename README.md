# Tandem

Tandem is a human-controlled coding assistant inside Steel Helix:

**DISCUSS → /begin → BUILD → TOUR & REFINE → /apply**

The agent constructs a proposal in a persistent shadow worktree. You inspect it through a guided tour, edit the preview or ask for refinements, and apply the result as uncommitted changes when ready. Tandem never commits your project.

Tours also work before changing anything: ask for a repository tour, a subsystem explanation, or a request's path through the code. Tours are independent objects with an overview and ordered, revisitable code ranges. Each stop names the exact lines to read.

## Using Tandem

After the [one-time Steel setup](helix/tandem/README.md), open a Git project normally:

```sh
cd /path/to/project
hx
```

Run `:tandem` to open the native conversation panel. Session discovery, controller startup, editor context, and the shadow workspace are automatic.

- Type a question and press Enter. Escape returns keyboard input to the code; click the panel or run `:tandem` to focus it again. PgUp/PgDn and the mouse wheel scroll conversation.
- Ask “Give me a quick tour,” “go deeper,” or “go back two steps.” Narration appears in a compact reserved strip above the code, with keyboard hints for Previous/Next and `/close` or `/apply`.
- In normal mode, `]t` / `[t` move forward/back through tour stops or review changes. Tandem temporarily borrows the default class motions while connected, restores them on disconnect, and respects custom bindings.
- Discuss an approach in ordinary conversation, then type `/begin` to authorize construction. Conversational agreement never starts the first build.
- During the proposal tour, edit and save the preview or ask for a change in ordinary conversation. Refinements update the shadow proposal; questions keep your tour position.
- Type `/apply` when satisfied. This copies the saved, refined proposal into your real working directory as ordinary uncommitted changes.
- `/proposal 1` revisits an older proposal and its saved draft. `/next`, `/prev`, `/revert`, and `/diff` support lightweight review. `/jump N` and `/close` navigate/end repository tours.
- `:tandem-hide` hides conversation without stopping the session. `:tandem-stop` cancels any active work, stops the controller, and removes Tandem's UI while leaving Helix open.

Questions automatically include the current file, cursor, selection, nearby unsaved code, stage, active tour, and tour stop. A context label shows the code location beside the composer. Type `/` for command completion and availability. In normal mode, Space t p compares old and current code in native splits, Space t r returns to the current tour stop, and Space t j searches stops. See the [keyboard controls](helix/tandem/README.md#interaction) for expanded messages and narration.

## Multiple editors and sessions

Each canonical Git working tree has one active controller and one connected editor. Running `:tandem` repeatedly focuses the same panel. A second editor receives a clear error instead of taking over context or creating a competing session. Separate Git worktrees can run independent sessions.

Closing Helix disconnects its editor but leaves the controller and proposals available. A later `:tandem` reconnects to that controller, including its bounded conversation history. Begin/Apply/Revert are blocked while the associated editor is disconnected or has unsaved buffers. Explicitly stopping the controller ends that session; starting again establishes a fresh baseline.

## Development and validation

```sh
nix develop
cargo test
cargo clippy --all-targets -- -D warnings
nix build
nix run -- --help

# Outside restricted builders: local sockets, child processes, and Bubblewrap.
cargo test --test runtime --test session -- --ignored
# Real Steel editor; isolated configuration, no injected editor keys or model calls.
python3 scripts/check-helix.py /path/to/steel/hx /path/to/helix/runtime
# Screen-level colors, destination visibility, narration layout, and q! regression.
python3 scripts/check-helix-ui.py /path/to/steel/hx /path/to/helix/runtime
# Optional live API check: existing Codex login and normal model usage.
python3 scripts/check-codex.py
```

Linux, Rust 1.89+, and Git are required. The Nix package and development shell select Bubblewrap explicitly. Codex also needs to be on PATH. Existing Codex login and model/provider configuration are reused; external MCP tools, plugins, and hooks are excluded because they can bypass the source-code sandbox. The entire runtime has read-only source access until Begin, and can write only shadow source during BUILD and proposal refinement. After application, further conversation is read-only until another `/begin`.

For an offline demonstration, run `tandem start /path/to/project --mock` in a terminal, then `:tandem` in Helix on that project. The explicit mock backend creates a labeled demonstration file; it is not an implementation agent. `tandem send SOCKET '{"action":"status"}'` remains available for protocol inspection.

## Current status and limits

This is a working Linux prototype, exercised with Codex CLI 0.154.0, the installed Steelix 2026-05-21 package. It includes dirty baselines, multi-file proposals, version switching, preservation of manual edits, safe hunk reversion, native conversation, and independent repository/proposal tours. Stable Helix cannot load the plugin.

Repositories need an existing commit. Symlinks, submodules, and non-UTF-8 filenames are rejected; ignored files are not imported initially. Save buffers and avoid simultaneous writes during application/reversion. Failed or cancelled revisions leave real files untouched and restore the saved preview from before that turn. Save preview buffers before asking for changes; questions with unsaved buffers remain read-only. Build commands have private scratch space but no network or permission expansion, so uncached dependency downloads need a future approval path. The conversation panel styles role labels, commands, and inline backtick code. Ctrl+X expands the draft into a native Helix buffer for multiline editing. Full Markdown and fenced-code syntax highlighting are not yet implemented.

Session directories can contain source snapshots and a private copy of the existing Codex login; remove them when finished. No credentials or machine configuration belong in this repository.

See [architecture](docs/architecture.md) and [local protocol](docs/protocol.md) for the boundaries and preservation rules.
