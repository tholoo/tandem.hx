# Current scope

Tandem is an experimental Linux application, not a stable release. It has been exercised with Codex CLI 0.154.0 and Steelix 2026-05-21. Stock Helix cannot load the Steel plugin. Other provider adapters are not implemented yet.

## Repository and editor behavior

- Repositories need an existing commit. Symlinks, submodules, and non-UTF-8 filenames are rejected. Ignored files are not imported into the initial preview.
- Save buffers before building, applying, reverting, or asking the agent to change a preview. Questions can use unsaved editor context.
- Avoid simultaneous writes during application or reversion. Conflicts are reported instead of partially applying an otherwise conflicting proposal; this is not a filesystem transaction across independent processes.
- One controller and one connected editor are allowed per working tree. Separate Git worktrees can have separate sessions.
- Closing Helix retains the running controller and its proposals. `:tandem-stop` ends that session. A new session establishes a fresh baseline; restoring sessions after a controller crash is not implemented.
- Comparison uses saved UTF-8 text and limits response size. Its native panes scroll independently; it does not insert padding rows to align every changed line.
- Conversation supports role colors, bold, and inline code. Full Markdown and fenced-code highlighting are not implemented.

## Agent permissions

Discussion is read-only. `/begin` grants source writes in the shadow workspace; subsequent proposal refinements use that same workspace. `/apply` copies saved proposal contents to the real working directory, and further conversation returns to read-only mode.

Additional permission requests are rejected. There is no interactive approval flow yet, so uncached dependency downloads and commands needing extra access can be blocked. The backend deliberately excludes external MCP tools, plugins, and hooks from the Codex runtime.

Failed or cancelled revisions restore the saved preview from before the turn. Human edits are preserved where they can be merged; conflicting edits require an explicit decision.

## Session data

Private runtime directories contain source snapshots, conversation state, and a copy of existing Codex authentication when file-based login is used. They are retained after stopping and should be removed when no longer needed. Do not share an entire session directory in a bug report.

See [security](../SECURITY.md) for the intended boundary and reporting guidance.
