#!/usr/bin/env python3
"""Format repository sources, or check them without rewriting (--check)."""

import argparse
import subprocess
from pathlib import Path


def run(*args):
    subprocess.run(args, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root = Path.cwd()
    if not (root / "Cargo.toml").exists():
        parser.error("run this from the Tandem repository root")
    files = (
        subprocess.check_output(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"]
        )
        .decode()
        .split("\0")
    )
    files = sorted({p for p in files if p and Path(p).is_file()})
    run("cargo", "fmt", "--all", *(["--", "--check"] if args.check else []))
    fixtures = [p for p in files if p.startswith("scripts/") and p.endswith(".rs")]
    if fixtures:
        run(
            "rustfmt",
            "--edition",
            "2024",
            *(["--check"] if args.check else []),
            *fixtures,
        )
    for suffix, command in [
        (".nix", ["nixfmt", *(["--check"] if args.check else [])]),
        (".py", ["ruff", "format", *(["--check"] if args.check else [])]),
        (".toml", ["taplo", "fmt", *(["--check"] if args.check else [])]),
    ]:
        selected = [p for p in files if p.endswith(suffix)]
        if selected:
            run(*command, *selected)
    docs = [p for p in files if Path(p).suffix in {".md", ".json", ".yml", ".yaml"}]
    if docs:
        run("prettier", "--check" if args.check else "--write", *docs)
    for name in (p for p in files if p.endswith(".scm")):
        path = Path(name)
        before = path.read_bytes()
        after = subprocess.check_output(["schemat"], input=before)
        if args.check:
            if before != after:
                raise SystemExit(f"Unformatted Scheme: {name}. Run nix fmt.")
        elif before != after:
            path.write_bytes(after)


if __name__ == "__main__":
    main()
