# Contributing to Tandem

Thanks for trying Tandem. Reports about a confusing interaction are as useful as patches. Describe what you were trying to do and what got in the way.

## Development setup

```sh
nix develop
pre-commit install
cargo build
```

The flake pins the toolchain and supplies all formatters. `pre-commit install` installs both the pre-commit and pre-push hooks for this checkout. Run Git commands inside `nix develop`, or use direnv with the included `.envrc`.

The commit hook checks formatting, Python lint, workflow syntax, and staged changes for secrets. The push hook runs Clippy and Rust tests. Checks do not rewrite or stage your files. The pre-commit runner preserves partially staged changes while checking the staged snapshot.

## Format and check

```sh
nix fmt
nix fmt -- --check
nix develop --command bash scripts/check.sh
nix build
```

Formatting uses rustfmt, **nixfmt**, Ruff, Taplo, Prettier, and Schemat. The same checks run in GitHub Actions. To run just the commit hooks:

```sh
pre-commit run --all-files
```

Runtime integration tests need Linux namespaces, Bubblewrap, local sockets, and child processes:

```sh
cargo test --test runtime --test session -- --ignored
```

The ordinary Rust suite and CI do not make model calls or require a Codex login.

## Editor changes

Use a Steel-enabled Helix build. The integration has been exercised with Steelix 2026-05-21; the plugin API is experimental. Supply your editor binary and runtime explicitly:

```sh
python3 scripts/check-helix.py /path/to/hx /path/to/runtime
python3 scripts/check-helix-ui.py /path/to/hx /path/to/runtime
```

Both use disposable repositories and isolated editor configuration. The first covers the controller/editor workflow. The second sends real keyboard input and checks rendering, navigation, editing, and shutdown. Neither uses your desktop window or a model account.

For the optional live backend test, use an existing Codex login:

```sh
python3 scripts/check-codex.py
```

That check makes real model calls and uses normal account usage. Never run it automatically in pull-request CI.

## Pull requests

Keep changes focused. Explain the problem, the resulting behavior, and how you checked it. For editor UI changes, include a screenshot or recording; [the media guide](docs/media/README.md) describes the reproducible demo.

Tour narration, navigation, and conversation should stay shared across repository and proposal tours. Keep provider-specific messages inside the backend adapter. The controller owns workspace permissions and application of changes. See [architecture](docs/architecture.md) for the boundaries.

Protocols and stored state can change during this pre-release phase. Update callers, docs, and tests together rather than adding migration layers.
