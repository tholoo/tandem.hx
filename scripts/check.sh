#!/usr/bin/env bash
set -euo pipefail
python3 scripts/format.py --check
ruff check scripts
actionlint
git diff --check
gitleaks git --redact --no-banner
cargo clippy --all-targets -- -D warnings
cargo test
