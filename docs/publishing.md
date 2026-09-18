# Publishing checklist

Tandem can be shared as an experimental project. Publishing the repository is distinct from declaring a stable release.

## GitHub About

Intended repository: `tholoo/tandem.hx`. The product and executable remain **Tandem** and `tandem`.

Description:

> Guided code tours and reviewable AI edits inside Helix.

Suggested topics: `helix`, `steel`, `rust`, `nix`, `ai`, `code-review`, `developer-tools`.

## Before making the repository public

- Run `nix develop --command bash scripts/check.sh`, the runtime integration tests, both native Helix checks, and `nix build`.
- Scan all Git history with `gitleaks git --redact --no-banner --log-opts=--all`. Review tracked files and media for private source and personal information.
- Inspect the README images at normal browser width. The included recordings use a disposable fixture and scripted responses, as stated in their caption.
- Confirm the MIT license and contributor attribution are appropriate for the code being published.

## GitHub settings after creating the repository

- Set the About description and topics above.
- Enable private vulnerability reporting before directing users to the Security tab.
- Enable Actions, and wait for the Checks workflow to pass on GitHub itself. A local run cannot verify hosted runner configuration.
- Protect `main` with the Checks job if you want changes to go through pull requests.
- Keep the experimental label until approval handling, recovery, and broader editor compatibility have been addressed.

The setup instructions and package metadata use `https://github.com/tholoo/tandem.hx`. Add a CI badge once the repository exists and its first hosted check passes.
