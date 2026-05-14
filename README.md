# domainesia

Agent-friendly CLI for MyDomaiNesia account and DNS workflows.

The tool is designed for automation agents and shell users who need stable JSON output, explicit authentication checks, and safe DNS changes. It currently focuses on authenticated read/discovery commands and DNS Management writes for DomaiNesia domains.

## Status

- Auth: working through browser-assisted cookie capture.
- Read coverage: dashboard, feature inventory, domains, domain details, DNS records, invoices, raw authenticated GET.
- Write coverage: DNS add/update/delete through the MyDomaiNesia DNS Management form.
- Safety: write commands are dry-run by default; live DNS writes require `DOMAINESIA_ALLOW_LIVE_WRITES=1`, `--live`, and `--confirm <fqdn>`.
- Backup: every live DNS write creates a pre-change JSON backup.

DomaiNesia's public API documentation is oriented around reseller/RNA usage. This CLI therefore works against the authenticated MyDomaiNesia web interface and does not assume a private customer API.

## Unofficial Project And Responsible Use

This project is an independent, unofficial CLI. It is not created, endorsed, sponsored, or maintained by DomaiNesia.

The CLI automates authenticated MyDomaiNesia web workflows using a user-controlled session. DomaiNesia may change its interface, policies, or supported access methods at any time, and those changes may break this tool or make a workflow inappropriate for some accounts.

Use this software only with accounts and domains you are authorized to manage. You are responsible for reviewing and complying with DomaiNesia's [Terms of Service](https://www.domainesia.com/tos/), acceptable use policies, and any applicable laws or organizational rules. The authors and contributors are not responsible for account actions, DNS changes, service disruption, policy violations, data loss, or other outcomes from using this tool.

When an official DomaiNesia API or supported integration is available for your use case, prefer that over web-session automation.

## DomaiNesia Policy Notes

This section is a practical risk note, not legal advice. Review the current DomaiNesia documents before using or publishing automation based on this project.

- DomaiNesia's [Terms of Service](https://www.domainesia.com/tos/) states that use of DomaiNesia services is subject to its Terms of Service and Acceptable Use Policies. It also states that violations by someone using your DomaiNesia account may be treated as violations by you.
- DomaiNesia's account terms state that users are responsible for keeping account credentials secure, recommend strong passwords and 2FA, and treat account activity as activity by the legitimate account owner. Do not share cookies, session files, passwords, HAR files, or captured browser profiles.
- DomaiNesia's usage-limit section for hosting services warns against excessive or unreasonable resource usage and includes automated bots, massive data scraping, third-party service emulation, auto-refresh, auto-post, and similar schemes. This CLI is intended for low-volume, user-directed account administration only. Do not use it for scraping, polling loops, mass automation, bypassing UI protections, or workflows that can burden DomaiNesia systems.
- DomaiNesia publicly describes REST API access as a reseller-domain feature on its [Reseller Domain](https://www.domainesia.com/reseller-domain/) page. This CLI does not claim to be that official API and should not be presented as a supported DomaiNesia integration.
- If DomaiNesia support, policy, or an official API says a workflow should not be automated through MyDomaiNesia web sessions, do not use this CLI for that workflow.

## Requirements

- macOS or Linux
- Rust stable toolchain for building
- `curl`
- Node.js and Google Chrome for `auth browser-login`
- A MyDomaiNesia account with access to the target domain

The Rust binary itself has no third-party crate dependencies.

## Install

```bash
git clone https://github.com/trisetiohidayat/domainesia.git
cd domainesia
make install-local
```

This installs the binary to:

```text
~/.local/bin/domainesia
```

Verify:

```bash
domainesia --help
domainesia --json doctor
```

## Configuration

Initialize local config:

```bash
domainesia --json init --domain example.my.id
```

Config is stored outside the repo:

```text
~/.domainesia/config.env
```

Supported keys:

```env
DOMAINESIA_BASE_URL=https://my.domainesia.com
DOMAINESIA_DEFAULT_DOMAIN=example.my.id
DOMAINESIA_COOKIE_JAR=/Users/you/.domainesia/cookies.txt
DOMAINESIA_LOGIN_ENDPOINT=https://...
DOMAINESIA_DNS_ADD_ENDPOINT=https://...
DOMAINESIA_CSRF_HEADER=X-CSRF-TOKEN
DOMAINESIA_CSRF_TOKEN=...
```

The CLI never prints cookie values or full tokens in `doctor`/`auth status`.

## Authentication

Preferred auth path:

```bash
domainesia --json auth browser-login --timeout-seconds 180
domainesia --json auth status
domainesia --json auth validate
```

`auth browser-login` opens a dedicated Chrome profile, waits for you to log in on the official MyDomaiNesia page, captures DomaiNesia cookies through Chrome DevTools Protocol, and writes a Netscape cookie jar to:

```text
~/.domainesia/cookies.txt
```

Manual cookie import:

```bash
domainesia auth open-login
domainesia --json auth import-cookies --from ~/Downloads/my.domainesia.cookies.txt
```

Logout removes the local cookie jar:

```bash
domainesia --json auth logout
domainesia --json auth logout --cookie-only
```

Endpoint-driven login is available only after a login endpoint is confirmed:

```bash
domainesia --json auth configure --login-endpoint https://captured-login-endpoint
printf '%s\n' "$DOMAINESIA_PASSWORD" | domainesia --json auth login --email you@example.com --password-stdin --live
```

`auth login` is experimental and disabled unless `DOMAINESIA_ENABLE_EXPERIMENTAL_ENDPOINTS=1` is set. It is a dry-run unless `--live` is present. The password must come from stdin so it is not stored in shell history.

## Command Overview

```bash
domainesia --json features list
domainesia --json features forms --path '/clientarea.php?action=domaindns&domainid=123456'

domainesia --json domains list
domainesia --json domains resolve --domain example.my.id
domainesia --json domains detail --domain example.my.id

domainesia --json dns list --domain example.my.id
domainesia --json dns export --domain example.my.id --output ./dns-backup.json
domainesia --json dns plan-add --domain example.my.id --name app --type A --value 192.0.2.10
domainesia --json dns add --domain example.my.id --name app --type A --value 192.0.2.10 --dry-run
DOMAINESIA_ALLOW_LIVE_WRITES=1 domainesia --json dns add --domain example.my.id --name app --type A --value 192.0.2.10 --live --confirm app.example.my.id
domainesia --json dns update --domain example.my.id --name app --value 192.0.2.11 --dry-run
domainesia --json dns delete --domain example.my.id --name app --dry-run

domainesia --json invoices list
domainesia --json raw get https://my.domainesia.com/clientarea.php
```

## DNS Management

List current records:

```bash
domainesia --json dns list --domain example.my.id
```

Export current records:

```bash
domainesia --json dns export --domain example.my.id --output ./dns-backup.json
```

Add a record safely:

```bash
DOMAINESIA_ALLOW_LIVE_WRITES=1 domainesia --json dns add \
  --domain example.my.id \
  --name app \
  --type A \
  --value 192.0.2.10 \
  --dry-run
```

Apply live:

```bash
domainesia --json dns add \
  --domain example.my.id \
  --name app \
  --type A \
  --value 192.0.2.10 \
  --live \
  --confirm app.example.my.id
```

Verify:

```bash
domainesia --json dns list --domain example.my.id
```

Before every live DNS write, the CLI writes a JSON backup of the current DNS state under:

```text
~/.domainesia/backups/<domain>/<timestamp>.json
```

Supported record types in the CLI parser:

```text
A, AAAA, CNAME, TXT, MX
```

The MyDomaiNesia form also exposes `MXE`, `SRV`, `URL`, and `FRAME`; add narrow support before using those live.

## Feature Inventory

`features list` reports known MyDomaiNesia surfaces:

- dashboard
- products/services
- domains
- domain details
- DNS Management
- nameservers
- private nameservers
- DNSSEC
- domain forwarding
- domain contacts
- invoices
- tickets
- account details
- user management
- contacts
- email history
- profile
- security

Use `features forms` to inspect form shape before implementing a dedicated write command:

```bash
domainesia --json features forms --path '/clientarea.php?action=domaindetails&id=123456'
```

## JSON Contract

Success:

```json
{"ok":true,"command":"dns list","data":{}}
```

Error:

```json
{"ok":false,"command":"dns add","error":{"message":"..."}}
```

Under `--json`, errors are machine-readable and should not include credentials.

## Safety Notes

- Do not commit `~/.domainesia`, cookies, HAR files, backups, or local env files.
- DNS write commands are dry-run by default.
- Live writes require `DOMAINESIA_ALLOW_LIVE_WRITES=1`.
- Live writes require `--live`.
- Live DNS writes require `--confirm <fqdn>`.
- Live DNS writes create a pre-change backup under `~/.domainesia/backups`.
- Always run `dns list` after a live write.
- Avoid raw non-GET requests; add a narrow command instead.
- Endpoint-driven login/generic DNS endpoint mode require `DOMAINESIA_ENABLE_EXPERIMENTAL_ENDPOINTS=1`.

## Development

```bash
cargo fmt --check
cargo test --locked
cargo build --release --locked
make install-local
```

Smoke test from another directory:

```bash
cd /tmp
domainesia --json doctor
domainesia --json auth status
domainesia --json features list
```

## Dependency Decision Record

Dependency: none beyond Rust standard library and system tools (`curl`, Node.js, Chrome for browser-login).

Purpose: avoid package risk while using the authenticated MyDomaiNesia web interface.

Version selected: not applicable.

Alternatives considered: Python Click from independent CLI examples, Rust crates such as `clap`/`reqwest`/`serde_json`, Playwright automation.

Security checks: avoided new third-party crate/npm dependencies, so no registry/advisory checks were needed for implementation.

Known advisories or recent incidents: not applicable for newly added dependencies.

Risk: low for the dependency surface; live account actions remain sensitive and require explicit flags.

Decision: implement dependency-free Rust CLI first, add dependencies only after a concrete API need is proven.
