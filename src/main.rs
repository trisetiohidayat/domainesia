use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const CDP_CAPTURE_JS: &str = r#"
import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

const [loginUrl, cookieJar, timeoutSeconds, profileDir, portArg, waitUrlPartArg] = process.argv.slice(2);
const port = Number(portArg || '9229');
const timeoutMs = Number(timeoutSeconds || '180') * 1000;
const waitUrlPart = waitUrlPartArg || 'clientarea.php';
const chromePath = process.env.DOMAINESIA_CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitJson(url, timeout) {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    try {
      const response = await fetch(url);
      if (response.ok) return await response.json();
    } catch {}
    await sleep(500);
  }
  throw new Error(`timed out waiting for ${url}`);
}

function cdp(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let id = 0;
  const pending = new Map();
  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(msg.error.message || JSON.stringify(msg.error)));
      else resolve(msg.result || {});
    }
  };
  return new Promise((resolve, reject) => {
    ws.onerror = () => reject(new Error('websocket connection failed'));
    ws.onopen = () => {
      resolve({
        send(method, params = {}) {
          const msgId = ++id;
          ws.send(JSON.stringify({ id: msgId, method, params }));
          return new Promise((resolve, reject) => pending.set(msgId, { resolve, reject }));
        },
        close() {
          ws.close();
        },
      });
    };
  });
}

function netscapeCookieLine(cookie) {
  const domain = cookie.domain || '';
  const includeSubdomains = domain.startsWith('.') ? 'TRUE' : 'FALSE';
  const path = cookie.path || '/';
  const secure = cookie.secure ? 'TRUE' : 'FALSE';
  const expires = cookie.expires && cookie.expires > 0 ? Math.floor(cookie.expires) : 0;
  return [domain, includeSubdomains, path, secure, expires, cookie.name, cookie.value].join('\t');
}

const args = [
  `--remote-debugging-port=${port}`,
  `--user-data-dir=${profileDir}`,
  '--no-first-run',
  '--no-default-browser-check',
  '--new-window',
  loginUrl,
];

const chrome = spawn(chromePath, args, { detached: true, stdio: 'ignore' });
chrome.unref();

const version = await waitJson(`http://127.0.0.1:${port}/json/version`, 15000);
const client = await cdp(version.webSocketDebuggerUrl);
const start = Date.now();
let cookies = [];
let matchedUrl = '';
while (Date.now() - start < timeoutMs) {
  const targets = await waitJson(`http://127.0.0.1:${port}/json/list`, 5000);
  const active = targets.find((target) => target.type === 'page' && target.url && target.url.includes(waitUrlPart) && !target.url.includes('/login'));
  if (active) {
    matchedUrl = active.url;
    const result = await client.send('Storage.getCookies');
    cookies = (result.cookies || []).filter((cookie) => /(^|\.)domainesia\.com$/i.test(cookie.domain));
    if (cookies.length > 0) break;
  }
  await sleep(1000);
}
client.close();

if (cookies.length === 0) {
  throw new Error(`no logged-in domainesia.com cookies captured before timeout; waited for URL containing ${waitUrlPart}`);
}

mkdirSync(dirname(cookieJar), { recursive: true });
writeFileSync(cookieJar, ['# Netscape HTTP Cookie File', ...cookies.map(netscapeCookieLine), ''].join('\n'), { mode: 0o600 });
console.log(JSON.stringify({ cookieJar, cookieCount: cookies.length, profileDir, port, matchedUrl }));
"#;

#[derive(Debug, Clone)]
struct Opts {
    json: bool,
    args: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct Config {
    values: BTreeMap<String, String>,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct DnsRecord {
    domain: String,
    name: String,
    record_type: String,
    value: String,
    ttl: u32,
    priority: Option<u16>,
}

#[derive(Debug, Clone)]
struct DomainInfo {
    id: String,
    domain: String,
    registration_date: String,
    next_due_date: String,
    status: String,
}

#[derive(Debug, Clone)]
struct DnsForm {
    token: String,
    domain_id: String,
    records: Vec<DnsRecord>,
}

fn main() {
    let opts = parse_opts(env::args().skip(1).collect());
    let result = run(&opts);
    if let Err(message) = result {
        emit_error(&opts, current_command_name(&opts.args), &message);
        std::process::exit(1);
    }
}

fn parse_opts(raw: Vec<String>) -> Opts {
    let mut json = false;
    let mut args = Vec::new();
    for arg in raw {
        if arg == "--json" {
            json = true;
        } else {
            args.push(arg);
        }
    }
    Opts { json, args }
}

fn run(opts: &Opts) -> Result<(), String> {
    if opts.args.is_empty() || has_flag(&opts.args, "--help") || has_flag(&opts.args, "-h") {
        print_help();
        return Ok(());
    }
    match opts.args[0].as_str() {
        "doctor" => cmd_doctor(opts),
        "init" => cmd_init(opts),
        "auth" => cmd_auth(opts),
        "features" => cmd_features(opts),
        "domains" => cmd_domains(opts),
        "dns" => cmd_dns(opts),
        "invoices" => cmd_invoices(opts),
        "endpoint" => cmd_endpoint(opts),
        "raw" => cmd_raw(opts),
        "version" => {
            emit_ok(
                opts,
                "version",
                &format!("{{\"version\":\"{}\"}}", json_escape(VERSION)),
            );
            Ok(())
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn cmd_doctor(opts: &Opts) -> Result<(), String> {
    let config = Config::load();
    let curl = command_exists("curl");
    let cookie_jar = config.get("DOMAINESIA_COOKIE_JAR");
    let cookie_readable = cookie_jar.map(|p| Path::new(p).is_file()).unwrap_or(false);
    let endpoint = config.get("DOMAINESIA_DNS_ADD_ENDPOINT").is_some();
    let login_endpoint = config.get("DOMAINESIA_LOGIN_ENDPOINT").is_some();
    let csrf = config.get("DOMAINESIA_CSRF_TOKEN").is_some();
    let body = format!(
        "{{\"version\":\"{}\",\"config_path\":\"{}\",\"config_exists\":{},\"base_url\":\"{}\",\"default_domain\":\"{}\",\"cookie_jar_configured\":{},\"cookie_jar_readable\":{},\"login_endpoint_configured\":{},\"dns_add_endpoint_configured\":{},\"csrf_token_configured\":{},\"curl_available\":{}}}",
        json_escape(VERSION),
        json_escape(&config.path.display().to_string()),
        config.path.exists(),
        json_escape(config.get("DOMAINESIA_BASE_URL").unwrap_or("https://my.domainesia.com")),
        json_escape(config.get("DOMAINESIA_DEFAULT_DOMAIN").unwrap_or("")),
        cookie_jar.is_some(),
        cookie_readable,
        login_endpoint,
        endpoint,
        csrf,
        curl
    );
    emit_ok(opts, "doctor", &body);
    Ok(())
}

fn cmd_init(opts: &Opts) -> Result<(), String> {
    let domain = value_after(&opts.args, "--domain")
        .ok_or("missing --domain, for example: --domain example.my.id")?;
    let base_url = value_after(&opts.args, "--base-url").unwrap_or("https://my.domainesia.com");
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create config directory: {e}"))?;
    }
    let cookie = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cookies.txt")
        .display()
        .to_string();
    let content = format!(
        "DOMAINESIA_BASE_URL={}\nDOMAINESIA_DEFAULT_DOMAIN={}\nDOMAINESIA_COOKIE_JAR={}\n",
        base_url, domain, cookie
    );
    fs::write(&path, content).map_err(|e| format!("failed to write config: {e}"))?;
    emit_ok(
        opts,
        "init",
        &format!(
            "{{\"config_path\":\"{}\",\"default_domain\":\"{}\",\"base_url\":\"{}\"}}",
            json_escape(&path.display().to_string()),
            json_escape(domain),
            json_escape(base_url)
        ),
    );
    Ok(())
}

fn cmd_auth(opts: &Opts) -> Result<(), String> {
    let sub = opts.args.get(1).map(String::as_str).unwrap_or("");
    match sub {
        "status" => cmd_auth_status(opts),
        "open-login" => cmd_auth_open_login(opts),
        "browser-login" => cmd_auth_browser_login(opts),
        "import-cookies" => cmd_auth_import_cookies(opts),
        "configure" => cmd_auth_configure(opts),
        "login" => cmd_auth_login(opts),
        _ => Err(
            "usage: domainesia auth <status|open-login|import-cookies|configure|login> ..."
                .to_string(),
        ),
    }
}

fn cmd_auth_status(opts: &Opts) -> Result<(), String> {
    let config = Config::load();
    let cookie_jar = config.get("DOMAINESIA_COOKIE_JAR");
    let cookie_readable = cookie_jar.map(|p| Path::new(p).is_file()).unwrap_or(false);
    let cookie_size = cookie_jar
        .and_then(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);
    let body = format!(
        "{{\"base_url\":\"{}\",\"cookie_jar_configured\":{},\"cookie_jar_readable\":{},\"cookie_jar_bytes\":{},\"login_endpoint_configured\":{},\"csrf_token_configured\":{}}}",
        json_escape(config.get("DOMAINESIA_BASE_URL").unwrap_or("https://my.domainesia.com")),
        cookie_jar.is_some(),
        cookie_readable,
        cookie_size,
        config.get("DOMAINESIA_LOGIN_ENDPOINT").is_some(),
        config.get("DOMAINESIA_CSRF_TOKEN").is_some()
    );
    emit_ok(opts, "auth status", &body);
    Ok(())
}

fn cmd_auth_open_login(opts: &Opts) -> Result<(), String> {
    let config = Config::load();
    let base_url = config
        .get("DOMAINESIA_BASE_URL")
        .unwrap_or("https://my.domainesia.com");
    let login_url = value_after(&opts.args, "--url").unwrap_or(base_url);
    let opened = if has_flag(&opts.args, "--no-open") {
        false
    } else {
        open_url(login_url)
    };
    emit_ok(
        opts,
        "auth open-login",
        &format!(
            "{{\"login_url\":\"{}\",\"opened\":{},\"next\":\"Log in manually, then export/copy cookies to the configured cookie jar or use auth import-cookies.\"}}",
            json_escape(login_url),
            opened
        ),
    );
    Ok(())
}

fn cmd_auth_browser_login(opts: &Opts) -> Result<(), String> {
    if !command_exists("node") {
        return Err(
            "auth browser-login requires node because it uses Chrome DevTools Protocol".to_string(),
        );
    }
    let config = Config::load();
    let base_url = config
        .get("DOMAINESIA_BASE_URL")
        .unwrap_or("https://my.domainesia.com");
    let login_url = value_after(&opts.args, "--url").unwrap_or(base_url);
    let timeout = value_after(&opts.args, "--timeout-seconds").unwrap_or("180");
    let port = value_after(&opts.args, "--port").unwrap_or("9229");
    let wait_url_part = value_after(&opts.args, "--wait-url-part").unwrap_or("clientarea.php");
    let cookie_jar = value_after(&opts.args, "--cookie-jar")
        .map(PathBuf::from)
        .or_else(|| config.get("DOMAINESIA_COOKIE_JAR").map(PathBuf::from))
        .unwrap_or_else(default_cookie_jar_path);
    let profile_dir = value_after(&opts.args, "--profile-dir")
        .map(PathBuf::from)
        .unwrap_or_else(default_browser_profile_path);
    let script_path = runtime_script_path()?;
    fs::write(&script_path, CDP_CAPTURE_JS)
        .map_err(|e| format!("failed to write browser capture helper: {e}"))?;
    let output = Command::new("node")
        .arg(&script_path)
        .arg(login_url)
        .arg(&cookie_jar)
        .arg(timeout)
        .arg(&profile_dir)
        .arg(port)
        .arg(wait_url_part)
        .output()
        .map_err(|e| format!("failed to run browser capture helper: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("browser login failed: {}{}", stderr, stdout));
    }
    set_owner_only_permissions(&cookie_jar)?;
    let cookie_string = cookie_jar.display().to_string();
    upsert_config_values(&[("DOMAINESIA_COOKIE_JAR", cookie_string.as_str())])?;
    let helper_response = String::from_utf8_lossy(&output.stdout);
    emit_ok(
        opts,
        "auth browser-login",
        &format!(
            "{{\"cookie_jar\":\"{}\",\"helper_response\":{},\"note\":\"Cookie jar captured from dedicated Chrome profile. Run auth status next.\"}}",
            json_escape(&cookie_jar.display().to_string()),
            helper_response.trim()
        ),
    );
    Ok(())
}

fn cmd_auth_import_cookies(opts: &Opts) -> Result<(), String> {
    let source = value_after(&opts.args, "--from").ok_or("missing --from <cookies.txt>")?;
    if !Path::new(source).is_file() {
        return Err("source cookie file is not readable".to_string());
    }
    let config = Config::load();
    let target = value_after(&opts.args, "--to")
        .map(PathBuf::from)
        .or_else(|| config.get("DOMAINESIA_COOKIE_JAR").map(PathBuf::from))
        .unwrap_or_else(|| default_cookie_jar_path());
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create cookie directory: {e}"))?;
    }
    fs::copy(source, &target).map_err(|e| format!("failed to import cookies: {e}"))?;
    set_owner_only_permissions(&target)?;
    let target_string = target.display().to_string();
    upsert_config_values(&[("DOMAINESIA_COOKIE_JAR", target_string.as_str())])?;
    emit_ok(
        opts,
        "auth import-cookies",
        &format!(
            "{{\"cookie_jar\":\"{}\",\"imported\":true}}",
            json_escape(&target.display().to_string())
        ),
    );
    Ok(())
}

fn cmd_auth_configure(opts: &Opts) -> Result<(), String> {
    let mut updates: Vec<(&str, String)> = Vec::new();
    for (flag, key) in [
        ("--login-endpoint", "DOMAINESIA_LOGIN_ENDPOINT"),
        ("--dns-add-endpoint", "DOMAINESIA_DNS_ADD_ENDPOINT"),
        ("--csrf-header", "DOMAINESIA_CSRF_HEADER"),
        ("--csrf-token", "DOMAINESIA_CSRF_TOKEN"),
        ("--cookie-jar", "DOMAINESIA_COOKIE_JAR"),
    ] {
        if let Some(value) = value_after(&opts.args, flag) {
            updates.push((key, value.to_string()));
        }
    }
    if updates.is_empty() {
        return Err("nothing to configure; pass --login-endpoint, --dns-add-endpoint, --csrf-header, --csrf-token, or --cookie-jar".to_string());
    }
    let borrowed = updates
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect::<Vec<_>>();
    upsert_config_values(&borrowed)?;
    let keys = updates
        .iter()
        .map(|(key, _)| format!("\"{}\"", json_escape(key)))
        .collect::<Vec<_>>()
        .join(",");
    emit_ok(
        opts,
        "auth configure",
        &format!("{{\"updated_keys\":[{}]}}", keys),
    );
    Ok(())
}

fn cmd_auth_login(opts: &Opts) -> Result<(), String> {
    let config = Config::load();
    let live = has_flag(&opts.args, "--live");
    let endpoint = value_after(&opts.args, "--endpoint")
        .map(String::from)
        .or_else(|| config.get("DOMAINESIA_LOGIN_ENDPOINT").map(String::from));
    let email = value_after(&opts.args, "--email");
    if !live {
        emit_ok(
            opts,
            "auth login",
            &format!(
                "{{\"mode\":\"dry-run\",\"ready_for_live\":{},\"endpoint_configured\":{},\"email_provided\":{},\"password_source\":\"{}\"}}",
                endpoint.is_some() && email.is_some() && has_flag(&opts.args, "--password-stdin"),
                endpoint.is_some(),
                email.is_some(),
                if has_flag(&opts.args, "--password-stdin") { "stdin" } else { "missing" }
            ),
        );
        return Ok(());
    }
    let endpoint = endpoint.ok_or("live login requires --endpoint or DOMAINESIA_LOGIN_ENDPOINT")?;
    let email = email.ok_or("live login requires --email")?;
    if !has_flag(&opts.args, "--password-stdin") {
        return Err(
            "live login requires --password-stdin so the password is not stored in shell history"
                .to_string(),
        );
    }
    if !command_exists("curl") {
        return Err("curl is required for live login".to_string());
    }
    let cookie_jar = config
        .get("DOMAINESIA_COOKIE_JAR")
        .map(PathBuf::from)
        .unwrap_or_else(default_cookie_jar_path);
    if let Some(parent) = cookie_jar.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create cookie directory: {e}"))?;
    }
    let mut password = String::new();
    io::stdin()
        .read_line(&mut password)
        .map_err(|e| format!("failed to read password from stdin: {e}"))?;
    let password = password.trim_end_matches(&['\r', '\n'][..]).to_string();
    if password.is_empty() {
        return Err("password from stdin was empty".to_string());
    }
    let email_field = value_after(&opts.args, "--email-field").unwrap_or("email");
    let password_field = value_after(&opts.args, "--password-field").unwrap_or("password");
    let payload = format!(
        "{}={}&{}={}",
        url_encode(email_field),
        url_encode(email),
        url_encode(password_field),
        url_encode(&password)
    );
    let mut command = Command::new("curl");
    command
        .arg("--silent")
        .arg("--show-error")
        .arg("--fail-with-body")
        .arg("--location")
        .arg("--cookie-jar")
        .arg(&cookie_jar)
        .arg("--cookie")
        .arg(&cookie_jar)
        .arg("--header")
        .arg("Content-Type: application/x-www-form-urlencoded")
        .arg("--request")
        .arg("POST")
        .arg("--data")
        .arg(payload)
        .arg(endpoint);
    if let (Some(header), Some(token)) = (
        config.get("DOMAINESIA_CSRF_HEADER"),
        config.get("DOMAINESIA_CSRF_TOKEN"),
    ) {
        command.arg("--header").arg(format!("{header}: {token}"));
    }
    let output = command
        .output()
        .map_err(|e| format!("failed to execute curl: {e}"))?;
    drop(password);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("login request failed: {}{}", stderr, stdout));
    }
    set_owner_only_permissions(&cookie_jar)?;
    let cookie_string = cookie_jar.display().to_string();
    upsert_config_values(&[("DOMAINESIA_COOKIE_JAR", cookie_string.as_str())])?;
    let response = String::from_utf8_lossy(&output.stdout);
    emit_ok(
        opts,
        "auth login",
        &format!(
            "{{\"mode\":\"live\",\"cookie_jar\":\"{}\",\"response_bytes\":{},\"response_preview\":\"{}\"}}",
            json_escape(&cookie_jar.display().to_string()),
            response.len(),
            json_escape(&response.chars().take(300).collect::<String>())
        ),
    );
    Ok(())
}

fn cmd_features(opts: &Opts) -> Result<(), String> {
    let sub = opts.args.get(1).map(String::as_str).unwrap_or("list");
    match sub {
        "list" => {
            let features = [
                ("dashboard", "/clientarea.php", "read"),
                ("products", "/dashboard/products", "read"),
                ("domains", "/dashboard/domains", "read"),
                (
                    "domain-details",
                    "/clientarea.php?action=domaindetails&id=<domain_id>",
                    "read",
                ),
                (
                    "dns-management",
                    "/clientarea.php?action=domaindns&domainid=<domain_id>",
                    "read/write",
                ),
                (
                    "nameservers",
                    "/clientarea.php?action=domaindetails&id=<domain_id>#tabNameservers",
                    "read/write",
                ),
                (
                    "private-nameservers",
                    "/clientarea.php?action=domainregisterns&domainid=<domain_id>",
                    "read/write",
                ),
                (
                    "dnssec",
                    "/index.php?m=rnadnssec&id=<domain_id>&domain=<domain>",
                    "read/write",
                ),
                (
                    "domain-forwarding",
                    "/index.php?m=rnadomainforwarding&id=<domain_id>&domain=<domain>",
                    "read/write",
                ),
                (
                    "domain-contacts",
                    "/clientarea.php?action=domaincontacts&domainid=<domain_id>",
                    "read/write",
                ),
                ("invoices", "/dashboard/invoices", "read"),
                ("tickets", "/tickets", "read/write"),
                (
                    "account-details",
                    "/clientarea.php?action=details",
                    "read/write",
                ),
                (
                    "user-management",
                    "/index.php?rp=/account/users",
                    "read/write",
                ),
                ("contacts", "/index.php?rp=/account/contacts", "read/write"),
                ("email-history", "/clientarea.php?action=emails", "read"),
                ("profile", "/index.php?rp=/user/profile", "read/write"),
                ("security", "/index.php?rp=/user/security", "read/write"),
            ];
            let items = features
                .iter()
                .map(|(name, path, access)| {
                    format!(
                        "{{\"name\":\"{}\",\"path\":\"{}\",\"access\":\"{}\"}}",
                        json_escape(name),
                        json_escape(path),
                        json_escape(access)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            emit_ok(
                opts,
                "features list",
                &format!("{{\"features\":[{}]}}", items),
            );
            Ok(())
        }
        "forms" => {
            let path = value_after(&opts.args, "--path").unwrap_or("/clientarea.php");
            let html = http_get_path(path)?;
            let forms = extract_forms_json(&html);
            emit_ok(
                opts,
                "features forms",
                &format!(
                    "{{\"path\":\"{}\",\"forms\":[{}]}}",
                    json_escape(path),
                    forms
                ),
            );
            Ok(())
        }
        _ => Err("usage: domainesia features <list|forms> [--path <path>]".to_string()),
    }
}

fn cmd_domains(opts: &Opts) -> Result<(), String> {
    let sub = opts.args.get(1).map(String::as_str).unwrap_or("");
    match sub {
        "list" => {
            let domains = load_domains()?;
            let items = domains
                .iter()
                .map(domain_json)
                .collect::<Vec<_>>()
                .join(",");
            emit_ok(
                opts,
                "domains list",
                &format!("{{\"count\":{},\"domains\":[{}]}}", domains.len(), items),
            );
            Ok(())
        }
        "resolve" => {
            let config = Config::load();
            let domain_name = value_after(&opts.args, "--domain")
                .or_else(|| config.get("DOMAINESIA_DEFAULT_DOMAIN"))
                .ok_or("missing --domain and no default domain configured")?;
            let domain = resolve_domain(domain_name)?;
            emit_ok(opts, "domains resolve", &domain_json(&domain));
            Ok(())
        }
        "detail" => {
            let domain = resolve_domain_from_args(&opts.args)?;
            let path = format!("/clientarea.php?action=domaindetails&id={}", domain.id);
            let html = http_get_path(&path)?;
            let token = extract_between(&html, "var csrfToken = '", "'").unwrap_or_default();
            emit_ok(
                opts,
                "domains detail",
                &format!(
                    "{{\"domain\":{},\"detail_path\":\"{}\",\"dns_path\":\"/clientarea.php?action=domaindns&domainid={}\",\"csrf_token_present\":{}}}",
                    domain_json(&domain),
                    json_escape(&path),
                    json_escape(&domain.id),
                    !token.is_empty()
                ),
            );
            Ok(())
        }
        _ => Err("usage: domainesia domains <list|resolve|detail> [--domain <domain>]".to_string()),
    }
}

fn cmd_invoices(opts: &Opts) -> Result<(), String> {
    let sub = opts.args.get(1).map(String::as_str).unwrap_or("");
    match sub {
        "list" => {
            let html = http_get_path("/dashboard/invoices")?;
            let invoices = parse_invoices_json(&html);
            emit_ok(
                opts,
                "invoices list",
                &format!("{{\"invoices\":[{}]}}", invoices),
            );
            Ok(())
        }
        _ => Err("usage: domainesia invoices list".to_string()),
    }
}

fn cmd_dns(opts: &Opts) -> Result<(), String> {
    let sub = opts.args.get(1).map(String::as_str).unwrap_or("");
    match sub {
        "list" => cmd_dns_list(opts),
        "plan-add" => {
            let record = parse_record(&opts.args)?;
            emit_ok(opts, "dns plan-add", &record_plan_json(&record));
            Ok(())
        }
        "add" => cmd_dns_add(opts),
        "update" => cmd_dns_update(opts),
        "delete" => cmd_dns_delete(opts),
        _ => Err("usage: domainesia dns <list|plan-add|add|update|delete> ...".to_string()),
    }
}

fn cmd_dns_add(opts: &Opts) -> Result<(), String> {
    if !has_flag(&opts.args, "--endpoint")
        && Config::load().get("DOMAINESIA_DNS_ADD_ENDPOINT").is_none()
    {
        return cmd_dns_form_add(opts);
    }
    let config = Config::load();
    let record = parse_record(&opts.args)?;
    let live = has_flag(&opts.args, "--live");
    let dry_run = has_flag(&opts.args, "--dry-run") || !live;
    let endpoint = value_after(&opts.args, "--endpoint")
        .map(String::from)
        .or_else(|| config.get("DOMAINESIA_DNS_ADD_ENDPOINT").map(String::from));
    if dry_run {
        let body = format!(
            "{{\"mode\":\"dry-run\",\"ready_for_live\":{},\"endpoint_configured\":{},\"record\":{}}}",
            endpoint.is_some(),
            endpoint.is_some(),
            record_json(&record)
        );
        emit_ok(opts, "dns add", &body);
        return Ok(());
    }
    let endpoint = endpoint.ok_or("live add requires --endpoint or DOMAINESIA_DNS_ADD_ENDPOINT")?;
    let cookie_jar = config
        .get("DOMAINESIA_COOKIE_JAR")
        .ok_or("live add requires DOMAINESIA_COOKIE_JAR in config")?;
    if !Path::new(cookie_jar).is_file() {
        return Err("configured cookie jar is not readable".to_string());
    }
    if !command_exists("curl") {
        return Err("curl is required for live requests".to_string());
    }
    let payload = record_json(&record);
    let mut command = Command::new("curl");
    command
        .arg("--silent")
        .arg("--show-error")
        .arg("--fail-with-body")
        .arg("--cookie")
        .arg(cookie_jar)
        .arg("--cookie-jar")
        .arg(cookie_jar)
        .arg("--header")
        .arg("Content-Type: application/json")
        .arg("--request")
        .arg("POST")
        .arg("--data")
        .arg(payload)
        .arg(endpoint);
    if let (Some(header), Some(token)) = (
        config.get("DOMAINESIA_CSRF_HEADER"),
        config.get("DOMAINESIA_CSRF_TOKEN"),
    ) {
        command.arg("--header").arg(format!("{header}: {token}"));
    }
    let output = command
        .output()
        .map_err(|e| format!("failed to execute curl: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("live request failed: {}{}", stderr, stdout));
    }
    let response = String::from_utf8_lossy(&output.stdout);
    emit_ok(
        opts,
        "dns add",
        &format!(
            "{{\"mode\":\"live\",\"record\":{},\"response_preview\":\"{}\"}}",
            record_json(&record),
            json_escape(response.trim())
        ),
    );
    Ok(())
}

fn cmd_dns_list(opts: &Opts) -> Result<(), String> {
    let form = load_dns_form(opts)?;
    emit_ok(
        opts,
        "dns list",
        &format!(
            "{{\"domain_id\":\"{}\",\"record_count\":{},\"records\":[{}]}}",
            json_escape(&form.domain_id),
            form.records.len(),
            records_json(&form.records)
        ),
    );
    Ok(())
}

fn cmd_dns_form_add(opts: &Opts) -> Result<(), String> {
    let mut form = load_dns_form(opts)?;
    let record = parse_record(&opts.args)?;
    let replace = has_flag(&opts.args, "--replace");
    if let Some(existing) = form.records.iter_mut().find(|r| r.name == record.name) {
        if !replace {
            return Err("record name already exists; use dns update or pass --replace".to_string());
        }
        *existing = record.clone();
    } else {
        form.records.push(record.clone());
    }
    submit_or_preview_dns_form(opts, "dns add", form, Some(record))
}

fn cmd_dns_update(opts: &Opts) -> Result<(), String> {
    let mut form = load_dns_form(opts)?;
    let name = value_after(&opts.args, "--name").ok_or("missing --name")?;
    let mut updated = None;
    for record in &mut form.records {
        if record.name == name {
            if let Some(value) = value_after(&opts.args, "--value") {
                record.value = value.to_string();
            }
            if let Some(record_type) = value_after(&opts.args, "--type") {
                record.record_type = record_type.to_uppercase();
            }
            if let Some(priority) = value_after(&opts.args, "--priority") {
                record.priority = priority.parse::<u16>().ok();
            }
            updated = Some(record.clone());
            break;
        }
    }
    let updated = updated.ok_or("record not found")?;
    submit_or_preview_dns_form(opts, "dns update", form, Some(updated))
}

fn cmd_dns_delete(opts: &Opts) -> Result<(), String> {
    let mut form = load_dns_form(opts)?;
    let name = value_after(&opts.args, "--name").ok_or("missing --name")?;
    let before = form.records.len();
    let deleted = form.records.iter().find(|r| r.name == name).cloned();
    form.records.retain(|record| record.name != name);
    if form.records.len() == before {
        return Err("record not found".to_string());
    }
    submit_or_preview_dns_form(opts, "dns delete", form, deleted)
}

fn cmd_endpoint(opts: &Opts) -> Result<(), String> {
    let sub = opts.args.get(1).map(String::as_str).unwrap_or("");
    match sub {
        "import-har" => {
            let path = opts
                .args
                .get(2)
                .ok_or("usage: domainesia endpoint import-har <path.har>")?;
            let text = fs::read_to_string(path).map_err(|e| format!("failed to read HAR: {e}"))?;
            let candidates = extract_endpoint_candidates(&text);
            let items = candidates
                .iter()
                .map(|c| format!("\"{}\"", json_escape(c)))
                .collect::<Vec<_>>()
                .join(",");
            emit_ok(
                opts,
                "endpoint import-har",
                &format!(
                    "{{\"candidate_count\":{},\"candidates\":[{}],\"note\":\"Review candidates manually. Do not store raw cookies or full HAR contents.\"}}",
                    candidates.len(), items
                ),
            );
            Ok(())
        }
        _ => Err("usage: domainesia endpoint import-har <path.har>".to_string()),
    }
}

fn cmd_raw(opts: &Opts) -> Result<(), String> {
    let method = opts.args.get(1).map(String::as_str).unwrap_or("");
    let url = opts.args.get(2).ok_or("usage: domainesia raw get <url>")?;
    if method != "get" {
        return Err("only raw get is supported without explicit DNS command".to_string());
    }
    if !url.starts_with("https://my.domainesia.com")
        && !url.starts_with("https://www.domainesia.com")
    {
        return Err("raw get is restricted to domainesia.com URLs".to_string());
    }
    let config = Config::load();
    let mut command = Command::new("curl");
    command
        .arg("--silent")
        .arg("--show-error")
        .arg("--fail")
        .arg(url);
    if let Some(cookie_jar) = config.get("DOMAINESIA_COOKIE_JAR") {
        if Path::new(cookie_jar).is_file() {
            command.arg("--cookie").arg(cookie_jar);
        }
    }
    let output = command
        .output()
        .map_err(|e| format!("failed to execute curl: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    let response = String::from_utf8_lossy(&output.stdout);
    emit_ok(
        opts,
        "raw get",
        &format!(
            "{{\"url\":\"{}\",\"bytes\":{},\"preview\":\"{}\"}}",
            json_escape(url),
            response.len(),
            json_escape(&response.chars().take(800).collect::<String>())
        ),
    );
    Ok(())
}

fn parse_record(args: &[String]) -> Result<DnsRecord, String> {
    let config = Config::load();
    let domain = value_after(args, "--domain")
        .map(String::from)
        .or_else(|| config.get("DOMAINESIA_DEFAULT_DOMAIN").map(String::from))
        .ok_or("missing --domain and no DOMAINESIA_DEFAULT_DOMAIN configured")?;
    let name = value_after(args, "--name")
        .ok_or("missing --name")?
        .to_string();
    let record_type = value_after(args, "--type")
        .ok_or("missing --type")?
        .to_uppercase();
    if !matches!(record_type.as_str(), "A" | "AAAA" | "CNAME" | "TXT" | "MX") {
        return Err("unsupported --type; use A, AAAA, CNAME, TXT, or MX".to_string());
    }
    let value = value_after(args, "--value")
        .ok_or("missing --value")?
        .to_string();
    let ttl = value_after(args, "--ttl")
        .unwrap_or("3600")
        .parse::<u32>()
        .map_err(|_| "--ttl must be a positive integer")?;
    let priority = value_after(args, "--priority")
        .map(|v| {
            v.parse::<u16>()
                .map_err(|_| "--priority must be an integer")
        })
        .transpose()?;
    Ok(DnsRecord {
        domain,
        name,
        record_type,
        value,
        ttl,
        priority,
    })
}

fn load_domains() -> Result<Vec<DomainInfo>, String> {
    let html = http_get_path("/dashboard/domains")?;
    Ok(parse_domains(&html))
}

fn resolve_domain_from_args(args: &[String]) -> Result<DomainInfo, String> {
    let config = Config::load();
    let domain_name = value_after(args, "--domain")
        .or_else(|| config.get("DOMAINESIA_DEFAULT_DOMAIN"))
        .ok_or("missing --domain and no default domain configured")?;
    resolve_domain(domain_name)
}

fn resolve_domain(domain_name: &str) -> Result<DomainInfo, String> {
    load_domains()?
        .into_iter()
        .find(|domain| domain.domain == domain_name || domain.id == domain_name)
        .ok_or_else(|| format!("domain not found: {domain_name}"))
}

fn load_dns_form(opts: &Opts) -> Result<DnsForm, String> {
    let domain = resolve_domain_from_args(&opts.args)?;
    let html = http_get_path(&format!(
        "/clientarea.php?action=domaindns&domainid={}",
        domain.id
    ))?;
    parse_dns_form(&html, &domain.domain).ok_or("failed to parse DNS management form".to_string())
}

fn submit_or_preview_dns_form(
    opts: &Opts,
    command_name: &str,
    form: DnsForm,
    changed: Option<DnsRecord>,
) -> Result<(), String> {
    let live = has_flag(&opts.args, "--live");
    let body = dns_form_body(&form);
    if !live {
        let changed_json = changed
            .as_ref()
            .map(record_json)
            .unwrap_or_else(|| "null".to_string());
        emit_ok(
            opts,
            command_name,
            &format!(
                "{{\"mode\":\"dry-run\",\"domain_id\":\"{}\",\"record_count\":{},\"changed\":{},\"post_path\":\"/clientarea.php?action=domaindns\"}}",
                json_escape(&form.domain_id),
                form.records.len(),
                changed_json
            ),
        );
        return Ok(());
    }
    let response = http_post_form("/clientarea.php?action=domaindns", &body)?;
    let success = response.contains("DNS Management") || response.contains("Changes Saved");
    emit_ok(
        opts,
        command_name,
        &format!(
            "{{\"mode\":\"live\",\"domain_id\":\"{}\",\"record_count\":{},\"success_hint\":{},\"response_bytes\":{}}}",
            json_escape(&form.domain_id),
            form.records.len(),
            success,
            response.len()
        ),
    );
    Ok(())
}

impl Config {
    fn load() -> Config {
        let path = config_path();
        let mut values = BTreeMap::new();
        if let Ok(text) = fs::read_to_string(&path) {
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = trimmed.split_once('=') {
                    values.insert(key.trim().to_string(), value.trim().to_string());
                }
            }
        }
        for key in [
            "DOMAINESIA_BASE_URL",
            "DOMAINESIA_DEFAULT_DOMAIN",
            "DOMAINESIA_COOKIE_JAR",
            "DOMAINESIA_LOGIN_ENDPOINT",
            "DOMAINESIA_DNS_ADD_ENDPOINT",
            "DOMAINESIA_CSRF_HEADER",
            "DOMAINESIA_CSRF_TOKEN",
        ] {
            if let Ok(value) = env::var(key) {
                values.insert(key.to_string(), value);
            }
        }
        Config { values, path }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

fn config_path() -> PathBuf {
    if let Ok(path) = env::var("DOMAINESIA_CONFIG") {
        return PathBuf::from(path);
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".domainesia").join("config.env")
}

fn default_cookie_jar_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".domainesia").join("cookies.txt")
}

fn default_browser_profile_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".domainesia")
        .join("chrome-profile")
}

fn runtime_script_path() -> Result<PathBuf, String> {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".domainesia");
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create runtime directory: {e}"))?;
    Ok(dir.join("cdp-cookie-capture.mjs"))
}

fn upsert_config_values(updates: &[(&str, &str)]) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create config directory: {e}"))?;
    }
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut values = BTreeMap::new();
    let mut order = Vec::new();
    for line in existing.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            if !values.contains_key(&key) {
                order.push(key.clone());
            }
            values.insert(key, value.trim().to_string());
        }
    }
    for (key, value) in updates {
        if !values.contains_key(*key) {
            order.push((*key).to_string());
        }
        values.insert((*key).to_string(), (*value).to_string());
    }
    let mut content = String::new();
    for key in order {
        if let Some(value) = values.get(&key) {
            content.push_str(&key);
            content.push('=');
            content.push_str(value);
            content.push('\n');
        }
    }
    fs::write(&path, content).map_err(|e| format!("failed to write config: {e}"))?;
    Ok(())
}

fn set_owner_only_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions)
            .map_err(|e| format!("failed to lock down file permissions: {e}"))?;
    }
    let _ = path;
    Ok(())
}

fn http_get_path(path_or_url: &str) -> Result<String, String> {
    let config = Config::load();
    let cookie_jar = config
        .get("DOMAINESIA_COOKIE_JAR")
        .ok_or("DOMAINESIA_COOKIE_JAR is not configured")?;
    if !Path::new(cookie_jar).is_file() {
        return Err("configured cookie jar is not readable".to_string());
    }
    let url = absolute_domainesia_url(path_or_url)?;
    let output = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--fail-with-body")
        .arg("--location")
        .arg("--cookie")
        .arg(cookie_jar)
        .arg(url)
        .output()
        .map_err(|e| format!("failed to execute curl: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "GET failed: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn http_post_form(path_or_url: &str, body: &str) -> Result<String, String> {
    let config = Config::load();
    let cookie_jar = config
        .get("DOMAINESIA_COOKIE_JAR")
        .ok_or("DOMAINESIA_COOKIE_JAR is not configured")?;
    if !Path::new(cookie_jar).is_file() {
        return Err("configured cookie jar is not readable".to_string());
    }
    let url = absolute_domainesia_url(path_or_url)?;
    let output = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--fail-with-body")
        .arg("--location")
        .arg("--cookie")
        .arg(cookie_jar)
        .arg("--cookie-jar")
        .arg(cookie_jar)
        .arg("--header")
        .arg("Content-Type: application/x-www-form-urlencoded")
        .arg("--request")
        .arg("POST")
        .arg("--data")
        .arg(body)
        .arg(url)
        .output()
        .map_err(|e| format!("failed to execute curl: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "POST failed: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn absolute_domainesia_url(path_or_url: &str) -> Result<String, String> {
    if path_or_url.starts_with("https://my.domainesia.com") {
        Ok(path_or_url.to_string())
    } else if path_or_url.starts_with('/') {
        Ok(format!("https://my.domainesia.com{path_or_url}"))
    } else {
        Err("URL/path must target my.domainesia.com".to_string())
    }
}

fn open_url(url: &str) -> bool {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "cmd"
    } else {
        "xdg-open"
    };
    let status = if cfg!(target_os = "windows") {
        Command::new(opener).args(["/C", "start", url]).status()
    } else {
        Command::new(opener).arg(url).status()
    };
    status.map(|s| s.success()).unwrap_or(false)
}

fn url_encode(input: &str) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn extract_endpoint_candidates(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let needles = ["dns", "record", "zone", "domain"];
    for part in text.split('"') {
        if !part.starts_with("http") {
            continue;
        }
        let lower = part.to_ascii_lowercase();
        if lower.contains("domainesia") && needles.iter().any(|needle| lower.contains(needle)) {
            let clean = part.replace("\\/", "/");
            if !candidates.contains(&clean) {
                candidates.push(clean);
            }
        }
    }
    candidates.truncate(30);
    candidates
}

fn parse_domains(html: &str) -> Vec<DomainInfo> {
    let mut domains = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("data-element-id=\"") {
        rest = &rest[pos + "data-element-id=\"".len()..];
        let Some(id) = rest.split('"').next() else {
            break;
        };
        let Some(domain_pos) = rest.find("data-domain=\"") else {
            continue;
        };
        let after_domain = &rest[domain_pos + "data-domain=\"".len()..];
        let Some(domain) = after_domain.split('"').next() else {
            continue;
        };
        let window = &rest[..rest.find("</tr>").unwrap_or(rest.len()).min(rest.len())];
        let hidden_dates = extract_all_between(window, "<span class=\"w-hidden\">", "</span>");
        let registration_date = hidden_dates.first().cloned().unwrap_or_default();
        let next_due_date = hidden_dates.get(1).cloned().unwrap_or_default();
        let status = extract_between(window, "status status-", "\"")
            .unwrap_or_default()
            .replace("active", "Active")
            .replace("expired", "Expired");
        domains.push(DomainInfo {
            id: id.to_string(),
            domain: domain.to_string(),
            registration_date,
            next_due_date,
            status,
        });
    }
    domains
}

fn parse_dns_form(html: &str, domain: &str) -> Option<DnsForm> {
    let token = extract_between(html, "name=\"token\" value=\"", "\"")?;
    let domain_id = extract_between(html, "name=\"domainid\" value=\"", "\"")?;
    let table = extract_between(html, "<tbody>", "</tbody>").unwrap_or_default();
    let mut records = Vec::new();
    for row in table.split("<tr").skip(1) {
        if !row.contains("dnsrecordhost[]") {
            continue;
        }
        let host = input_value_after_name(row, "dnsrecordhost[]");
        let address = input_value_after_name(row, "dnsrecordaddress[]");
        if host.trim().is_empty() && address.trim().is_empty() {
            continue;
        }
        let record_type = selected_option_value(row).unwrap_or_else(|| "A".to_string());
        let priority_raw = input_value_after_name(row, "dnsrecordpriority[]");
        let priority = priority_raw.parse::<u16>().ok();
        records.push(DnsRecord {
            domain: domain.to_string(),
            name: html_unescape(&host),
            record_type,
            value: html_unescape(&address),
            ttl: 3600,
            priority,
        });
    }
    Some(DnsForm {
        token,
        domain_id,
        records,
    })
}

fn parse_invoices_json(html: &str) -> String {
    let table = extract_between(html, "<tbody>", "</tbody>").unwrap_or_default();
    let mut invoices = Vec::new();
    for row in table.split("<tr").skip(1) {
        if !row.contains("invoice?id=") {
            continue;
        }
        let id = extract_between(row, "invoice?id=", "'").unwrap_or_default();
        let cells = extract_all_between(row, "<td", "</td>");
        let invoice_date = cells.get(1).map(|s| clean_html(s)).unwrap_or_default();
        let due_date = cells.get(2).map(|s| clean_html(s)).unwrap_or_default();
        let total = cells.get(3).map(|s| clean_html(s)).unwrap_or_default();
        let status = cells.get(4).map(|s| clean_html(s)).unwrap_or_default();
        invoices.push(format!(
            "{{\"id\":\"{}\",\"invoice_date\":\"{}\",\"due_date\":\"{}\",\"total\":\"{}\",\"status\":\"{}\",\"path\":\"/invoice?id={}\"}}",
            json_escape(&id),
            json_escape(&invoice_date),
            json_escape(&due_date),
            json_escape(&total),
            json_escape(&status),
            json_escape(&id)
        ));
    }
    invoices.join(",")
}

fn extract_forms_json(html: &str) -> String {
    let mut forms = Vec::new();
    for form in html.split("<form").skip(1) {
        let attrs = form.split('>').next().unwrap_or("");
        let body = form.split("</form>").next().unwrap_or("");
        let method = attr_value(attrs, "method").unwrap_or_else(|| "get".to_string());
        let action = attr_value(attrs, "action").unwrap_or_default();
        let names = extract_all_between(body, "name=\"", "\"")
            .into_iter()
            .map(|name| format!("\"{}\"", json_escape(&name)))
            .collect::<Vec<_>>()
            .join(",");
        forms.push(format!(
            "{{\"method\":\"{}\",\"action\":\"{}\",\"field_names\":[{}]}}",
            json_escape(&method),
            json_escape(&html_unescape(&action)),
            names
        ));
    }
    forms.join(",")
}

fn dns_form_body(form: &DnsForm) -> String {
    let mut pairs = vec![
        ("token".to_string(), form.token.clone()),
        ("sub".to_string(), "save".to_string()),
        ("domainid".to_string(), form.domain_id.clone()),
    ];
    for record in &form.records {
        pairs.push(("dnsrecid[]".to_string(), "".to_string()));
        pairs.push(("dnsrecordhost[]".to_string(), record.name.clone()));
        pairs.push(("dnsrecordtype[]".to_string(), record.record_type.clone()));
        pairs.push(("dnsrecordaddress[]".to_string(), record.value.clone()));
        pairs.push((
            "dnsrecordpriority[]".to_string(),
            record
                .priority
                .map(|p| p.to_string())
                .unwrap_or_else(|| "N/A".to_string()),
        ));
    }
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", url_encode(key), url_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn record_plan_json(record: &DnsRecord) -> String {
    format!(
        "{{\"fqdn\":\"{}\",\"record\":{},\"steps\":[\"Open MyDomaiNesia DNS Management for the domain\",\"Create or edit the matching record\",\"Verify propagation after save\"],\"safe_to_run_live\":false}}",
        json_escape(&fqdn(record)),
        record_json(record)
    )
}

fn records_json(records: &[DnsRecord]) -> String {
    records
        .iter()
        .map(record_json)
        .collect::<Vec<_>>()
        .join(",")
}

fn record_json(record: &DnsRecord) -> String {
    let priority = record
        .priority
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"domain\":\"{}\",\"name\":\"{}\",\"fqdn\":\"{}\",\"type\":\"{}\",\"value\":\"{}\",\"ttl\":{},\"priority\":{}}}",
        json_escape(&record.domain),
        json_escape(&record.name),
        json_escape(&fqdn(record)),
        json_escape(&record.record_type),
        json_escape(&record.value),
        record.ttl,
        priority
    )
}

fn domain_json(domain: &DomainInfo) -> String {
    format!(
        "{{\"id\":\"{}\",\"domain\":\"{}\",\"registration_date\":\"{}\",\"next_due_date\":\"{}\",\"status\":\"{}\",\"detail_path\":\"/clientarea.php?action=domaindetails&id={}\",\"dns_path\":\"/clientarea.php?action=domaindns&domainid={}\"}}",
        json_escape(&domain.id),
        json_escape(&domain.domain),
        json_escape(&domain.registration_date),
        json_escape(&domain.next_due_date),
        json_escape(&domain.status),
        json_escape(&domain.id),
        json_escape(&domain.id)
    )
}

fn fqdn(record: &DnsRecord) -> String {
    if record.name == "@" || record.name == record.domain {
        record.domain.clone()
    } else if record.name.ends_with(&record.domain) {
        record.name.clone()
    } else {
        format!("{}.{}", record.name.trim_end_matches('.'), record.domain)
    }
}

fn print_help() {
    println!(
        "domainesia {VERSION}\n\nUSAGE:\n  domainesia [--json] <command>\n\nCOMMANDS:\n  doctor                         Check config and local prerequisites\n  init --domain <domain>          Write ~/.domainesia/config.env\n  auth status                     Check local auth material\n  auth open-login                 Open MyDomaiNesia login in a browser\n  auth browser-login              Launch Chrome, wait for login, capture cookies\n  auth import-cookies --from <f>  Copy browser-exported cookies into config\n  auth configure ...              Store login/DNS endpoints or CSRF settings\n  auth login ...                  Endpoint-driven login; dry-run by default\n  features list|forms             Inventory MyDomaiNesia routes/forms\n  domains list|resolve|detail     Read domain portfolio and IDs\n  dns list                        List DNS records for a domain\n  dns plan-add ...                Print a DNS add plan\n  dns add ... [--dry-run|--live]  Add a DNS record via DNS Management form\n  dns update ... [--dry-run|--live] Update a DNS record by host name\n  dns delete ... [--dry-run|--live] Delete a DNS record by host name\n  invoices list                   List invoices\n  endpoint import-har <file>      Extract endpoint candidates from a local HAR\n  raw get <url>                   Read-only GET for domainesia.com URLs\n  version                         Print version\n\nAUTH FLAGS:\n  auth browser-login [--timeout-seconds 180] [--port 9229] [--wait-url-part clientarea.php]\n  auth login --email <email> --password-stdin [--endpoint <url>] [--live]\n  auth configure [--login-endpoint <url>] [--dns-add-endpoint <url>] [--csrf-header <name>] [--csrf-token <token>]\n\nDNS FLAGS:\n  --domain <domain> --name <name> --type <A|AAAA|CNAME|TXT|MX> --value <value> [--ttl 3600] [--priority n]\n"
    );
}

fn emit_ok(opts: &Opts, command: &str, data: &str) {
    if opts.json {
        println!(
            "{{\"ok\":true,\"command\":\"{}\",\"data\":{}}}",
            json_escape(command),
            data
        );
    } else {
        println!("{data}");
    }
}

fn emit_error(opts: &Opts, command: String, message: &str) {
    if opts.json {
        let _ = writeln!(
            io::stderr(),
            "{{\"ok\":false,\"command\":\"{}\",\"error\":{{\"message\":\"{}\"}}}}",
            json_escape(&command),
            json_escape(message)
        );
    } else {
        let _ = writeln!(io::stderr(), "error: {message}");
    }
}

fn current_command_name(args: &[String]) -> String {
    args.iter().take(2).cloned().collect::<Vec<_>>().join(" ")
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

fn extract_between(input: &str, start: &str, end: &str) -> Option<String> {
    let after_start = input.split_once(start)?.1;
    Some(after_start.split_once(end)?.0.to_string())
}

fn extract_all_between(input: &str, start: &str, end: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = input;
    while let Some((_, after_start)) = rest.split_once(start) {
        if let Some((value, after_end)) = after_start.split_once(end) {
            values.push(value.to_string());
            rest = after_end;
        } else {
            break;
        }
    }
    values
}

fn attr_value(input: &str, name: &str) -> Option<String> {
    extract_between(input, &format!("{name}=\""), "\"")
        .or_else(|| extract_between(input, &format!("{name}='"), "'"))
}

fn input_value_after_name(row: &str, name: &str) -> String {
    let Some(pos) = row.find(&format!("name=\"{name}\"")) else {
        return String::new();
    };
    let tag_start = row[..pos].rfind("<input").unwrap_or(pos);
    let tag_end = row[pos..]
        .find('>')
        .map(|end| pos + end)
        .unwrap_or(row.len());
    attr_value(&row[tag_start..tag_end], "value").unwrap_or_default()
}

fn selected_option_value(row: &str) -> Option<String> {
    let selected_pos = row.find("selected=\"selected\"")?;
    let before = &row[..selected_pos];
    let option_pos = before.rfind("<option")?;
    attr_value(&before[option_pos..], "value")
}

fn clean_html(input: &str) -> String {
    let visible_part = if input.contains("w-hidden") && input.contains("</span>") {
        input
            .rsplit_once("</span>")
            .map(|(_, tail)| tail)
            .unwrap_or(input)
    } else if input.starts_with(' ') || input.contains('>') {
        input.split_once('>').map(|(_, tail)| tail).unwrap_or(input)
    } else {
        input
    };
    let without_tags = strip_tags(visible_part);
    html_unescape(&without_tags)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_tags(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn html_unescape(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

fn command_exists(name: &str) -> bool {
    env::var_os("PATH")
        .and_then(|paths| {
            env::split_paths(&paths)
                .map(|path| path.join(name))
                .find(|candidate| candidate.is_file())
        })
        .is_some()
}

fn json_escape(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_fqdn_from_short_name() {
        let record = DnsRecord {
            domain: "example.my.id".to_string(),
            name: "10router".to_string(),
            record_type: "A".to_string(),
            value: "203.0.113.10".to_string(),
            ttl: 3600,
            priority: None,
        };
        assert_eq!(fqdn(&record), "10router.example.my.id");
    }

    #[test]
    fn extracts_endpoint_candidates() {
        let text = r#"{"url":"https:\/\/my.domainesia.com\/clientarea\/dns\/records\/123"}"#;
        let candidates = extract_endpoint_candidates(text);
        assert_eq!(
            candidates,
            vec!["https://my.domainesia.com/clientarea/dns/records/123"]
        );
    }

    #[test]
    fn escapes_json() {
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
    }

    #[test]
    fn url_encodes_form_values() {
        assert_eq!(url_encode("a b+c@example.com"), "a+b%2Bc%40example.com");
    }
}
