# Architecture

> Tandem permits autonomy while constructing a proposal, but the human controls what enters and remains in the real codebase.

## Boundaries

One Rust package provides a controller library and one executable. The controller is a local process; its JSONL Unix socket connects a conversation TUI and thin Steel plugin. An `AgentBackend` receives a turn with a workspace capability and emits backend-neutral events. Only the Codex adapter knows app-server messages. The explicit mock backend exercises the same controller.

The controller owns `Discuss`, `Plan`, `Building`, `Tour`, and `Review`. Conversational agreement never calls Begin. Plan changes and revision feedback stay read-only until the user explicitly begins. The real working tree is written only by application/reversion in the controller. BUILD can edit the persistent shadow workspace autonomously.

The hard pre-BUILD write gate is structural: the complete Codex runtime runs in Bubblewrap with a read-only host filesystem and read-only shadow source. BUILD changes only the shadow mount to writable. Git metadata stays read-only. Codex receives explicit per-turn sandbox policy and no escalation approval. Failure to establish this environment is an error, never an unsandboxed fallback. Runtime state has a separate private writable directory. External MCP servers/plugins/hooks are excluded, since an external service is outside this filesystem boundary.

## Git and proposals

A private clone uses shared Git objects; a detached worktree belongs to that private clone. No worktree registration, index operation, commit, branch, stash, reset, or history mutation occurs in the real repository. Actual tracked contents and non-ignored untracked files form the baseline, including staged/unstaged differences as they exist on disk. The user's index remains unchanged.

Snapshots are Git tree objects, never commits. Proposal IDs refer to immutable baseline/result trees. The shadow worktree is synchronized incrementally between turns. Ignored build outputs remain local to it. Each proposal is multi-file; it is not a sequence of independent file acceptances.

Apply/revert uses BASE, intended result, and current disk state. Non-overlapping text edits merge; overlapping text edits, binary changes, creations/deletions, and unsupported paths fail explicitly before writes. Text hunks are independently reversible. Writes are preflighted, files are replaced atomically, and ordinary IO failure triggers a guarded rollback. There is no claim of a cross-process, multi-file filesystem transaction: users must save and avoid simultaneous writes during application. Unsaved Helix buffers block consequential transitions.

Human edits after application are carried into the next build baseline. A reverse-merge conflict probe rejects proposals that overwrite those choices. Older proposals are reconciled with recorded human choices before application. Conflicts leave the real files untouched; no conflict markers are silently written into them.

## Independent tours

A Tour has an identity, source (`Repository` or `Proposal(id)`), overview, ordered stops, and current stop. Proposals do not contain tours. Repository tours are temporary activities within DISCUSS or PLAN, preserving that stage when closed. Proposal completion creates a tour and enters the visible TOUR stage. Both use identical navigation and rendering.

Tour order follows an explanation or execution story, not filesystem order. Duplicate locations are valid. Narration lives in a temporary editor-local card, never in a permanent assistant/Zellij pane. Normal conversation receives current editor context, stage, active proposal, active tour, and current stop automatically. No special question hotkey exists.

## Deliberate limits

The first slice supports Unix/Linux, committed Git repositories, UTF-8 filenames, regular files, and executable bits. Symlinks and submodules fail explicitly rather than following paths outside the workspace. Ignored files and unsaved editor buffers are not baseline content. Snapshots and session metadata are retained on disk; crash-resume UI is not yet provided.

No scope drift indicator, provenance display, concept navigator, pinned plan invariants, multi-agent comparison, implementation journal, risk tour, review annotations, special editing mode, or automatic commit is part of this design. Keep approval mechanics small; the adapter currently denies permission expansion.

## API references

- [Codex app-server](https://learn.chatgpt.com/docs/app-server): stdio JSONL, thread start/resume, turn start, completion notifications, output schemas, sandbox policies. Adapter checked against locally generated CLI 0.154.0 schemas.
- [Helix Steel branch](https://github.com/mattwparas/helix/tree/09d67dfe7300ab18c267e6b0cbfbb493cce21d37): editor hooks, components, native commands, and main-thread callbacks. Stable Helix does not provide this integration.
