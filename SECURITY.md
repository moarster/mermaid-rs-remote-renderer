# Security policy

## Reporting a vulnerability

**Please do not open public issues for security-sensitive bugs.**

Use GitHub's [private vulnerability reporting](https://github.com/moarster/mermaid-rs-remote-renderer/security/advisories/new)
to send a confidential report. You should hear back within a week — if you
don't, please ping again; silence isn't acknowledgement.

## Scope

This repository is the HTTP wrapper around
[`mermaid-rs-renderer`](https://github.com/1jehuang/mermaid-rs-renderer).

**In scope:**

- Auth / token-gate bypass.
- Rate-limit bypass (e.g. via spoofed forwarded headers when the operator
  has trust enabled).
- Resource exhaustion not caught by the configured timeouts and limits.
- Container or process privilege escalation.
- Sensitive data leakage in logs or error responses.

**Out of scope** (please file with the upstream project, then let me know
once a fix lands so we can pull it in):

- Rendering bugs in `mermaid-rs-renderer` (wrong SVG, parser crash on valid
  Mermaid input).
- Vulnerabilities in third-party dependencies — file with that dependency
  first.

## Supported versions

Only the latest commit on `main` is actively supported.
