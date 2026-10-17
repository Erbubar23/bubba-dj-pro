use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use crossbeam_channel::unbounded;
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};
use tungstenite::handshake::server::{ErrorResponse, Request as WsRequest, Response as WsResponse};
use tungstenite::{Error as WsError, Message};

use crate::app::App;
use crate::commands::handle_ui_command;
use crate::config::log_line;
use crate::library;
use crate::loader::load_track;

const HTML_TEMPLATE: &str = include_str!("web/index.html");
const CSS_CONTENT: &str = include_str!("web/style.css");
const JS_CONTENT: &str = include_str!("web/app.js");

const MAX_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const STATE_INTERVAL: Duration = Duration::from_millis(16);

/// Binds the preferred port, falling back to any free port
pub fn bind_port(preferred: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", preferred)).or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
}

/// Returns true if another Bubba DJ Pro instance answers on this port
pub fn is_bubba_running(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(300)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = s.write_all(format!("GET /api/ping HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", port).as_bytes());
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf.contains("bubba-dj-pro")
}

// ======================= HTTP =======================

pub fn start_http(app: Arc<App>, listener: TcpListener, ws_port: u16) -> u16 {
    let http_port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let server = match Server::from_listener(listener, None) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            log_line(&format!("Error iniciando servidor HTTP: {}", e));
            return http_port;
        }
    };

    let html = HTML_TEMPLATE
        .replace("{{WS_PORT}}", &ws_port.to_string())
        .replace("{{TOKEN}}", &app.token);
    let html: Arc<str> = Arc::from(html);

    thread::Builder::new()
        .name("http".into())
        .spawn(move || {
            for request in server.incoming_requests() {
                let app = Arc::clone(&app);
                let html = Arc::clone(&html);
                thread::spawn(move || handle_http(app, request, http_port, &html));
            }
        })
        .ok();
    http_port
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn header_value<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn respond_json(req: Request, status: u16, body: &Value) {
    let resp = Response::from_string(body.to_string())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"));
    let _ = req.respond(resp);
}

fn handle_http(app: Arc<App>, req: Request, port: u16, html: &str) {
    // DNS-rebinding protection: only accept our own host names
    let host_ok = header_value(&req, "Host")
        .map(|h| h == format!("127.0.0.1:{}", port) || h == format!("localhost:{}", port))
        .unwrap_or(false);
    if !host_ok {
        let _ = req.respond(Response::from_string("Forbidden").with_status_code(403));
        return;
    }

    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let path = path.to_string();
    let query = query.to_string();

    if path == "/api/ping" {
        let _ = req.respond(Response::from_string("bubba-dj-pro"));
        return;
    }

    if path.starts_with("/api/") {
        if header_value(&req, "X-Bubba-Token") != Some(app.token.as_str()) {
            respond_json(req, 403, &json!({ "error": "token inválido" }));
            return;
        }
        handle_api(app, req, &path, &query);
        return;
    }

    let (body, content_type) = match path.as_str() {
        "/" | "/index.html" => (html, "text/html; charset=utf-8"),
        "/style.css" => (CSS_CONTENT, "text/css; charset=utf-8"),
        "/app.js" => (JS_CONTENT, "application/javascript; charset=utf-8"),
        _ => {
            let _ = req.respond(Response::from_string("Not found").with_status_code(404));
            return;
        }
    };
    let resp = Response::from_string(body)
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-cache"));
    let _ = req.respond(resp);
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| library::percent_decode(v))
}

fn handle_api(app: Arc<App>, mut req: Request, path: &str, query: &str) {
    match (req.method().clone(), path) {
        (Method::Get, "/api/roots") => {
            let last = app.config.lock().ok().and_then(|c| c.last_folder.clone());
            let roots = library::roots(app.exe_dir.as_deref());
            respond_json(req, 200, &json!({ "roots": roots, "last_folder": last }));
        }
        (Method::Post, "/api/browse") => {
            let mut body = String::new();
            let _ = req.as_reader().take(64 * 1024).read_to_string(&mut body);
            let target = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(String::from))
                .unwrap_or_default();
            let listing = {
                let cache = app.analysis_cache.lock().map(|c| c.clone()).unwrap_or_default();
                library::browse(&target, &cache)
            };
            match listing {
                Ok(v) => {
                    if let Ok(mut cfg) = app.config.lock() {
                        if cfg.last_folder.as_deref() != Some(target.as_str()) {
                            cfg.last_folder = Some(target);
                            cfg.save();
                        }
                    }
                    respond_json(req, 200, &v)
                }
                Err(e) => respond_json(req, 404, &json!({ "error": e })),
            }
        }
        (Method::Post, "/api/upload") => {
            let deck = query_param(query, "deck").and_then(|d| d.parse::<usize>().ok()).unwrap_or(0).min(1);
            let force = query_param(query, "force").as_deref() == Some("1");
            let name = header_value(&req, "X-File-Name").map(library::percent_decode).unwrap_or_default();
            let name = library::sanitize_file_name(&name);
            let dir = std::env::temp_dir().join("BubbaDJPro_uploads");
            let _ = std::fs::create_dir_all(&dir);
            let dest: PathBuf = dir.join(&name);

            let result = File::create(&dest).and_then(|mut f| {
                let copied = std::io::copy(&mut req.as_reader().take(MAX_UPLOAD_BYTES), &mut f)?;
                Ok(copied)
            });
            match result {
                Ok(bytes) if bytes > 0 => {
                    respond_json(req, 200, &json!({ "ok": true, "path": dest.to_string_lossy() }));
                    load_track(&app, deck, dest, force);
                }
                Ok(_) => respond_json(req, 400, &json!({ "error": "archivo vacío" })),
                Err(e) => respond_json(req, 500, &json!({ "error": e.to_string() })),
            }
        }
        _ => respond_json(req, 404, &json!({ "error": "no encontrado" })),
    }
}

// ======================= WebSocket =======================

pub fn start_ws(app: Arc<App>, listener: TcpListener) {
    thread::Builder::new()
        .name("ws-accept".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let app = Arc::clone(&app);
                thread::spawn(move || client_session(app, stream));
            }
        })
        .ok();
}

fn client_session(app: Arc<App>, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let expected = format!("t={}", app.token);
    let check = |req: &WsRequest, resp: WsResponse| -> Result<WsResponse, ErrorResponse> {
        let token_ok = req.uri().query().map(|q| q.split('&').any(|kv| kv == expected)).unwrap_or(false);
        if token_ok {
            Ok(resp)
        } else {
            let mut err = ErrorResponse::new(Some("Forbidden".into()));
            *err.status_mut() = tungstenite::http::StatusCode::FORBIDDEN;
            Err(err)
        }
    };

    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut ws = match tungstenite::accept_hdr(stream, check) {
        Ok(ws) => ws,
        Err(_) => return,
    };
    if ws.get_ref().set_nonblocking(true).is_err() {
        return;
    }

    let (tx, rx) = unbounded::<Arc<str>>();

    // Initial state for the new window
    let routing = app.routing_msg.read().map(|r| r.clone()).unwrap_or(Value::Null);
    let _ = tx.send(Arc::from(routing.to_string()));
    let _ = tx.send(Arc::from(app.midi_msg().to_string()));
    if let Ok(tracks) = app.tracks.read() {
        for t in tracks.iter().flatten() {
            let _ = tx.send(Arc::clone(t));
        }
    }
    let id = app.hub.add(tx);

    let mut last_state = Instant::now() - STATE_INTERVAL;
    'session: loop {
        // 1. Incoming commands
        loop {
            match ws.read() {
                Ok(Message::Text(txt)) => {
                    if let Ok(val) = serde_json::from_str::<Value>(txt.as_str()) {
                        handle_ui_command(&app, &val);
                    }
                }
                Ok(Message::Close(_)) => break 'session,
                Ok(_) => {}
                Err(WsError::Io(e)) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break 'session,
            }
        }

        // 2. Queued events (tracks, toasts, routing...)
        while let Ok(msg) = rx.try_recv() {
            if ws.write(Message::Text(msg.as_ref().into())).is_err() {
                break 'session;
            }
        }

        // 3. Real-time telemetry (~60 FPS)
        if last_state.elapsed() >= STATE_INTERVAL {
            last_state = Instant::now();
            if let Some(snap) = app.snapshot() {
                let msg = json!({ "type": "state", "state": snap }).to_string();
                if ws.write(Message::Text(msg.into())).is_err() {
                    break 'session;
                }
            }
        }

        match ws.flush() {
            Ok(()) => {}
            Err(WsError::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => break 'session,
        }
        thread::sleep(Duration::from_millis(4));
    }

    app.hub.remove(id);
}
