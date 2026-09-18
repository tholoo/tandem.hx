# Architecture

> Tandem permits autonomy while constructing a proposal, but the human controls what enters and remains in the real codebase.

## Boundaries

Tandem is pre-release. Protocol, state, editor integration, and tests evolve together; breaking changes are expected during development.

One Rust package provides a controller library and one executable. The controller is a local process; its JSONL Unix socket connects a thin Steel plugin with a native conversation panel and temporary tour card. An `AgentBackend` receives a turn with a workspace capability and emits backend-neutral events. Only the Codex adapter knows app-server messages. The explicit mock backend exercises the same controller.

The controller owns `Discuss`, `Building`, `Tour`, and `Applied`. Ordinary discussion stays read-only until `/begin`. A pending proposal is authorized for iterative shadow edits: questions and change requests use the same conversation. The backend receives a `Refine` capability, answers questions without editing, and returns an updated tour when it changes code. An unchanged turn retains the proposal and tour position. Dirty editor buffers keep the turn read-only so questions can still use unsaved context. `/apply` alone copies the saved proposal to the real working tree; `/revert` reverses applied changes. After application, conversation becomes read-only until the next `/begin`.

The hard pre-BUILD write gate is structural: the complete Codex runtime runs in Bubblewrap with a read-only host filesystem and read-only shadow source. BUILD and proposal refinement change only the shadow mount to writable. Git metadata stays read-only. Codex receives explicit per-turn sandbox policy and no escalation approval. Failure to establish this environment is an error, never an unsandboxed fallback. Runtime state has a separate private writable directory, and BUILD commands have isolated temporary scratch space. Command network access is disabled; permission-expansion UI is deferred. External MCP servers/plugins/hooks are excluded, since an external service is outside this filesystem boundary.

## Editor lifecycle

`:tandem` starts a local `tandem editor` transport. It discovers the canonical checkout, acquires an exclusive editor lease, and starts or reconnects to one controller. The controller holds a separate process-lifetime file lock per checkout, including during baseline capture. Locks live in a private runtime registry, never in the real repository. OS locks release on process exit; stale addresses do not authorize replacing a live controller. Linked Git worktrees have distinct identities.

A second editor cannot overwrite the active editor's context. Closing the editor releases its lease but retains the controller, proposals, and bounded in-memory conversation history. Reconnecting initially blocks consequential actions until fresh editor context arrives. Disconnected editors also block Begin/Apply/Revert. `:tandem-stop` cancels and reaps the backend before shutting down the controller; Helix remains open.

The native panel renders conversation with role and inline-code styling and receives input without switching the current code document. A context label beside the composer shows the source file, line or selected range, and unsaved status. The expanded composer uses a named Helix scratch buffer and keeps source context pinned; its draft survives closing the buffer. The controller parses explicit conversation commands; UI code cannot infer permission to build. Slash completion offers descriptions and explains unavailable commands.

The plugin owns temporary stop pickers and native code comparisons. A comparison requests saved baseline/current content and hunk positions from the controller. It places an old scratch snapshot to the left of the editable current file, hides chat, and restores the previous windows and cursor when toggled closed. Temporary snapshots are excluded from assistant context and dirty-buffer checks. Both repository and proposal tours use the same rendering/navigation machinery.

Conversation presentation stays provider-neutral: theme styles, a busy animation, a compact current-activity line, and contextual Previous/Next. The animation uses editor callbacks only while visible and busy. The native UI consumes Tandem events. Additional providers implement `AgentBackend`. Codex is currently the only live backend.

## Git and proposals

A private clone uses shared Git objects; a detached worktree belongs to that private clone. No worktree registration, index operation, commit, branch, stash, reset, or history mutation occurs in the real repository. Actual tracked contents and non-ignored untracked files form the baseline, including staged/unstaged differences as they exist on disk. The user's index remains unchanged.

Snapshots are Git tree objects, never commits. Proposal IDs refer to immutable baseline/result trees, with saved editable drafts tracked separately by proposal ID. Applying or starting a revision captures saved preview edits; switching proposals retains each draft. A turn-start snapshot restores the previous saved preview on failure or cancellation. The shadow worktree is synchronized incrementally between turns. Ignored build outputs remain local to it. Each proposal can span multiple files.

Apply/revert uses BASE, intended result, and current disk state. Non-overlapping text edits merge; overlapping text edits, binary changes, creations/deletions, and unsupported paths fail explicitly before writes. Text hunks are independently reversible. Writes are preflighted, files are replaced atomically, and ordinary IO failure triggers a guarded rollback. There is no claim of a cross-process, multi-file filesystem transaction: users must save and avoid simultaneous writes during application. Unsaved Helix buffers block consequential transitions.

Human edits after application are carried into the next build baseline. A reverse-merge conflict probe rejects proposals that overwrite those choices. Human deltas are composed newest-first for the probe so a later manual choice supersedes an earlier one. Unapplied drafts, including saved human edits, remain the starting implementation for refinement turns. Real-workspace edits are reconciled at application or the next explicit Begin. Older proposals are reconciled with recorded human choices before application. Conflicts leave the real files untouched; no conflict markers are silently written into them.

## Independent tours

A Tour has an identity, source (`Repository` or `Proposal(id)`), overview, ordered stops, and current stop. Each stop has a required inclusive reading range (`line` through `end_line`), a title, and an explanation. Proposals do not contain tours. Repository tours are temporary activities within DISCUSS, preserving that stage when closed. Proposal completion creates a tour and enters the visible TOUR stage. Both use identical navigation and rendering.

Tour order follows an explanation or execution story, not filesystem order. Duplicate locations are valid. Narration lives in a temporary reserved strip above the code, with its source (Repository or Preview), file/range, and line count visible. Navigation selects the inclusive reading range with a normal Helix selection; subsequent cursor movement remains ordinary editing. Normal conversation receives current editor context, stage, active proposal, active tour, and current stop automatically. Navigation frames the range after the strip changes the code viewport, and delayed callbacks stop accessing documents once no editor views remain. The user can return to the current stop after exploring or search the stop list. Closing a repository tour restores the original buffer and selection if that document is still open.

Narration-only refinements replace the narration while retaining its repository/proposal source. Code refinements create a new proposal and tour; they do not write to the real repository. The backend can request a stop in the active tour; the controller checks the tour identity and bounds before navigating. This narrow capability cannot begin BUILD or apply code. Questions about code leave the current tour in place.

## Deliberate limits

The first slice supports Unix/Linux, committed Git repositories, UTF-8 filenames, regular files, and executable bits. Symlinks and submodules fail explicitly rather than following paths outside the workspace. Ignored files and unsaved editor buffers are not baseline content. Snapshots and session metadata are retained on disk; crash-resume UI is not yet provided.

## API references

- [Codex app-server](https://learn.chatgpt.com/docs/app-server): stdio JSONL, thread start/resume, turn start, completion notifications, output schemas, sandbox policies. Adapter checked against locally generated CLI 0.154.0 schemas.
- [Helix Steel branch](https://github.com/mattwparas/helix/tree/09d67dfe7300ab18c267e6b0cbfbb493cce21d37): editor hooks, components, native commands, and main-thread callbacks. Stable Helix does not provide this integration.
