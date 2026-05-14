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

Preferred auth is `domainesia auth browser-login`, which opens a dedicated Chrome profile and stores cookies under `~/.domainesia/cookies.txt`.

Endpoint-driven login is available for controlled environments, but passwords must be supplied through stdin:

```bash
printf '%s\n' "$DOMAINESIA_PASSWORD" | domainesia auth login --email you@example.com --password-stdin --live
```

## Write Safety

Write commands are dry-run by default. Use `--live` only after reviewing the JSON preview.

DNS write commands fetch the current DNS Management form, preserve existing records, apply the requested change, then POST the full form. Verify after live writes:

```bash
domainesia --json dns list --domain example.my.id
```

## Reporting Issues

Open a private security advisory or contact the repository owner directly for bugs that could expose credentials, cookies, account data, or unintended live writes.
