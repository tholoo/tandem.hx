# Local protocol v1

The controller listens on a private Unix socket (`0600`, in a `0700` session directory). Connections carry UTF-8 JSON, one object per newline, with a 1 MiB request limit. No network listener exists. The Rust definitions in `src/protocol.rs` are authoritative.

Each request has `version`, a client-chosen integer `id`, and `action`:

```json
{"version":1,"id":1,"action":"message","text":"Tour the authentication flow."}
{"version":1,"id":2,"action":"plan","text":"Add API key authentication."}
{"version":1,"id":3,"action":"begin"}
{"version":1,"id":4,"action":"tour_next"}
{"version":1,"id":5,"action":"apply"}
```

A response echoes `version` and `id`, with `view`, optional `text`, and optional `error`. Conflicts are errors; the controller does not write conflict markers or partially apply an otherwise conflicting proposal. In addition, every connection receives `event` objects: `state`, `message`, `activity`, or `error`. Clients must distinguish events from responses. A newly connected client immediately receives current state.

Other actions: `status`, `context`, `revise` (`feedback`), `switch` (`proposal`), `tour_previous`, `tour_jump` (`index`), `tour_close`, `next_change`, `previous_change`, `revert` (`change`), `diff`, `cancel`, `shutdown`. Tour position 0 is the overview; numbered stops start at 1. Change IDs start at 0. Closing a repository tour returns to the same DISCUSS/PLAN conversation; applying a proposal enters REVIEW. Revision feedback selects PLAN; it never starts BUILD itself.

The view contains the stage, busy flag, real/shadow roots, proposal IDs, independent active tour, review changes, editor context, and optional navigation location. `generation` changes when the editor should act on navigation/reload, so repeated context updates do not yank the cursor back to a tour stop.

`context` publishes `file` (absolute path or null), 1-based `line`/`column`, `selection`, `nearby` unsaved source, and all dirty buffer paths. The controller attaches this plus stage, proposal, active tour, and stop to every assistant turn. No special question command is required.

Use `tandem send SOCKET '{"action":"status"}'` to inspect the current state. `tandem bridge SOCKET` is a transparent JSONL stdio bridge for Steel, which has process ports but no required native Unix-socket extension. It never simulates editor input.
