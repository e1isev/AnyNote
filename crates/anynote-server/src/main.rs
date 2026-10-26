//! AnyNote in the browser.
//!
//! One self-contained program for machines that can't run the desktop app
//! (for example a Raspberry Pi on Raspbian 11, whose WebKitGTK is too old for
//! Tauri 2). It serves the app's UI, which is built into the binary, and the
//! same commands the desktop app uses, then opens the UI in Chromium as an
//! app window. Notes live in the same data folder as the desktop app.
//!
//! ```text
//! anynote-server [--port 47878] [--host 127.0.0.1] [--data-dir DIR] [--no-browser]
//! ```

mod dispatch;

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anynote_core::{paths::default_data_dir, Store};
use include_dir::{include_dir, Dir};
use serde_json::{json, Map, Value};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::dispatch::{dispatch, Broadcast, Reply};

static UI: Dir = include_dir!("$ANYNOTE_UI_DIR");
static ICON: &[u8] = include_bytes!("../../../app/src-tauri/icons/128x128.png");

const DEFAULT_PORT: u16 = 47878;
const KEPT_EVENTS: usize = 512;
const POLL_TIMEOUT: Duration = Duration::from_secs(25);

struct Options {
    host: IpAddr,
    port: u16,
    data_dir: PathBuf,
    browser: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut o =
        Options { host: IpAddr::from([127, 0, 0, 1]), port: DEFAULT_PORT, data_dir: default_data_dir(), browser: true };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--port" => o.port = value("--port")?.parse().map_err(|_| "bad --port")?,
            "--host" => o.host = value("--host")?.parse().map_err(|_| "bad --host")?,
            "--data-dir" => o.data_dir = PathBuf::from(value("--data-dir")?),
            "--no-browser" => o.browser = false,
            "--version" | "-V" => {
                println!("anynote-server {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "AnyNote in your web browser.\n\n\
                     Usage: anynote-server [--port N] [--host IP] [--data-dir DIR] [--no-browser]\n\n\
                     Opens AnyNote in Chromium (or your default browser). Notes are kept in\n\
                     {} unless --data-dir is given.\n\
                     --host 0.0.0.0 makes it reachable from other devices on your network (no password!).",
                    default_data_dir().display()
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other} (try --help)")),
        }
    }
    Ok(o)
}

/// Page updates for other windows, read by long-polling `/api/events`.
struct Events {
    log: Mutex<(u64, VecDeque<(u64, String)>)>,
    changed: Condvar,
}

impl Events {
    fn push(&self, b: Broadcast) {
        let payload = json!({ "pageId": b.page_id, "update": b.update }).to_string();
        let mut log = self.log.lock().unwrap();
        let seq = log.0;
        log.0 += 1;
        log.1.push_back((seq, payload));
        while log.1.len() > KEPT_EVENTS {
            log.1.pop_front();
        }
        self.changed.notify_all();
    }

    /// Events numbered `since` or later, waiting up to `timeout` for one.
    fn wait(&self, since: i64, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        let mut log = self.log.lock().unwrap();
        loop {
            let next = log.0;
            if since < 0 || (next as i64) > since || Instant::now() >= deadline {
                let events: Vec<&str> = log
                    .1
                    .iter()
                    .filter(|(seq, _)| since >= 0 && *seq as i64 >= since)
                    .map(|(_, e)| e.as_str())
                    .collect();
                return format!("{{\"next\":{next},\"events\":[{}]}}", events.join(","));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            log = self.changed.wait_timeout(log, left).unwrap().0;
        }
    }
}

struct App {
    store: Mutex<Store>,
    data_dir: PathBuf,
    events: Events,
    /// `Host` values of this machine's own browser (DNS-rebinding protection).
    hosts: Vec<String>,
    /// Phones and tablets on the local network may connect (with the key).
    phone_enabled: AtomicBool,
    /// Port of the local-network listener, once it runs.
    phone_port: Mutex<Option<u16>>,
    /// Secret that other devices need; it travels in the QR-code link and
    /// is then kept in a cookie.
    key: String,
    port: u16,
}

/// Is this request allowed, and does it need a redirect that stores the key?
enum Access {
    Allowed,
    StoreKey(String),
    Denied,
}

impl App {
    fn access(&self, req: &Request, path: &str, query: &str) -> Access {
        let loopback = req.remote_addr().is_some_and(|a| a.ip().is_loopback());
        let own_host = header(req, "host").is_some_and(|h| self.hosts.contains(&h));
        if loopback && own_host {
            return Access::Allowed;
        }
        if !self.phone_enabled.load(Ordering::Relaxed) {
            return Access::Denied;
        }
        if query.split('&').any(|kv| kv.strip_prefix("key=") == Some(self.key.as_str())) {
            return Access::StoreKey(path.to_string());
        }
        let cookie = header(req, "cookie").unwrap_or_default();
        let has_key = cookie.split(';').any(|c| c.trim().strip_prefix("anynote_key=") == Some(self.key.as_str()));
        if has_key {
            Access::Allowed
        } else {
            Access::Denied
        }
    }

    /// Turn access from other devices on or off; starts the listener once.
    fn set_phone_access(self: &Arc<Self>, enable: bool) -> Result<(), String> {
        let flag = self.data_dir.join("phone-access");
        if enable {
            let mut port = self.phone_port.lock().unwrap();
            if port.is_none() {
                let lan_port = self.port + 1;
                let server = Server::http(("0.0.0.0", lan_port))
                    .map_err(|e| format!("cannot listen on port {lan_port} for phones: {e}"))?;
                let app = self.clone();
                std::thread::spawn(move || {
                    for request in server.incoming_requests() {
                        let app = app.clone();
                        std::thread::spawn(move || handle(&app, request));
                    }
                });
                *port = Some(lan_port);
            }
            let _ = std::fs::write(&flag, b"on\n");
        } else {
            let _ = std::fs::remove_file(&flag);
        }
        self.phone_enabled.store(enable, Ordering::Relaxed);
        Ok(())
    }

    fn phone_status(&self) -> Value {
        let enabled = self.phone_enabled.load(Ordering::Relaxed);
        let port = *self.phone_port.lock().unwrap();
        let urls: Vec<String> = match (enabled, port) {
            (true, Some(port)) => {
                lan_addresses().iter().map(|ip| format!("http://{ip}:{port}/?key={}", self.key)).collect()
            }
            _ => Vec::new(),
        };
        json!({ "enabled": enabled, "urls": urls })
    }
}

/// The key other devices need, created once and kept in the data folder.
fn load_or_create_key(data_dir: &Path) -> String {
    let path = data_dir.join("phone-key");
    if let Ok(k) = std::fs::read_to_string(&path) {
        let k = k.trim();
        if k.len() >= 10 && k.chars().all(|c| c.is_ascii_alphanumeric()) {
            return k.to_string();
        }
    }
    // The tail of a UUIDv7 is random; two of them give 80 random bits.
    let a = anynote_core::ids::new_id().replace('-', "");
    let b = anynote_core::ids::new_id().replace('-', "");
    let key = format!("{}{}", &a[a.len() - 10..], &b[b.len() - 10..]);
    let _ = std::fs::write(&path, &key);
    key
}

/// This machine's IPv4 addresses on the local network.
fn lan_addresses() -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = Vec::new();
    // The address used to reach the outside world (no packet is sent).
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("192.0.2.1:9").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                out.push(addr.ip());
            }
        }
    }
    // Every address, for networks without a default route (e.g. a phone hotspot
    // without data): `hostname -I` lists them on Raspberry Pi OS and most Linux.
    if let Ok(o) = Command::new("hostname").arg("-I").output() {
        for ip in String::from_utf8_lossy(&o.stdout).split_whitespace().filter_map(|s| s.parse::<IpAddr>().ok()) {
            if !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out.retain(|ip| ip.is_ipv4() && !ip.is_loopback() && !ip.is_unspecified());
    out
}

fn main() {
    let opts = parse_args().unwrap_or_else(|e| {
        eprintln!("anynote-server: {e}");
        std::process::exit(2);
    });
    let url = format!(
        "http://{}:{}/",
        if opts.host.is_unspecified() { IpAddr::from([127, 0, 0, 1]) } else { opts.host },
        opts.port
    );

    let server = match Server::http((opts.host, opts.port)) {
        Ok(s) => s,
        Err(e) => {
            // Already running? Then just open another window.
            if already_running(opts.port) {
                println!("AnyNote is already running at {url}");
                if opts.browser {
                    open_browser(&url, &opts.data_dir);
                }
                return;
            }
            eprintln!("anynote-server: cannot listen on {}:{}: {e}", opts.host, opts.port);
            std::process::exit(1);
        }
    };

    let mut store = Store::open(&opts.data_dir).unwrap_or_else(|e| {
        eprintln!("anynote-server: cannot open {}: {e}", opts.data_dir.display());
        std::process::exit(1);
    });
    if let Err(e) = anynote_core::welcome::seed_welcome(&mut store) {
        eprintln!("anynote-server: could not create the welcome notebook: {e}");
    }
    let hosts = vec![format!("127.0.0.1:{}", opts.port), format!("localhost:{}", opts.port)];
    let app = Arc::new(App {
        store: Mutex::new(store),
        data_dir: opts.data_dir.clone(),
        events: Events { log: Mutex::new((0, VecDeque::new())), changed: Condvar::new() },
        hosts,
        // Listening beyond this machine (--host) means other devices are expected.
        phone_enabled: AtomicBool::new(!opts.host.is_loopback()),
        phone_port: Mutex::new(None),
        key: load_or_create_key(&opts.data_dir),
        port: opts.port,
    });
    if opts.data_dir.join("phone-access").exists() {
        if let Err(e) = app.set_phone_access(true) {
            eprintln!("anynote-server: {e}");
        }
    }
    if !opts.host.is_loopback() {
        println!("Other devices: http://<this computer's address>:{}/?key={}", opts.port, app.key);
    }

    println!("AnyNote is running at {url}");
    println!("Notes are stored in {}", opts.data_dir.display());
    println!("Close this window (or press Ctrl+C) to stop AnyNote.");
    install_menu_entry();
    if opts.browser {
        open_browser(&url, &opts.data_dir);
    }

    for request in server.incoming_requests() {
        let app = app.clone();
        std::thread::spawn(move || handle(&app, request));
    }
}

fn header(req: &Request, name: &str) -> Option<String> {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

fn respond(req: Request, status: u16, content_type: &str, body: Vec<u8>, cache: &str) {
    let mut res = Response::from_data(body).with_status_code(StatusCode(status));
    for (k, v) in [
        ("Content-Type", content_type),
        ("Cache-Control", cache),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "no-referrer"),
    ] {
        if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            res.add_header(h);
        }
    }
    let _ = req.respond(res);
}

fn text(req: Request, status: u16, msg: &str) {
    respond(req, status, "text/plain; charset=utf-8", msg.as_bytes().to_vec(), "no-store");
}

fn handle(app: &Arc<App>, mut req: Request) {
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let path = path.to_string();

    // This machine's own browser may always connect (when it addresses us by
    // name, which defeats DNS rebinding); other devices need the key.
    match app.access(&req, &path, query) {
        Access::Allowed => {}
        Access::StoreKey(to) => {
            let mut res = Response::empty(StatusCode(302));
            for (k, v) in [
                ("Location", to.as_str()),
                ("Set-Cookie", &format!("anynote_key={}; Path=/; Max-Age=31536000; SameSite=Lax; HttpOnly", app.key)),
                ("Cache-Control", "no-store"),
            ] {
                if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                    res.add_header(h);
                }
            }
            let _ = req.respond(res);
            return;
        }
        Access::Denied => {
            return text(
                req,
                403,
                "AnyNote: open the link (or scan the QR code) shown in AnyNote under Settings > Use on phone or tablet.",
            );
        }
    }

    if let Some(command) = path.strip_prefix("/api/") {
        // API calls must carry our header (cross-site requests can't without
        // a CORS preflight, which we never approve) and a same-origin Origin.
        let same_origin = match (header(&req, "origin"), header(&req, "host")) {
            (Some(origin), Some(host)) => origin == format!("http://{host}"),
            (None, _) => true,
            _ => false,
        };
        if header(&req, "x-anynote").as_deref() != Some("1") || !same_origin {
            return text(req, 403, "forbidden");
        }
        if command == "phone_access" {
            let mut body = Vec::new();
            let _ = req.as_reader().read_to_end(&mut body);
            let enable = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v.get("enable")?.as_bool());
            // Only this machine may switch access for other devices.
            let local = req.remote_addr().is_some_and(|a| a.ip().is_loopback());
            if let Some(enable) = enable.filter(|_| local) {
                if let Err(e) = app.set_phone_access(enable) {
                    return text(req, 500, &e);
                }
            }
            return respond(req, 200, "application/json", app.phone_status().to_string().into_bytes(), "no-store");
        }
        if command == "events" {
            let since = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("since="))
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(-1);
            let body = app.events.wait(since, POLL_TIMEOUT);
            return respond(req, 200, "application/json", body.into_bytes(), "no-store");
        }
        if *req.method() != Method::Post {
            return text(req, 405, "use POST");
        }
        let mut body = Vec::new();
        if let Err(e) = req.as_reader().read_to_end(&mut body) {
            return text(req, 400, &e.to_string());
        }
        let is_json = header(&req, "content-type").is_some_and(|t| t.starts_with("application/json"));
        let args: Map<String, Value> = if is_json && !body.is_empty() {
            match serde_json::from_slice(&body) {
                Ok(Value::Object(m)) => m,
                _ => return text(req, 400, "arguments must be a JSON object"),
            }
        } else {
            Map::new()
        };
        let header_of = |name: &str| header(&req, name);
        let command = command.to_string();
        let mut broadcasts = Vec::new();
        let result = {
            let mut store = app.store.lock().unwrap_or_else(|e| e.into_inner());
            let request = dispatch::Request {
                command: &command,
                args,
                body: if is_json { Vec::new() } else { body },
                header: &header_of,
            };
            dispatch(&mut store, &app.data_dir, request, &mut broadcasts)
        };
        for b in broadcasts {
            app.events.push(b);
        }
        return match result {
            Ok(Reply::Json(v)) => respond(req, 200, "application/json", v.to_string().into_bytes(), "no-store"),
            Ok(Reply::Bytes(b)) => respond(req, 200, "application/octet-stream", b, "no-store"),
            Ok(Reply::Empty) => respond(req, 204, "text/plain", Vec::new(), "no-store"),
            Err(e) => text(req, 500, &e),
        };
    }

    if let Some(hash) = path.strip_prefix("/asset/") {
        let result = app.store.lock().unwrap_or_else(|e| e.into_inner()).asset(hash);
        return match result {
            Ok((bytes, mime)) => respond(req, 200, &mime, bytes, "max-age=31536000, immutable"),
            Err(_) => text(req, 404, "no such asset"),
        };
    }

    if path == "/favicon.ico" || path == "/icon.png" {
        return respond(req, 200, "image/png", ICON.to_vec(), "max-age=86400");
    }

    // The UI: static files, with index.html for everything else.
    let file = path.trim_start_matches('/');
    match UI.get_file(file).filter(|_| !file.is_empty()) {
        Some(f) => {
            let cache = if file.starts_with("assets/") { "max-age=31536000, immutable" } else { "no-cache" };
            respond(req, 200, content_type(file), f.contents().to_vec(), cache)
        }
        None => match UI.get_file("index.html") {
            Some(index) => respond(req, 200, "text/html; charset=utf-8", index.contents().to_vec(), "no-cache"),
            None => text(
                req,
                500,
                "This build has no user interface. Build it first: cd app && npm install && npx vite build, then rebuild anynote-server.",
            ),
        },
    }
}

fn content_type(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// Is an AnyNote server already answering on `port`?
fn already_running(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let req = format!(
        "POST /api/ping HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nX-AnyNote: 1\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
    );
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf.contains("\"anynote\"")
}

/// Open the UI as an app window in Chromium if there is one, else in the
/// default browser.
fn open_browser(url: &str, data_dir: &Path) {
    let profile = data_dir.join("browser-profile");
    for chromium in ["chromium-browser", "chromium", "google-chrome", "google-chrome-stable", "microsoft-edge"] {
        if find_in_path(chromium).is_some() {
            let spawned = Command::new(chromium)
                .arg(format!("--app={url}"))
                .arg(format!("--user-data-dir={}", profile.display()))
                .args([
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--password-store=basic",
                    "--class=AnyNote",
                    // Touch screens report touches (not mouse clicks), for swiping.
                    "--touch-events=enabled",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if spawned.is_ok() {
                return;
            }
        }
    }
    for opener in ["xdg-open", "open"] {
        if Command::new(opener).arg(url).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok() {
            return;
        }
    }
    println!("Open {url} in your web browser.");
}

fn find_in_path(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|d| Path::new(d).join(program)).find(|p| p.is_file())
}

/// Add AnyNote to the desktop's application menu (Linux), once.
fn install_menu_entry() {
    if !cfg!(target_os = "linux") {
        return;
    }
    let (Some(home), Ok(exe)) = (std::env::var_os("HOME"), std::env::current_exe()) else { return };
    let home = PathBuf::from(home);
    let apps = home.join(".local/share/applications");
    let entry = apps.join("anynote.desktop");
    let icon = home.join(".local/share/icons/hicolor/128x128/apps/anynote.png");
    let exe = exe.canonicalize().unwrap_or(exe);
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=AnyNote\nComment=Notebooks with text, ink, tasks and math\n\
         Exec=\"{}\"\nIcon={}\nTerminal=false\nCategories=Office;\nStartupWMClass=AnyNote\n",
        exe.display(),
        icon.display()
    );
    if std::fs::read_to_string(&entry).ok().as_deref() == Some(desktop.as_str()) {
        return;
    }
    let ok = std::fs::create_dir_all(&apps).is_ok()
        && icon.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok())
        && std::fs::write(&icon, ICON).is_ok()
        && std::fs::write(&entry, desktop).is_ok();
    if ok {
        println!("Added AnyNote to your applications menu (Office).");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events() -> Events {
        Events { log: Mutex::new((0, VecDeque::new())), changed: Condvar::new() }
    }

    #[test]
    fn long_poll_returns_new_events_only() {
        let ev = events();
        let first: Value = serde_json::from_str(&ev.wait(-1, Duration::ZERO)).unwrap();
        assert_eq!(first["next"], 0);
        ev.push(Broadcast { page_id: "p".into(), update: vec![1, 2] });
        let got: Value = serde_json::from_str(&ev.wait(0, Duration::ZERO)).unwrap();
        assert_eq!(got["next"], 1);
        assert_eq!(got["events"][0]["pageId"], "p");
        assert_eq!(got["events"][0]["update"], json!([1, 2]));
        let none: Value = serde_json::from_str(&ev.wait(1, Duration::from_millis(10))).unwrap();
        assert_eq!(none["events"], json!([]));
    }

    #[test]
    fn long_poll_wakes_up_on_push() {
        let ev = Arc::new(events());
        let waiter = {
            let ev = ev.clone();
            std::thread::spawn(move || ev.wait(0, Duration::from_secs(5)))
        };
        std::thread::sleep(Duration::from_millis(50));
        ev.push(Broadcast { page_id: "p".into(), update: vec![] });
        let got: Value = serde_json::from_str(&waiter.join().unwrap()).unwrap();
        assert_eq!(got["events"].as_array().unwrap().len(), 1);
    }
}
