# AGENTS.md

Project-specific instructions for agents working on `domainesia-cli`.

## Project Context

`domainesia-cli` is an unofficial Rust CLI for authenticated MyDomaiNesia account and DNS workflows. It automates user-controlled MyDomaiNesia web sessions and is not an official DomaiNesia API client.

Keep the project positioned as:

- Unofficial and independent.
- Low-volume, user-directed account administration.
- Safer-by-default: dry-run first, explicit live write gates, no secret output.

Do not describe this project as endorsed, sponsored, supported, or maintained by DomaiNesia.

## Safety Boundaries

Never commit, paste, print, or summarize secret values from:

- `~/.domainesia/cookies.txt`
- `~/.domainesia/config.env`
- HAR files from authenticated browser sessions
- Browser profiles under `~/.domainesia/chrome-profile`
- CSRF tokens, session cookies, passwords, OTPs, or authenticated HTML responses

Do not add logging that prints cookies, hidden form tokens, full HTML bodies, request headers, `Set-Cookie`, or authenticated response previews.

For headless/VPS auth, prefer `auth import-cookies --from-stdin` over storing cookies in command arguments. `auth headless-login` is experimental and must not bypass CAPTCHA, 2FA, or other interactive protections.

All MyDomaiNesia URL handling must remain constrained to:

- Relative paths beginning with `/`
- Exact `https://my.domainesia.com`
- Paths, queries, or fragments under the exact `my.domainesia.com` host

Do not loosen URL validation to prefix-only matching.

## Live DNS Writes

Treat live DNS writes as destructive account actions.

The CLI must keep all three live-write gates:

- `DOMAINESIA_ALLOW_LIVE_WRITES=1`
- `--live`
- `--confirm <fqdn>`

Do not bypass these gates for tests, demos, convenience, or agent automation.

Before changing live-write behavior, ensure the CLI still:

- Defaults to dry-run.
- Fetches current DNS state before submitting changes.
- Preserves existing records.
- Writes a pre-change backup under `~/.domainesia/backups`.
- Produces stable JSON output.

Only run live DNS commands when the user explicitly asks for the exact change and confirms the target FQDN. Prefer dry-run and read-only commands for exploration.

## DomaiNesia Policy Notes

Respect the project README policy notes:

- Users are responsible for complying with DomaiNesia Terms of Service and acceptable-use policies.
- Do not build scraping, polling loops, mass automation, UI-protection bypasses, or high-volume workflows.
- Prefer official DomaiNesia APIs or supported integrations when available.

References:

- https://www.domainesia.com/tos/
- https://www.domainesia.com/reseller-domain/

## Development Workflow

This project intentionally has no third-party Rust crate dependencies. Prefer the Rust standard library and existing local helpers before adding dependencies.

If a dependency is proposed, perform a current supply-chain review first and document:

- Dependency
- Purpose
- Version selected
- Alternatives considered
- Security checks
- Known advisories or recent incidents
- Risk
- Decision

Required checks before committing:

```bash
cargo fmt --check
cargo test --locked
cargo build --release --locked
```

Use stable, machine-readable JSON for `--json` output. Avoid changing JSON field names without updating tests and documentation.

## File And Repo Hygiene

Keep local account data outside the repository. The project uses `~/.domainesia` for local runtime state; do not move cookie/config storage into the repo.

Before publishing or pushing, scan for accidental secrets or personal account details:

```bash
git status --short
git ls-files
rg -n "(COOKIE|Set-Cookie|session|csrf|token|password|secret|BEGIN (RSA|OPENSSH|PRIVATE)|AKIA|ghp_|github_pat_|DOMAINESIA_PASSWORD)" -S .
```

Expected public files include source, docs, examples, tests, and sanitized fixtures only.

## Documentation

Keep README and SECURITY.md aligned with behavior changes. If a safety gate, auth mode, cookie path, backup behavior, or policy note changes, update docs in the same commit.

Maintain clear disclaimers:

- This is an unofficial project.
- Users are responsible for account actions and policy compliance.
- Official APIs or supported integrations are preferred when available.
