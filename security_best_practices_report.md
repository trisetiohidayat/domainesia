# Security Best Practices Report

Executive summary: the CLI already has strong baseline controls for a personal/internal tool: dry-run defaults, explicit `--live`, exact `--confirm <fqdn>` for DNS writes, cookie file permission hardening, and CI. The initial review found risks around accidental sensitive output, permissive URL validation, local auth-helper hardening, and cleanup of browser-session material. These findings were addressed in follow-up hardening commits.

## High Severity

### H-1: Error and preview output can leak sensitive authenticated HTML

Impact: account details, CSRF tokens, email addresses, or other authenticated page content could be written to terminal logs, CI logs, shell history captures, or agent transcripts.

Evidence:

- `src/main.rs` returned `response_preview` from endpoint-driven login output.
- `src/main.rs` returned `response_preview` from generic live DNS endpoint output.
- `src/main.rs` included full failed GET stdout in error messages.
- `src/main.rs` included full failed POST stdout in error messages.

Recommendation:

- Do not include HTML response bodies in errors.
- Replace response previews with `response_bytes`, final URL/status metadata, and maybe a small allowlisted success indicator.
- Redact likely sensitive patterns before any error output: `token`, `csrfToken`, email, `Set-Cookie`, hidden inputs, and full HTML bodies.

## Medium Severity

### M-1: URL validation uses prefix matching and can accept lookalike hosts

Evidence:

- `src/main.rs` accepted URLs starting with `https://my.domainesia.com`.

Risk:

`https://my.domainesia.com.evil.example/...` satisfies a simple prefix check. Browser/curl cookie domain rules should prevent DomaiNesia cookies from being sent to that host, but the CLI would still make requests to unintended hosts and weaken the safety boundary.

Recommendation:

- Parse/validate host strictly.
- Accept only:
  - relative paths starting with `/`
  - `https://my.domainesia.com`
  - `https://my.domainesia.com/...`
  - `https://my.domainesia.com?...`
  - `https://my.domainesia.com#...`

### M-2: Config and backup files are not explicitly permission-hardened

Evidence:

- `src/main.rs` writes `~/.domainesia/config.env`.
- `src/main.rs` writes config updates.
- `src/main.rs` writes DNS backups.
- `src/main.rs` had `set_owner_only_permissions`, but it was not applied to config or backup files.

Risk:

Config may contain endpoint/CSRF values and account-local paths. Backups reveal DNS topology. They are less sensitive than cookies, but should still be private by default.

Recommendation:

- Create `~/.domainesia` and subdirectories with owner-only permissions on Unix.
- Apply `0600` to config and backup files after write.
- Apply `0700` to `~/.domainesia`, `backups`, and `chrome-profile` where practical.

### M-3: Browser auth helper is written to a predictable path and executed with `node` from PATH

Evidence:

- `src/main.rs` writes a runtime helper script.
- `src/main.rs` places it at `~/.domainesia/cdp-cookie-capture.mjs`.
- `src/main.rs` executes `node`.
- `src/main.rs` allows `DOMAINESIA_CHROME_PATH` override.

Risk:

On a compromised or shared local account, a predictable helper path and PATH-based interpreter resolution could be abused to capture cookies. This is primarily a local-threat risk, but this CLI explicitly handles session cookies.

Recommendation:

- Write helper script with `0600`.
- Ensure parent directory is `0700`.
- Prefer absolute trusted `node` discovery or document PATH trust assumptions.
- Consider embedding less JS at runtime or using a temporary file with randomized name and owner-only permissions.
- Validate `DOMAINESIA_CHROME_PATH` points to an executable file.

### M-4: `auth logout` does not clear the dedicated Chrome profile

Evidence:

- `src/main.rs` implemented `auth logout` by removing only the cookie jar.
- `src/main.rs` stores the dedicated Chrome profile under `~/.domainesia/chrome-profile`.

Risk:

The browser profile may retain cookies/session state after `auth logout`, so a later `auth browser-login` can rehydrate a session even when the user expects logout to remove local auth state.

Recommendation:

- Add `auth logout --all` or make logout also remove the dedicated Chrome profile.
- At minimum, document that logout currently removes the CLI cookie jar only.

## Low Severity

### L-1: Generic endpoint-driven login and generic DNS endpoint path should be discouraged or feature-gated

Evidence:

- `src/main.rs` implements endpoint-driven login.
- `src/main.rs` still supports generic endpoint-based `dns add` if `DOMAINESIA_DNS_ADD_ENDPOINT` is configured.

Risk:

These paths are more fragile and harder to secure than the current form-driven DNS implementation and browser-assisted auth.

Recommendation:

- Mark these as experimental in help/docs.
- Consider requiring an environment opt-in such as `DOMAINESIA_ENABLE_EXPERIMENTAL_ENDPOINTS=1`.

## Positive Controls Observed

- DNS writes are dry-run by default.
- Live form-driven DNS writes require exact `--confirm <fqdn>`.
- Cookie jar files are hardened to `0600` on Unix after import/browser login.
- CI runs formatting, tests, and release build.
- Fixture tests cover sanitized domain, DNS, and invoice parser behavior.
- Repository `.gitignore` excludes common local secret artifacts.

## Suggested Fix Order

1. Remove/redact response bodies and previews from CLI output.
2. Tighten URL validation to exact host checks.
3. Harden config, backup, runtime helper, and directory permissions.
4. Extend `auth logout` to remove the dedicated Chrome profile or add `--all`.
5. Feature-gate generic endpoint-driven auth/DNS paths.

## Remediation Status

- H-1 remediated: response body previews and full curl stdout were removed from error/live output.
- M-1 remediated: URL validation now accepts only exact `https://my.domainesia.com` host forms or relative paths.
- M-2 remediated: config/helper/backup files use owner-only permissions, local directories use owner-only permissions, symlinks are rejected, and writes are atomic.
- M-4 remediated: `auth logout` removes the dedicated Chrome profile by default; `--cookie-only` keeps the profile.
- L-1 remediated: endpoint-driven login and generic DNS endpoint mode require `DOMAINESIA_ENABLE_EXPERIMENTAL_ENDPOINTS=1`.
- Additional hardening: live DNS writes require `DOMAINESIA_ALLOW_LIVE_WRITES=1` in addition to `--live` and `--confirm <fqdn>`.
