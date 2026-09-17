# Steel integration

Targets [mattwparas/helix](https://github.com/mattwparas/helix) commit `09d67dfe7300ab18c267e6b0cbfbb493cce21d37` (Steel `24cd21598c091fb88bc10a6375a1ded25e677c37`). Stable Helix cannot load this plugin.

Put `tandem.scm` in your Helix configuration directory, then add to `init.scm`:

```scheme
(require "tandem.scm")
(tandem-connect)
```

Launch that Helix with `TANDEM_SOCKET` set to the socket printed by `tandem start`. `tandem` must be on PATH. Use your existing Zellij editor/conversation layout; narration appears in a temporary card within Helix. Click Previous/Next/Close or Review. Optional commands are `:tandem-next`, `:tandem-previous`, `:tandem-review`, `:tandem-revert`, and `:tandem-close`; bind them only if desired. `/jump N` in the conversation pane provides lightweight rewind.

The plugin uses native editor commands and component APIs. A `tandem bridge` subprocess carries JSONL to the controller's Unix socket; it neither edits nor injects keystrokes. Incoming IPC is read on a background thread, then scheduled onto Helix's main thread. Editor hooks publish file, cursor, selection, nearby unsaved text, and dirty buffers automatically.

Save buffers normally before Begin/Review/Revert. Proposal tours show the shadow preview; Review opens the actual working tree for ordinary editing. Files with unsaved changes are never automatically reloaded.

To build the tested editor independently (outside this repository):

```sh
git clone https://github.com/mattwparas/helix.git helix-steel
cd helix-steel
git checkout 09d67dfe7300ab18c267e6b0cbfbb493cce21d37
cargo build --locked --bin hx
export HELIX_RUNTIME="$PWD/runtime"
# Put this build's target/debug directory before stable hx on PATH.
```

The integration check can disable grammar downloads with `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1`; normal editing benefits from building grammars. The pinned APIs are experimental. Tandem's test script launches this real editor in a PTY with isolated configuration and validates both tour sources, context, native navigation, application, and reversion without sending keystrokes.
