# Native Steel integration

Tandem runs inside Helix: conversation occupies a right-hand panel, and temporary tour narration occupies a compact reserved strip above the code.

Tandem uses the prebuilt Nix Steelix 2026-05-21 package.

## One-time installation

Build Tandem with `nix build` and put `result/bin` on PATH. Link or copy the complete `result/share/tandem/helix/tandem` directory into your Helix configuration directory as `tandem`. Add this to `init.scm`:

```scheme
(require "tandem/tandem.scm")
```

Open a project normally and run `:tandem` when wanted. An optional normal-mode binding in Helix's configuration is:

```toml
[keys.normal.space]
a = ":tandem"
```

Home Manager can manage the directory link, initialization, and binding declaratively. Install the complete plugin directory from the same build.

Closing Helix leaves the controller running. After updating Tandem, finish the current proposal and stop the session with `:tandem-stop`; reopen Helix to load the updated plugin, then run `:tandem`. The new session starts from the current working directory.

## Interaction

`:tandem` starts or reconnects and focuses conversation. Enter sends text, Escape returns input to code, and clicking the panel focuses it. The composer supports cursor movement, Home/End, Backspace/Delete, paste, Ctrl-A/E/U, and Ctrl-C to cancel. Enter keeps an ordinary message in the composer while the agent is busy; cancel or wait for Ready before sending it. Slash commands, including `/cancel`, remain available. PgUp/PgDn or the mouse wheel scroll conversation. Use `/begin` to build, `/apply` to apply, and `/cancel` to interrupt.

In normal mode, `]t` goes forward and `[t` goes back: tour stops while a tour is active, changes after application. These bindings temporarily replace Helix's default class motions while connected and are restored on disconnect. Existing custom bindings are respected. With the optional configuration above, Space a returns to conversation; Escape returns to code.

Additional normal-mode defaults are installed under Space t, with existing custom bindings taking precedence. Press Space t to see each key's description:

| Keys      | Action                                                                      | Conversation command         |
| --------- | --------------------------------------------------------------------------- | ---------------------------- |
| Space t p | Toggle old/current comparison at the cursor                                 | `/peek`                      |
| Space t r | Return to the current tour stop after exploring                             | `/return`                    |
| Space t j | Fuzzy-search tour stops; arrows or Ctrl-n/p select, Enter jumps, Esc closes | `/stops`                     |
| Space t e | Expand the draft into a native Helix message buffer                         | `/compose` or Ctrl+X in chat |
| Space t s | Send the expanded message                                                   |                              |
| Space t q | Return to chat and keep the expanded draft                                  |                              |

The message buffer supports normal Helix editing and multiline text. Escape switches it to normal mode. Closing it with `:bc!` also keeps its draft. The source context stays on the code you were looking at when you opened it. Comparison uses saved contents: old code on the left, current code on the right, with chat temporarily hidden. Use Ctrl-w h/l to switch panes and Space t p again to restore your previous splits, cursor, and chat. The current file stays editable; the old side is a disposable scratch snapshot. Both panes start at the selected changed block and scroll independently. Save the current buffer before opening comparison.

Type `/` in chat to browse commands and their availability. Up/Down selects a suggestion, Tab completes it, and Enter sends. Unavailable commands explain why and keep the draft. The context label above the composer shows the file, line or selection range, and unsaved status sent with your question.

Use `/begin` to implement the approach discussed, or `/begin <request>` to give the implementation request directly. If the agent needs clarification before making changes, its question appears in the conversation.

Up/Down in chat recalls your submitted messages for editing and resending. Down past the newest message restores the unsent draft and cursor position. When slash-command suggestions are open, Up/Down selects those instead.

Distinct syntax accent colors distinguish You/Assistant labels even when a theme uses identical normal/focused text colors. Markdown `**bold**` renders in bold. Inline backtick code and command tokens use the theme’s code color and a subtle background, in conversation and narration. An animated spinner runs while the agent is busy, and the latest tool activity occupies one status line rather than filling the conversation. The UI uses the same presentation for every provider. Full Markdown/code-block highlighting is not yet implemented.

`:tandem-hide` hides conversation without ending the session. `:tandem-stop` cancels active work and stops Tandem while preserving the editor and real files. The temporary tour shows Repository or Preview, a reading range such as `src/auth.rs:42–57 · 16 lines`, and keyboard hints for Previous/Next and `/close` or `/apply`.

The narration strip reserves space above the code. Tour navigation selects the reading range using Helix’s normal selection, with the cursor at its start. The highlight shows the reading boundary and ordinary cursor movement clears it; `/return` selects the range again. Editing commands act on this normal selection. Multi-line stops align the start near the top of the viewport; single-line stops are centered. Closing a repository tour restores the original buffer and selection while it remains open. Optional editor commands are `:tandem-next`, `:tandem-previous`, `:tandem-apply`, `:tandem-revert`, and `:tandem-close`.

Save buffers before Begin/Apply/Revert. Proposal tours show an editable shadow preview. Save edits, ask for refinements in ordinary conversation, and use `/apply` when ready to copy the result into the real working tree. Discuss an approach and use `/begin` directly to start a new proposal. Unsaved documents are never automatically reloaded. Questions publish current editor context before submitting, including file/cursor/selection and dirty buffers.

## Implementation and verification

The thin plugin uses native clipping, component, event, cursor, and editor-command interfaces. A `tandem editor` subprocess discovers the checkout, acquires its editor lease, starts or reconnects to the controller, and transports JSONL. The controller alone owns stages and source writes. The native conversation panel does not become the editor's current code document, so questions retain code context.

An isolated check launches the actual editor in a PTY and invokes native functions to validate automatic startup, conversation input/rendering, both tour sources, editor context, navigation, ordinary editing, revisions, application/reversion, and stopping. It never sends terminal key sequences into Helix. A separate screen-level check uses pyte (included in `nix develop`) to assert distinct label/command colors, centered destinations, top narration, and clean forced quit.

These plugin interfaces remain experimental.

## Runtime compatibility

Use the runtime shipped for your Steelix build, with matching grammar binaries and query files. A mismatch can disable syntax highlighting even when `hx --health rust` finds the parser. Helix's log may report `Failed to compile highlights` and an invalid node type.

The test and recording scripts place the supplied runtime in their isolated Helix configuration, so it takes precedence over wrapper-provided runtime paths. This affects only the disposable test editor. The plugin does not replace your installed editor runtime.
