# Local protocol v1

The controller listens on a private Unix socket (`0600`, in a `0700` session directory). Connections carry UTF-8 JSON, one object per newline, with a 1 MiB request limit. No network listener exists. The Rust definitions in `src/protocol.rs` are authoritative.

Each request has `version`, a client-chosen integer `id`, and `action`:

```json
{"version":1,"id":1,"action":"message","text":"Tour the authentication flow."}
{"version":1,"id":2,"action":"message","text":"How should we add API key authentication?"}
{"version":1,"id":3,"action":"begin","text":"Add API-key authentication."}
{"version":1,"id":4,"action":"tour_next"}
{"version":1,"id":5,"action":"apply"}
```

A response echoes `version` and `id`, with `view`, optional `text`, optional `comparison`, and optional `error`. Conflicts are errors; the controller does not write conflict markers or partially apply an otherwise conflicting proposal. In addition, every connection receives `event` objects: `history`, `state`, `message`, `activity`, or `error`. History contains bounded conversation lines; message text includes the speaker label. Clients must distinguish events from responses. A newly connected client receives conversation history and current state.

The native composer sends `input` (`text`); the controller interprets explicit commands such as `/begin` and `/apply`, or treats the text as ordinary conversation. `/begin <request>` passes that request directly to the build turn; bare `/begin` continues the agreed conversation. The `begin` action uses an empty `text` for the latter. If the agent asks for clarification without changing code, Tandem returns to the preceding conversation and tour.

Other actions: `status`, `context`, `revise` (`feedback`), `switch` (`proposal`), `tour_previous`, `tour_jump` (`index`), `tour_close`, `next_change`, `previous_change`, `revert` (`change`), `diff`, `cancel`, `shutdown`. Tour position 0 is the overview; numbered stops start at 1. Change IDs start at 0. Closing a repository tour returns to the same DISCUSS conversation; applying a proposal enters APPLIED. `/begin` works directly from discussion. While an unapplied proposal is active, ordinary messages can refine it; questions leave the proposal and tour position intact. The optional `revise` action is equivalent to a message. Saved preview edits are included at application.

The view contains the stage, busy flag, real/shadow roots, proposal IDs, independent active tour, review changes, editor context, and optional navigation location. `generation` changes when the editor should act on navigation/reload, so repeated context updates do not yank the cursor back to a tour stop.

Each tour stop contains `file`, `line`, `end_line`, `title`, and `body`. Both line numbers are required, 1-based, and inclusive. The controller validates the whole range against the source. Agents choose the smallest contiguous block that supports each explanation; separate blocks become separate stops.

`context` publishes `file` (absolute path or null), 1-based `line`/`column`, `selection`, optional 1-based `selection_start_line`/`selection_end_line`, `nearby` unsaved source, and all dirty code buffer paths. The controller attaches this plus stage, proposal, active tour, and stop to every assistant turn. The expanded message buffer is excluded from source context and dirty-buffer checks.

`peek` returns a structured `comparison` at the current editor location, relative to the selected proposal's baseline: repository-relative `file`, absolute `current_file`, full UTF-8 `old` and `current` contents (null for an absent file), and 1-based `old_line` / `current_line` hunk positions. It includes saved preview edits, or working-directory edits after application. Unsaved selected buffers, binary or oversized files, and locations without a text change return an error. The plugin opens native old/current splits and hides chat until comparison closes. `/return`, `/stops`, and `/compose` are handled locally by the plugin, using tour navigation actions or editor APIs.

Use `tandem send SOCKET '{"action":"status"}'` to inspect the current state. `tandem editor PROJECT` is the managed JSONL stdio transport for Steel. It discovers/starts the controller and holds the exclusive editor lease; `editor_connection` updates the controller on attachment/disconnection. It emits a `connected` event with the canonical project and handles `stop` by canceling then shutting down. It never simulates editor input.
