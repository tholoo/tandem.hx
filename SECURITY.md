# Security

Tandem is pre-release. Only the current development branch is maintained, and it has not undergone an independent security audit.

## Intended boundary

The controller alone applies proposals to the real repository. The Codex runtime has read-only source access during discussion and can write shadow source during implementation and refinement. Git metadata remains read-only. Additional permission requests are denied, and external MCP tools, plugins, and hooks are excluded.

This is a write-control boundary, not a claim that untrusted repositories are safe or that the agent cannot read sensitive host data. Use Tandem with repositories and model providers you trust. Approval UI and crash recovery remain unfinished.

Session directories can contain private source and authentication data. Do not attach them to issues. Share the smallest relevant excerpt and remove credentials, personal paths, and private code.

## Reporting a vulnerability

Use **Security → Report a vulnerability** on GitHub when private reporting is enabled. If that option is unavailable, open an issue asking for a private reporting channel without including vulnerability details or sensitive data. Do not post credentials or a working attack in a public issue.
