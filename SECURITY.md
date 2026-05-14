# Security Policy

This CLI controls authenticated MyDomaiNesia account surfaces. Treat it like an admin tool.

## Secrets

Never commit or paste:

- MyDomaiNesia passwords or OTPs
- `~/.domainesia/cookies.txt`
- HAR files exported from an authenticated browser session
- CSRF tokens copied from a live session
- `~/.domainesia/config.env` if it contains account-specific paths or endpoints

The repository ignores common local secret file names, but that is not a substitute for review before commit.

## Auth Model

Preferred auth is `domainesia-cli auth browser-login`, which opens a dedicated Chrome profile and stores cookies under `~/.domainesia/cookies.txt`.

For VPS or other systems without Chrome, prefer importing a Netscape cookie jar through stdin:

```bash
cat ~/.domainesia/cookies.txt | ssh user@vps 'domainesia-cli --json auth import-cookies --from-stdin'
```

Treat this as secret transfer. Use SSH or another encrypted channel, and never paste cookie contents into chat, logs, issues, or shell history.

Endpoint-driven login is available for controlled environments, but passwords must be supplied through stdin:

```bash
printf '%s\n' "$DOMAINESIA_PASSWORD" | domainesia-cli auth login --email you@example.com --password-stdin --live
```

`auth headless-login` is also experimental and uses `curl` to submit the MyDomaiNesia login form without Chrome. It must not bypass CAPTCHA, 2FA, or other interactive protections. If MyDomaiNesia requires a challenge, use `auth browser-login` or cookie import instead.

## Write Safety

Write commands are dry-run by default. Use `--live` only after reviewing the JSON preview.

DNS write commands fetch the current DNS Management form, preserve existing records, apply the requested change, then POST the full form. Verify after live writes:

```bash
domainesia-cli --json dns list --domain example.my.id
```

Live DNS writes also require an exact confirmation guard:

```bash
DOMAINESIA_ALLOW_LIVE_WRITES=1 domainesia-cli --json dns add --domain example.my.id --name app --type A --value 192.0.2.10 --live --confirm app.example.my.id
```

Before every live DNS write, the CLI writes a pre-change backup under:

```text
~/.domainesia/backups/<domain>/<timestamp>.json
```

Use `auth validate` before write sessions and `auth logout` when finished on shared machines:

```bash
domainesia-cli --json auth validate
domainesia-cli --json auth logout
domainesia-cli --json auth logout --cookie-only
```

Endpoint-driven login and generic endpoint-driven DNS mode are experimental and disabled unless `DOMAINESIA_ENABLE_EXPERIMENTAL_ENDPOINTS=1` is set.
Live DNS writes are disabled unless `DOMAINESIA_ALLOW_LIVE_WRITES=1` is set.

## Reporting Issues

Open a private security advisory or contact the repository owner directly for bugs that could expose credentials, cookies, account data, or unintended live writes.
