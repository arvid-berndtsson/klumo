use super::{
    ApiRoute, DEFAULT_WEB_HOST, DEFAULT_WEB_PORT, SharedApiRoutes, WebServerConfig,
    WebServerHandle, WebServerState,
};
use anyhow::{Context, Result, anyhow};
use klumo_engine::JsEngine;
use serde_json::Value as JsonValue;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

fn guess_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" | "cjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> Result<()> {
    let mut headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    if !head_only {
        headers.extend_from_slice(body);
    }
    stream.write_all(&headers)?;
    Ok(())
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

fn decode_percent_path(path: &str) -> Option<String> {
    let mut out = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            let value = u8::from_str_radix(hex, 16).ok()?;
            out.push(value);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn resolve_request_path(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let without_query = raw_path.split('?').next().unwrap_or("/");
    let decoded = decode_percent_path(without_query)?;
    let normalized = decoded.trim_start_matches('/');
    let mut candidate = root.to_path_buf();
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return None;
        }
        candidate.push(segment);
    }
    Some(candidate)
}

fn handle_web_connection(
    mut stream: TcpStream,
    root: &Path,
    api_routes: &SharedApiRoutes,
) -> Result<()> {
    let mut buffer = [0_u8; 16_384];
    let read = stream.read(&mut buffer)?;
    if read == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&buffer[..read]);
    let mut lines = request.lines();
    let first_line = match lines.next() {
        Some(line) => line,
        None => return Ok(()),
    };
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let raw_path = parts.next().unwrap_or("/");
    let head_only = method.eq_ignore_ascii_case("HEAD");

    if !method.eq_ignore_ascii_case("GET") && !head_only {
        return write_http_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            b"Method Not Allowed",
            head_only,
        );
    }

    let path_without_query = raw_path.split('?').next().unwrap_or("/");
    let normalized_request_path =
        decode_percent_path(path_without_query).unwrap_or_else(|| path_without_query.to_string());

    if let Ok(routes) = api_routes.lock() {
        if let Some(route) = routes.get(&normalized_request_path) {
            let status = format!("{} {}", route.status, status_text(route.status));
            return write_http_response(
                &mut stream,
                &status,
                &route.content_type,
                &route.body,
                head_only,
            );
        }
    }

    let mut target = match resolve_request_path(root, raw_path) {
        Some(path) => path,
        None => {
            return write_http_response(
                &mut stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                b"Bad Request",
                head_only,
            );
        }
    };

    if target.is_dir() {
        target.push("index.html");
    }

    if !target.exists()
        && !Path::new(raw_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .contains('.')
    {
        let spa = root.join("index.html");
        if spa.exists() {
            target = spa;
        }
    }

    let mut file = match File::open(&target) {
        Ok(file) => file,
        Err(_) => {
            return write_http_response(
                &mut stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"Not Found",
                head_only,
            );
        }
    };

    let mut body = Vec::new();
    file.read_to_end(&mut body)?;
    write_http_response(
        &mut stream,
        "200 OK",
        guess_content_type(&target),
        &body,
        head_only,
    )
}

fn start_web_server(
    config: &WebServerConfig,
    api_routes: SharedApiRoutes,
) -> Result<WebServerHandle> {
    let root_dir = config.root_dir.canonicalize().with_context(|| {
        format!(
            "failed resolving web root directory {}",
            config.root_dir.display()
        )
    })?;

    if !root_dir.is_dir() {
        return Err(anyhow!(
            "web root '{}' is not a directory",
            root_dir.display()
        ));
    }

    let listener = TcpListener::bind((config.host.as_str(), config.port)).with_context(|| {
        format!(
            "failed binding web server on {}:{}",
            config.host, config.port
        )
    })?;
    listener
        .set_nonblocking(true)
        .context("failed setting listener nonblocking mode")?;

    let actual_port = listener
        .local_addr()
        .context("failed reading listener local address")?
        .port();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let root_for_thread = root_dir.clone();
    let routes_for_thread = api_routes;

    let join_handle = thread::spawn(move || {
        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    if let Err(err) =
                        handle_web_connection(stream, &root_for_thread, &routes_for_thread)
                    {
                        eprintln!("error: web daemon request failed: {err:#}");
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(40));
                }
                Err(err) => {
                    eprintln!("error: web daemon listener failed: {err}");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });

    let runtime_cfg = WebServerConfig {
        host: config.host.clone(),
        port: actual_port,
        root_dir,
    };
    let url = format!("http://{}:{}/", runtime_cfg.host, runtime_cfg.port);

    Ok(WebServerHandle {
        config: runtime_cfg,
        url,
        stop_tx,
        join_handle: Some(join_handle),
    })
}

fn open_url_in_default_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let status = Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = Command::new("cmd").args(["/C", "start", "", url]).status();

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let status: io::Result<std::process::ExitStatus> = Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "unsupported platform for browser launch",
    ));

    let status = status.with_context(|| format!("failed launching browser for {url}"))?;
    if !status.success() {
        return Err(anyhow!("browser command exited with status {}", status));
    }
    Ok(())
}

fn prompt_yes_no(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush().context("failed flushing stdout")?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("failed reading confirmation input")?;
    let normalized = answer.trim().to_ascii_lowercase();
    Ok(matches!(normalized.as_str(), "y" | "yes"))
}

pub(crate) fn web_server_scope_text(state: &WebServerState) -> String {
    let route_count = state
        .api_routes
        .lock()
        .map(|routes| routes.len())
        .unwrap_or_default();
    if let Some(active) = state.active.as_ref() {
        return format!(
            "Web daemon is running at {} and serving files from {}. Registered API routes: {}. File changes are reflected on refresh because content is read from disk per request. JavaScript APIs available in REPL: klumo.web.start(opts), klumo.web.stop(), klumo.web.restart(opts), klumo.web.status(), klumo.web.open(), klumo.web.routeJson(path, payload, opts), klumo.web.routeText(path, text, opts), klumo.web.unroute(path).",
            active.url,
            active.config.root_dir.display(),
            route_count
        );
    }

    format!(
        "Web daemon is not running. Registered API routes: {}. JavaScript APIs available in REPL: klumo.web.start(opts), klumo.web.stop(), klumo.web.restart(opts), klumo.web.status(), klumo.web.open(), klumo.web.routeJson(path, payload, opts), klumo.web.routeText(path, text, opts), klumo.web.unroute(path).",
        route_count
    )
}

pub(crate) fn install_repl_web_javascript_api(engine: &mut dyn JsEngine) -> Result<()> {
    engine.eval_script(
        r#"
globalThis.klumo = globalThis.klumo || {};
globalThis.__klumo_web_commands = Array.isArray(globalThis.__klumo_web_commands)
  ? globalThis.__klumo_web_commands
  : [];
globalThis.__klumo_web_status = globalThis.__klumo_web_status || { running: false };
const __klumoQueueWeb = (command) => {
  globalThis.__klumo_web_commands.push(command);
  return { queued: true, action: command.action };
};
globalThis.klumo.web = {
  start: (options = {}) => __klumoQueueWeb({ action: "start", options }),
  stop: () => __klumoQueueWeb({ action: "stop" }),
  restart: (options = {}) => __klumoQueueWeb({ action: "restart", options }),
  open: () => __klumoQueueWeb({ action: "open" }),
  status: () => globalThis.__klumo_web_status,
  routeJson: (path, payload, options = {}) =>
    __klumoQueueWeb({ action: "route_json", path, payload, options }),
  routeText: (path, text, options = {}) =>
    __klumoQueueWeb({ action: "route_text", path, text, options }),
  unroute: (path) => __klumoQueueWeb({ action: "unroute", path }),
};
"#,
        "<repl-web-api>",
    )?;
    Ok(())
}

pub(crate) fn drain_repl_web_commands(engine: &mut dyn JsEngine) -> Result<Vec<JsonValue>> {
    let out = engine.eval_script(
        r#"
(() => {
  const queue = Array.isArray(globalThis.__klumo_web_commands)
    ? globalThis.__klumo_web_commands
    : [];
  globalThis.__klumo_web_commands = [];
  return JSON.stringify(queue);
})()
"#,
        "<repl-web-drain>",
    )?;

    let raw = out.value.unwrap_or_else(|| "[]".to_string());
    serde_json::from_str::<Vec<JsonValue>>(&raw).context("failed parsing REPL web command queue")
}

pub(crate) fn write_repl_web_status(engine: &mut dyn JsEngine, state: &WebServerState) -> Result<()> {
    let (running, url, host, port, root_dir) = if let Some(active) = state.active.as_ref() {
        (
            true,
            active.url.clone(),
            active.config.host.clone(),
            active.config.port,
            active.config.root_dir.display().to_string(),
        )
    } else {
        (
            false,
            String::new(),
            String::new(),
            0,
            state
                .last_config
                .as_ref()
                .map(|cfg| cfg.root_dir.display().to_string())
                .unwrap_or_default(),
        )
    };

    let routes: Vec<String> = state
        .api_routes
        .lock()
        .map(|routes| routes.keys().cloned().collect())
        .unwrap_or_default();

    let payload = serde_json::json!({
        "running": running,
        "url": url,
        "host": host,
        "port": port,
        "rootDir": root_dir,
        "routes": routes
    });
    let payload_text =
        serde_json::to_string(&payload).context("failed serializing REPL web status JSON")?;
    engine.eval_script(
        &format!("globalThis.__klumo_web_status = {payload_text};"),
        "<repl-web-status>",
    )?;
    Ok(())
}

fn bool_from_value(value: Option<&JsonValue>) -> Option<bool> {
    value.and_then(JsonValue::as_bool)
}

fn string_from_value(value: Option<&JsonValue>) -> Option<String> {
    value.and_then(JsonValue::as_str).map(str::to_string)
}

pub(crate) fn route_path(raw: &str) -> Result<String> {
    let normalized = if raw.starts_with('/') {
        raw.to_string()
    } else {
        format!("/{raw}")
    };
    if normalized.contains("..") {
        return Err(anyhow!("invalid route path '{}'", raw));
    }
    Ok(normalized)
}

fn register_json_route(
    path: &str,
    payload: &JsonValue,
    status: u16,
    state: &mut WebServerState,
) -> Result<()> {
    let key = route_path(path)?;
    let body = serde_json::to_vec(payload).context("failed encoding JSON route payload")?;
    let mut routes = state
        .api_routes
        .lock()
        .map_err(|_| anyhow!("failed locking API route table"))?;
    routes.insert(
        key.clone(),
        ApiRoute {
            status,
            content_type: "application/json; charset=utf-8".to_string(),
            body,
        },
    );
    println!("registered API route {key}");
    Ok(())
}

fn register_text_route(
    path: &str,
    text: &str,
    status: u16,
    content_type: Option<String>,
    state: &mut WebServerState,
) -> Result<()> {
    let key = route_path(path)?;
    let mut routes = state
        .api_routes
        .lock()
        .map_err(|_| anyhow!("failed locking API route table"))?;
    routes.insert(
        key.clone(),
        ApiRoute {
            status,
            content_type: content_type.unwrap_or_else(|| "text/plain; charset=utf-8".to_string()),
            body: text.as_bytes().to_vec(),
        },
    );
    println!("registered API route {key}");
    Ok(())
}

fn run_web_start(
    state: &mut WebServerState,
    config: WebServerConfig,
    open_override: Option<bool>,
    ask_open: bool,
) -> Result<()> {
    if state.active.is_some() {
        println!("web daemon is already running. Use .web restart or .web stop.");
        return Ok(());
    }
    let handle = start_web_server(&config, Arc::clone(&state.api_routes))?;
    println!(
        "web daemon started at {} (dir={})",
        handle.url,
        handle.config.root_dir.display()
    );
    state.last_config = Some(handle.config.clone());
    let url = handle.url.clone();
    state.active = Some(handle);

    let should_open = match open_override {
        Some(value) => value,
        None if ask_open => prompt_yes_no("Open this page in your default browser now?")?,
        None => false,
    };
    if should_open {
        match open_url_in_default_browser(&url) {
            Ok(()) => println!("opened {}", url),
            Err(err) => eprintln!("error: failed opening browser: {err:#}"),
        }
    }
    Ok(())
}

fn run_web_stop(state: &mut WebServerState) {
    let mut active = match state.active.take() {
        Some(active) => active,
        None => {
            println!("web daemon is not running.");
            return;
        }
    };
    let url = active.url.clone();
    active.stop();
    println!("web daemon stopped ({url})");
}

fn run_web_restart(
    state: &mut WebServerState,
    override_config: Option<WebServerConfig>,
    open_after_restart: bool,
) -> Result<()> {
    let restart_cfg = override_config.unwrap_or_else(|| {
        if let Some(active) = state.active.as_ref() {
            active.config.clone()
        } else if let Some(last) = state.last_config.as_ref() {
            last.clone()
        } else {
            WebServerConfig {
                host: DEFAULT_WEB_HOST.to_string(),
                port: DEFAULT_WEB_PORT,
                root_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            }
        }
    });

    if let Some(mut active) = state.active.take() {
        active.stop();
    }

    let handle = start_web_server(&restart_cfg, Arc::clone(&state.api_routes))?;
    println!(
        "web daemon restarted at {} (dir={})",
        handle.url,
        handle.config.root_dir.display()
    );
    state.last_config = Some(handle.config.clone());
    let url = handle.url.clone();
    state.active = Some(handle);
    if open_after_restart {
        open_url_in_default_browser(&url)?;
        println!("opened {url}");
    }
    Ok(())
}

pub(crate) fn apply_repl_web_commands(
    commands: Vec<JsonValue>,
    state: &mut WebServerState,
) -> Result<()> {
    for command in commands {
        let Some(action) = command.get("action").and_then(JsonValue::as_str) else {
            continue;
        };
        match action {
            "start" => {
                let options = command.get("options").and_then(JsonValue::as_object);
                let host = string_from_value(options.and_then(|o| o.get("host")))
                    .unwrap_or_else(|| DEFAULT_WEB_HOST.to_string());
                let port = options
                    .and_then(|o| o.get("port"))
                    .and_then(JsonValue::as_u64)
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(DEFAULT_WEB_PORT);
                let root_dir = string_from_value(options.and_then(|o| o.get("dir")))
                    .map(PathBuf::from)
                    .unwrap_or(
                        std::env::current_dir().context("failed getting current directory")?,
                    );
                let open_override = bool_from_value(options.and_then(|o| o.get("open")));
                let ask_open = !bool_from_value(options.and_then(|o| o.get("noOpenPrompt")))
                    .unwrap_or(false)
                    && open_override.is_none();
                run_web_start(
                    state,
                    WebServerConfig {
                        host,
                        port,
                        root_dir,
                    },
                    open_override,
                    ask_open,
                )?;
            }
            "stop" => run_web_stop(state),
            "restart" => {
                let options = command.get("options").and_then(JsonValue::as_object);
                let override_config = options.map(|o| WebServerConfig {
                    host: string_from_value(o.get("host"))
                        .unwrap_or_else(|| DEFAULT_WEB_HOST.to_string()),
                    port: o
                        .get("port")
                        .and_then(JsonValue::as_u64)
                        .and_then(|p| u16::try_from(p).ok())
                        .unwrap_or(DEFAULT_WEB_PORT),
                    root_dir: string_from_value(o.get("dir"))
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from(".")),
                });
                let open_after_restart =
                    bool_from_value(options.and_then(|o| o.get("open"))).unwrap_or(false);
                run_web_restart(state, override_config, open_after_restart)?;
            }
            "open" => {
                let url = state
                    .active
                    .as_ref()
                    .map(|active| active.url.clone())
                    .ok_or_else(|| anyhow!("web daemon is not running"))?;
                open_url_in_default_browser(&url)?;
                println!("opened {url}");
            }
            "route_json" => {
                let Some(path) = command.get("path").and_then(JsonValue::as_str) else {
                    continue;
                };
                let payload = command.get("payload").cloned().unwrap_or(JsonValue::Null);
                let status = command
                    .get("options")
                    .and_then(JsonValue::as_object)
                    .and_then(|o| o.get("status"))
                    .and_then(JsonValue::as_u64)
                    .and_then(|v| u16::try_from(v).ok())
                    .unwrap_or(200);
                register_json_route(path, &payload, status, state)?;
            }
            "route_text" => {
                let Some(path) = command.get("path").and_then(JsonValue::as_str) else {
                    continue;
                };
                let text = command
                    .get("text")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default();
                let options = command.get("options").and_then(JsonValue::as_object);
                let status = options
                    .and_then(|o| o.get("status"))
                    .and_then(JsonValue::as_u64)
                    .and_then(|v| u16::try_from(v).ok())
                    .unwrap_or(200);
                let content_type = string_from_value(options.and_then(|o| o.get("contentType")));
                register_text_route(path, text, status, content_type, state)?;
            }
            "unroute" => {
                let Some(path) = command.get("path").and_then(JsonValue::as_str) else {
                    continue;
                };
                let key = route_path(path)?;
                if let Ok(mut routes) = state.api_routes.lock() {
                    routes.remove(&key);
                }
                println!("removed API route {key}");
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn print_web_usage() {
    println!("web daemon commands:");
    println!(
        "  .web start [--dir <path>] [--port <n>] [--host <ip>] [--open|--no-open|--no-open-prompt]"
    );
    println!("  .web stop");
    println!("  .web status");
    println!("  .web restart");
    println!("  .web open");
}

pub(crate) fn parse_web_start(tokens: &[&str]) -> Result<(WebServerConfig, Option<bool>, bool)> {
    let mut host = DEFAULT_WEB_HOST.to_string();
    let mut port = DEFAULT_WEB_PORT;
    let mut root_dir = std::env::current_dir().context("failed getting current directory")?;
    let mut open_override: Option<bool> = None;
    let mut ask_open = true;

    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "--dir" => {
                let value = tokens
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("missing value for --dir"))?;
                root_dir = PathBuf::from(value);
                i += 2;
            }
            "--port" => {
                let value = tokens
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("missing value for --port"))?;
                port = value
                    .parse::<u16>()
                    .with_context(|| format!("invalid --port value '{value}'"))?;
                i += 2;
            }
            "--host" => {
                let value = tokens
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("missing value for --host"))?;
                host = (*value).to_string();
                i += 2;
            }
            "--open" => {
                open_override = Some(true);
                ask_open = false;
                i += 1;
            }
            "--no-open" => {
                open_override = Some(false);
                ask_open = false;
                i += 1;
            }
            "--no-open-prompt" => {
                ask_open = false;
                i += 1;
            }
            unknown => return Err(anyhow!("unknown .web start flag '{unknown}'")),
        }
    }

    Ok((
        WebServerConfig {
            host,
            port,
            root_dir,
        },
        open_override,
        ask_open,
    ))
}

pub(crate) fn handle_web_command(input: &str, state: &mut WebServerState) -> Result<()> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.is_empty() || parts[0] != ".web" {
        return Ok(());
    }

    let action = parts.get(1).copied().unwrap_or("status");
    match action {
        "help" => print_web_usage(),
        "status" => {
            let route_count = state
                .api_routes
                .lock()
                .map(|routes| routes.len())
                .unwrap_or_default();
            if let Some(active) = state.active.as_ref() {
                println!(
                    "web daemon: running at {} (dir={}, routes={})",
                    active.url,
                    active.config.root_dir.display(),
                    route_count
                );
            } else if let Some(last) = state.last_config.as_ref() {
                println!(
                    "web daemon: stopped (last config host={} port={} dir={}, routes={})",
                    last.host,
                    last.port,
                    last.root_dir.display(),
                    route_count
                );
            } else {
                println!("web daemon: stopped (routes={route_count})");
            }
        }
        "start" => {
            let (config, open_override, ask_open) = parse_web_start(&parts[2..])?;
            run_web_start(state, config, open_override, ask_open)?;
        }
        "stop" => run_web_stop(state),
        "restart" => run_web_restart(state, None, false)?,
        "open" => {
            let url = state
                .active
                .as_ref()
                .map(|active| active.url.clone())
                .ok_or_else(|| anyhow!("web daemon is not running"))?;
            open_url_in_default_browser(&url)?;
            println!("opened {url}");
        }
        _ => {
            print_web_usage();
            return Err(anyhow!("unknown .web action '{action}'"));
        }
    }
    Ok(())
}
