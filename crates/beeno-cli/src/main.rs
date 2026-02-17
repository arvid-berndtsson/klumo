use anyhow::{Context, Result, anyhow};
use beeno_compiler::{CompileRequest, Compiler, CompilerRouter, FileCompileCache, SourceKind};
use beeno_config::{
    CliRunOverrides, EnvConfig, ProgressSetting, ProviderSetting, RunDefaults, load_file_config,
    resolve_run_defaults,
};
use beeno_core::{ProgressMode, RunOptions, compile_file, eval_inline, run_file};
use beeno_engine::{BoaEngine, JsEngine};
use beeno_engine_v8::V8Engine;
use beeno_llm::{
    LlmClient, LlmTranslateRequest, ProviderRouter, ProviderSelection, ReachabilityProbe,
};
use beeno_llm_ollama::OllamaClient;
use beeno_llm_openai::OpenAiCompatibleClient;
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

const REPL_HISTORY_LIMIT: usize = 20;
const REPL_SELF_HEAL_ATTEMPTS: usize = 2;
const DEFAULT_WEB_HOST: &str = "127.0.0.1";
const DEFAULT_WEB_PORT: u16 = 4173;

#[derive(Debug, Clone)]
struct WebServerConfig {
    host: String,
    port: u16,
    root_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct ApiRoute {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

type SharedApiRoutes = Arc<Mutex<HashMap<String, ApiRoute>>>;

#[derive(Debug)]
struct WebServerHandle {
    config: WebServerConfig,
    url: String,
    stop_tx: mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl WebServerHandle {
    fn stop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug)]
struct WebServerState {
    active: Option<WebServerHandle>,
    last_config: Option<WebServerConfig>,
    api_routes: SharedApiRoutes,
}

impl Default for WebServerState {
    fn default() -> Self {
        Self {
            active: None,
            last_config: None,
            api_routes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Drop for WebServerState {
    fn drop(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.stop();
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProviderArg {
    Auto,
    Ollama,
    Openai,
}

impl ProviderArg {
    fn as_setting(self) -> ProviderSetting {
        match self {
            ProviderArg::Auto => ProviderSetting::Auto,
            ProviderArg::Ollama => ProviderSetting::Ollama,
            ProviderArg::Openai => ProviderSetting::Openai,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "beeno", version, about = "Beeno runtime (M2 UX)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run a file in Beeno.
    Run {
        file: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        print_js: bool,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        force_llm: bool,
        #[arg(long)]
        self_heal: bool,
        #[arg(long, default_value_t = 1)]
        max_heal_attempts: usize,
        #[arg(long)]
        no_progress: bool,
        #[arg(long)]
        verbose: bool,
        #[arg(long, value_enum)]
        provider: Option<ProviderArg>,
        #[arg(long)]
        ollama_url: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Compile a source file into JavaScript.
    Bundle {
        file: PathBuf,
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        force_llm: bool,
        #[arg(long)]
        no_progress: bool,
        #[arg(long)]
        verbose: bool,
        #[arg(long, value_enum)]
        provider: Option<ProviderArg>,
        #[arg(long)]
        ollama_url: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Evaluate inline JavaScript.
    Eval { code: String },
    /// Start a JavaScript REPL.
    Repl {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        print_js: bool,
        #[arg(long)]
        no_cache: bool,
        #[arg(long)]
        no_progress: bool,
        #[arg(long)]
        verbose: bool,
        #[arg(long, value_enum)]
        provider: Option<ProviderArg>,
        #[arg(long)]
        ollama_url: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
}

struct OllamaProbe {
    client: OllamaClient,
}

type BeenoProviderRouter = ProviderRouter<OllamaClient, MaybeOpenAiClient, OllamaProbe>;
type BeenoCompiler = CompilerRouter<BeenoProviderRouter, FileCompileCache>;

impl ReachabilityProbe for OllamaProbe {
    fn ollama_reachable(&self) -> bool {
        self.client.is_reachable()
    }
}

struct MaybeOpenAiClient {
    inner: Option<OpenAiCompatibleClient>,
}

impl LlmClient for MaybeOpenAiClient {
    fn translate_to_js(&self, req: &LlmTranslateRequest, model: &str) -> Result<String> {
        let client = self.inner.as_ref().ok_or_else(|| {
            anyhow!("OPENAI_API_KEY is required for OpenAI-compatible translation")
        })?;
        client.translate_to_js(req, model)
    }
}

fn parse_kind_hint(lang: Option<&str>) -> Option<SourceKind> {
    lang.map(SourceKind::from_hint)
}

fn sanitize_repl_javascript(input: &str) -> String {
    let mut output = String::new();
    for line in input.lines() {
        let trimmed = line.trim_start();

        if let Some(rest) = trimmed.strip_prefix("export default ") {
            output.push_str(rest);
            output.push('\n');
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("export ") {
            output.push_str(rest);
            output.push('\n');
            continue;
        }

        if trimmed.starts_with("import ") {
            continue;
        }

        output.push_str(line);
        output.push('\n');
    }

    output.trim_end().to_string()
}

fn read_global_names(engine: &mut dyn JsEngine) -> Result<HashSet<String>> {
    let out = engine
        .eval_script(
            "JSON.stringify(Object.getOwnPropertyNames(globalThis))",
            "<repl-scope>",
        )
        .context("failed reading REPL global scope")?;
    let raw = out
        .value
        .ok_or_else(|| anyhow!("scope probe returned empty result"))?;
    let names: Vec<String> =
        serde_json::from_str(&raw).context("failed parsing REPL global scope JSON")?;
    Ok(names.into_iter().collect())
}

fn scope_context_text(bindings: &HashSet<String>) -> Option<String> {
    if bindings.is_empty() {
        return None;
    }
    let mut names: Vec<&str> = bindings.iter().map(String::as_str).collect();
    names.sort_unstable();
    Some(format!(
        "Bindings currently defined in this REPL session: {}. Avoid redeclaring them with const/let/class.",
        names.join(", ")
    ))
}

fn joined_recent_entries(history: &VecDeque<String>) -> Option<String> {
    if history.is_empty() {
        return None;
    }

    let joined = history
        .iter()
        .enumerate()
        .map(|(idx, item)| format!("{}. {}", idx + 1, item))
        .collect::<Vec<_>>()
        .join("\n");
    Some(joined)
}

fn build_repl_scope_context(
    bindings: &HashSet<String>,
    statement_history: &VecDeque<String>,
    js_history: &VecDeque<String>,
    web_server_context: Option<&str>,
) -> Option<String> {
    let mut sections = Vec::new();

    if let Some(bindings_text) = scope_context_text(bindings) {
        sections.push(bindings_text);
    }
    if let Some(statements) = joined_recent_entries(statement_history) {
        sections.push(format!(
            "Previously run REPL statements (oldest to newest):\n{}",
            statements
        ));
    }
    if let Some(js_snippets) = joined_recent_entries(js_history) {
        sections.push(format!(
            "Previously generated JavaScript snippets (oldest to newest):\n{}",
            js_snippets
        ));
    }
    if let Some(web_context) = web_server_context {
        sections.push(web_context.to_string());
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn push_bounded(history: &mut VecDeque<String>, item: String, cap: usize) {
    history.push_back(item);
    while history.len() > cap {
        history.pop_front();
    }
}

fn build_repl_self_heal_request(
    user_prompt: &str,
    generated_js: Option<&str>,
    error_text: &str,
    attempt: usize,
) -> String {
    let js_section = generated_js
        .map(|js| format!("Previously generated JavaScript that failed:\n{js}\n\n"))
        .unwrap_or_default();
    format!(
        "Repair the REPL translation so it executes successfully in this session.\n\
Return ONLY runnable JavaScript script statements. No markdown, no prose, no import/export.\n\
Preserve user intent as much as possible.\n\
Attempt: {}\n\
Original REPL prompt:\n{}\n\n\
{}\
Observed error:\n{}\n",
        attempt + 1,
        user_prompt,
        js_section,
        error_text
    )
}

fn compile_repl_heal_candidate(
    compiler: &BeenoCompiler,
    repl_lang: &str,
    provider_selection: ProviderSelection,
    model_override: Option<String>,
    no_cache: bool,
    scope_context: Option<String>,
    heal_prompt: String,
    attempt: usize,
) -> Result<String> {
    let healed = compiler.compile(&CompileRequest {
        source_text: heal_prompt,
        source_id: format!("<repl-self-heal-{attempt}>"),
        kind_hint: Some(SourceKind::Unknown(repl_lang.to_string())),
        language_hint: Some(repl_lang.to_string()),
        scope_context,
        force_llm: true,
        provider_selection,
        model_override,
        no_cache,
    })?;
    let sanitized_js = sanitize_repl_javascript(&healed.javascript);
    if sanitized_js.trim().is_empty() {
        return Err(anyhow!(
            "self-heal generated empty JavaScript after module-syntax cleanup"
        ));
    }
    Ok(sanitized_js)
}

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

fn web_server_scope_text(state: &WebServerState) -> String {
    let route_count = state
        .api_routes
        .lock()
        .map(|routes| routes.len())
        .unwrap_or_default();
    if let Some(active) = state.active.as_ref() {
        return format!(
            "Web daemon is running at {} and serving files from {}. Registered API routes: {}. File changes are reflected on refresh because content is read from disk per request. JavaScript APIs available in REPL: beeno.web.start(opts), beeno.web.stop(), beeno.web.restart(opts), beeno.web.status(), beeno.web.open(), beeno.web.routeJson(path, payload, opts), beeno.web.routeText(path, text, opts), beeno.web.unroute(path).",
            active.url,
            active.config.root_dir.display(),
            route_count
        );
    }

    format!(
        "Web daemon is not running. Registered API routes: {}. JavaScript APIs available in REPL: beeno.web.start(opts), beeno.web.stop(), beeno.web.restart(opts), beeno.web.status(), beeno.web.open(), beeno.web.routeJson(path, payload, opts), beeno.web.routeText(path, text, opts), beeno.web.unroute(path).",
        route_count
    )
}

fn install_repl_web_javascript_api(engine: &mut dyn JsEngine) -> Result<()> {
    engine.eval_script(
        r#"
globalThis.beeno = globalThis.beeno || {};
globalThis.__beeno_web_commands = Array.isArray(globalThis.__beeno_web_commands)
  ? globalThis.__beeno_web_commands
  : [];
globalThis.__beeno_web_status = globalThis.__beeno_web_status || { running: false };
const __beenoQueueWeb = (command) => {
  globalThis.__beeno_web_commands.push(command);
  return { queued: true, action: command.action };
};
globalThis.beeno.web = {
  start: (options = {}) => __beenoQueueWeb({ action: "start", options }),
  stop: () => __beenoQueueWeb({ action: "stop" }),
  restart: (options = {}) => __beenoQueueWeb({ action: "restart", options }),
  open: () => __beenoQueueWeb({ action: "open" }),
  status: () => globalThis.__beeno_web_status,
  routeJson: (path, payload, options = {}) =>
    __beenoQueueWeb({ action: "route_json", path, payload, options }),
  routeText: (path, text, options = {}) =>
    __beenoQueueWeb({ action: "route_text", path, text, options }),
  unroute: (path) => __beenoQueueWeb({ action: "unroute", path }),
};
"#,
        "<repl-web-api>",
    )?;
    Ok(())
}

fn drain_repl_web_commands(engine: &mut dyn JsEngine) -> Result<Vec<JsonValue>> {
    let out = engine.eval_script(
        r#"
(() => {
  const queue = Array.isArray(globalThis.__beeno_web_commands)
    ? globalThis.__beeno_web_commands
    : [];
  globalThis.__beeno_web_commands = [];
  return JSON.stringify(queue);
})()
"#,
        "<repl-web-drain>",
    )?;

    let raw = out.value.unwrap_or_else(|| "[]".to_string());
    serde_json::from_str::<Vec<JsonValue>>(&raw).context("failed parsing REPL web command queue")
}

fn write_repl_web_status(engine: &mut dyn JsEngine, state: &WebServerState) -> Result<()> {
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
        &format!("globalThis.__beeno_web_status = {payload_text};"),
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

fn route_path(raw: &str) -> Result<String> {
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

fn apply_repl_web_commands(commands: Vec<JsonValue>, state: &mut WebServerState) -> Result<()> {
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

fn print_web_usage() {
    println!("web daemon commands:");
    println!(
        "  .web start [--dir <path>] [--port <n>] [--host <ip>] [--open|--no-open|--no-open-prompt]"
    );
    println!("  .web stop");
    println!("  .web status");
    println!("  .web restart");
    println!("  .web open");
}

fn parse_web_start(tokens: &[&str]) -> Result<(WebServerConfig, Option<bool>, bool)> {
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

fn handle_web_command(input: &str, state: &mut WebServerState) -> Result<()> {
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

fn provider_to_selection(provider: ProviderSetting) -> ProviderSelection {
    match provider {
        ProviderSetting::Auto => ProviderSelection::Auto,
        ProviderSetting::Ollama => ProviderSelection::Ollama,
        ProviderSetting::Openai => ProviderSelection::OpenAiCompatible,
    }
}

fn resolved_progress_mode(progress: ProgressSetting, verbose: bool) -> ProgressMode {
    match progress {
        ProgressSetting::Silent => ProgressMode::Silent,
        ProgressSetting::Verbose => ProgressMode::Verbose,
        ProgressSetting::Auto => {
            if verbose {
                ProgressMode::Verbose
            } else {
                ProgressMode::Minimal
            }
        }
    }
}

fn build_run_options(resolved: &RunDefaults, model_override: Option<String>) -> RunOptions {
    RunOptions {
        kind_hint: parse_kind_hint(resolved.lang.as_deref()),
        language_hint: resolved.lang.clone(),
        force_llm: resolved.force_llm,
        no_cache: resolved.no_cache,
        print_js: resolved.print_js,
        provider_selection: provider_to_selection(resolved.provider),
        model_override,
        progress_mode: resolved_progress_mode(resolved.progress, resolved.verbose),
    }
}

fn resolve_config(config: Option<PathBuf>, cli_overrides: &CliRunOverrides) -> Result<RunDefaults> {
    let cwd = std::env::current_dir().context("failed getting current directory")?;
    let file_cfg = load_file_config(config.as_deref(), &cwd)?;
    let env_cfg = EnvConfig::from_current_env();
    Ok(resolve_run_defaults(
        cli_overrides,
        &env_cfg,
        file_cfg.as_ref(),
    ))
}

fn build_compiler(resolved: &RunDefaults) -> Result<BeenoCompiler> {
    let ollama_client = OllamaClient::new(resolved.ollama_url.clone())?;
    let openai_client = MaybeOpenAiClient {
        inner: resolved.openai_api_key.clone().map(|api_key| {
            OpenAiCompatibleClient::from_parts(resolved.openai_base_url.clone(), api_key)
        }),
    };

    let router = ProviderRouter {
        ollama: ollama_client.clone(),
        openai: openai_client,
        reachability: OllamaProbe {
            client: ollama_client,
        },
        ollama_model: resolved.ollama_model.clone(),
        openai_model: resolved.openai_model.clone(),
    };

    Ok(CompilerRouter {
        translator: router,
        cache: FileCompileCache::default(),
    })
}

fn build_engine() -> Result<Box<dyn JsEngine>> {
    let selected = std::env::var("BEENO_ENGINE").unwrap_or_else(|_| "boa".to_string());
    match selected.trim().to_ascii_lowercase().as_str() {
        "boa" => Ok(Box::new(BoaEngine::new())),
        "v8" => Ok(Box::new(V8Engine::new()?)),
        other => Err(anyhow!("unknown engine '{other}'. Supported: 'boa', 'v8'")),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_command(
    file: PathBuf,
    config: Option<PathBuf>,
    lang: Option<String>,
    print_js: bool,
    no_cache: bool,
    force_llm: bool,
    self_heal: bool,
    max_heal_attempts: usize,
    no_progress: bool,
    verbose: bool,
    provider: Option<ProviderArg>,
    ollama_url: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let cli_overrides = CliRunOverrides {
        provider: provider.map(ProviderArg::as_setting),
        ollama_url,
        model,
        lang,
        force_llm: force_llm.then_some(true),
        print_js: print_js.then_some(true),
        no_cache: no_cache.then_some(true),
        verbose: verbose.then_some(true),
        no_progress: no_progress.then_some(true),
    };

    let resolved = resolve_config(config, &cli_overrides)?;
    let compiler = build_compiler(&resolved)?;
    let options = build_run_options(&resolved, cli_overrides.model.clone());

    let mut engine = build_engine()?;
    let mut outcome = None;
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..=max_heal_attempts {
        match run_file(engine.as_mut(), &compiler, &file, &options) {
            Ok(ok) => {
                outcome = Some(ok);
                break;
            }
            Err(err) => {
                if !self_heal {
                    return Err(err).with_context(|| format!("failed running {}", file.display()));
                }
                if !is_self_heal_supported_source(&file) {
                    return Err(err).with_context(|| {
                        format!(
                            "failed running {} (self-heal currently supports .js/.mjs/.cjs/.jsx)",
                            file.display()
                        )
                    });
                }
                if attempt >= max_heal_attempts {
                    last_err = Some(err);
                    break;
                }

                let error_text = format!("{err:#}");
                if !matches!(options.progress_mode, ProgressMode::Silent) {
                    eprintln!(
                        "[beeno] runtime failed, attempting self-heal ({}/{})",
                        attempt + 1,
                        max_heal_attempts
                    );
                }

                if let Err(heal_err) =
                    try_self_heal(&compiler, &file, &options, &error_text, attempt)
                {
                    return Err(heal_err)
                        .with_context(|| format!("self-heal failed for {}", file.display()));
                }
            }
        }
    }

    let outcome = match outcome {
        Some(value) => value,
        None => {
            let err = last_err
                .map(|e| format!("{e:#}"))
                .unwrap_or_else(|| "unknown error".to_string());
            return Err(anyhow!(
                "failed running {} after {} self-heal attempts: {}",
                file.display(),
                max_heal_attempts,
                err
            ));
        }
    };

    if let Some(value) = outcome.eval.value {
        println!("{value}");
    }

    Ok(())
}

fn default_bundle_output(file: &std::path::Path) -> PathBuf {
    let mut out = file.to_path_buf();
    out.set_extension("bundle.js");
    out
}

fn is_self_heal_supported_source(file: &Path) -> bool {
    file.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "js" | "mjs" | "cjs" | "jsx"
            )
        })
        .unwrap_or(false)
}

fn backup_path_for(file: &Path) -> PathBuf {
    let mut backup = file.as_os_str().to_os_string();
    backup.push(".beeno.bak");
    PathBuf::from(backup)
}

fn build_self_heal_request(path: &Path, source: &str, error_text: &str) -> String {
    format!(
        "Repair this JavaScript file so it runs successfully.\n\
Return ONLY complete JavaScript source for the full file, no markdown, no prose.\n\
Preserve behavior and structure as much as possible.\n\
File: {}\n\
Runtime error:\n{}\n\
SOURCE START\n{}\n\
SOURCE END",
        path.display(),
        error_text,
        source
    )
}

fn try_self_heal(
    compiler: &BeenoCompiler,
    file: &Path,
    options: &RunOptions,
    error_text: &str,
    attempt: usize,
) -> Result<()> {
    let current_source = fs::read_to_string(file)
        .with_context(|| format!("failed reading source for self-heal {}", file.display()))?;

    let backup = backup_path_for(file);
    if !backup.exists() {
        fs::copy(file, &backup).with_context(|| {
            format!(
                "failed creating self-heal backup {} -> {}",
                file.display(),
                backup.display()
            )
        })?;
    }

    if !matches!(options.progress_mode, ProgressMode::Silent) {
        eprintln!(
            "[beeno] self-heal attempt {}: requesting file patch via LLM",
            attempt + 1
        );
    }

    let repaired = compiler.compile(&CompileRequest {
        source_text: build_self_heal_request(file, &current_source, error_text),
        source_id: format!("{}#self-heal-{}", file.display(), attempt + 1),
        kind_hint: Some(SourceKind::Unknown("self-heal".to_string())),
        language_hint: Some("self-heal-javascript".to_string()),
        scope_context: None,
        force_llm: true,
        provider_selection: options.provider_selection,
        model_override: options.model_override.clone(),
        no_cache: true,
    })?;

    if repaired.javascript.trim().is_empty() {
        return Err(anyhow!("self-heal generated empty output"));
    }

    fs::write(file, repaired.javascript)
        .with_context(|| format!("failed writing healed file {}", file.display()))?;

    if !matches!(options.progress_mode, ProgressMode::Silent) {
        eprintln!("[beeno] self-heal wrote patch to {}", file.display());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn bundle_command(
    file: PathBuf,
    output: Option<PathBuf>,
    config: Option<PathBuf>,
    lang: Option<String>,
    no_cache: bool,
    force_llm: bool,
    no_progress: bool,
    verbose: bool,
    provider: Option<ProviderArg>,
    ollama_url: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let cli_overrides = CliRunOverrides {
        provider: provider.map(ProviderArg::as_setting),
        ollama_url,
        model,
        lang,
        force_llm: force_llm.then_some(true),
        print_js: None,
        no_cache: no_cache.then_some(true),
        verbose: verbose.then_some(true),
        no_progress: no_progress.then_some(true),
    };

    let resolved = resolve_config(config, &cli_overrides)?;
    let compiler = build_compiler(&resolved)?;
    let options = build_run_options(&resolved, cli_overrides.model.clone());

    let compiled = compile_file(&compiler, &file, &options)
        .with_context(|| format!("failed bundling {}", file.display()))?;

    let target = output.unwrap_or_else(|| default_bundle_output(&file));
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed creating output dir {}", parent.display()))?;
        }
    }

    fs::write(&target, &compiled.javascript)
        .with_context(|| format!("failed writing bundle {}", target.display()))?;

    match options.progress_mode {
        ProgressMode::Silent => {}
        ProgressMode::Minimal => {
            if let Some(provider) = compiled.metadata.provider {
                let model = compiled.metadata.model.unwrap_or_default();
                eprintln!(
                    "[beeno] bundled via {}:{} (cache_hit={})",
                    format!("{provider:?}").to_ascii_lowercase(),
                    model,
                    compiled.metadata.cache_hit
                );
            }
            eprintln!("[beeno] wrote bundle {}", target.display());
        }
        ProgressMode::Verbose => {
            eprintln!(
                "[beeno] bundle compile complete provider={:?} model={:?} cache_hit={}",
                compiled.metadata.provider, compiled.metadata.model, compiled.metadata.cache_hit
            );
            eprintln!("[beeno] wrote bundle {}", target.display());
        }
    }

    println!("{}", target.display());
    Ok(())
}

fn eval_command(code: String) -> Result<()> {
    let mut engine = build_engine()?;
    let out = eval_inline(engine.as_mut(), &code)?;
    if let Some(value) = out.value {
        println!("{value}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn repl_command(
    config: Option<PathBuf>,
    lang: Option<String>,
    print_js: bool,
    no_cache: bool,
    no_progress: bool,
    verbose: bool,
    provider: Option<ProviderArg>,
    ollama_url: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let cli_overrides = CliRunOverrides {
        provider: provider.map(ProviderArg::as_setting),
        ollama_url,
        model,
        lang,
        force_llm: None,
        print_js: print_js.then_some(true),
        no_cache: no_cache.then_some(true),
        verbose: verbose.then_some(true),
        no_progress: no_progress.then_some(true),
    };
    let resolved = resolve_config(config, &cli_overrides)?;
    let compiler = build_compiler(&resolved)?;

    let mut engine = build_engine()?;
    install_repl_web_javascript_api(engine.as_mut())?;
    let baseline_globals = read_global_names(engine.as_mut())?;
    let mut known_bindings: HashSet<String> = HashSet::new();
    let mut statement_history: VecDeque<String> = VecDeque::new();
    let mut js_history: VecDeque<String> = VecDeque::new();
    let mut web_server = WebServerState::default();
    let mut line = String::new();
    let repl_lang = resolved
        .lang
        .clone()
        .unwrap_or_else(|| "pseudocode".to_string());
    let provider_selection = provider_to_selection(resolved.provider);

    println!("Beeno REPL (M2). Type .help for commands, .exit to quit.");
    write_repl_web_status(engine.as_mut(), &web_server)?;
    loop {
        line.clear();
        print!("beeno> ");
        io::stdout().flush().context("failed flushing stdout")?;

        let bytes = io::stdin()
            .read_line(&mut line)
            .context("failed reading REPL input")?;
        if bytes == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == ".help" {
            println!("REPL commands:");
            println!("  .help - show this help");
            println!("  .exit - quit");
            print_web_usage();
            println!("JavaScript web APIs:");
            println!("  beeno.web.start({{ dir, port, host, open, noOpenPrompt }})");
            println!("  beeno.web.stop()");
            println!("  beeno.web.restart({{ dir, port, host, open }})");
            println!("  beeno.web.open()");
            println!("  beeno.web.status()");
            println!("  beeno.web.routeJson(path, payload, {{ status }})");
            println!("  beeno.web.routeText(path, text, {{ status, contentType }})");
            println!("  beeno.web.unroute(path)");
            continue;
        }
        if trimmed.starts_with(".web") {
            if let Err(err) = handle_web_command(trimmed, &mut web_server) {
                eprintln!("error: {err:#}");
            }
            if let Err(err) = write_repl_web_status(engine.as_mut(), &web_server) {
                eprintln!("error: failed refreshing JS web status: {err:#}");
            }
            continue;
        }
        if trimmed == ".exit" {
            break;
        }

        let compiled = compiler.compile(&CompileRequest {
            source_text: trimmed.to_string(),
            source_id: "<repl>".to_string(),
            kind_hint: Some(SourceKind::Unknown(repl_lang.clone())),
            language_hint: Some(repl_lang.clone()),
            scope_context: build_repl_scope_context(
                &known_bindings,
                &statement_history,
                &js_history,
                Some(&web_server_scope_text(&web_server)),
            ),
            force_llm: true,
            provider_selection,
            model_override: cli_overrides.model.clone(),
            no_cache: resolved.no_cache,
        });

        let mut candidate_js = match compiled {
            Ok(compiled) => {
                let sanitized_js = sanitize_repl_javascript(&compiled.javascript);
                if sanitized_js.trim().is_empty() {
                    eprintln!("error: translated REPL code was empty after removing module syntax");
                    continue;
                }
                sanitized_js
            }
            Err(err) => {
                let mut healed: Option<String> = None;
                let initial_error = format!("{err:#}");
                for attempt in 0..REPL_SELF_HEAL_ATTEMPTS {
                    eprintln!(
                        "[beeno] repl translation failed, attempting self-heal ({}/{})",
                        attempt + 1,
                        REPL_SELF_HEAL_ATTEMPTS
                    );
                    let heal_prompt =
                        build_repl_self_heal_request(trimmed, None, &initial_error, attempt);
                    let heal_scope = build_repl_scope_context(
                        &known_bindings,
                        &statement_history,
                        &js_history,
                        Some(&web_server_scope_text(&web_server)),
                    );
                    match compile_repl_heal_candidate(
                        &compiler,
                        &repl_lang,
                        provider_selection,
                        cli_overrides.model.clone(),
                        resolved.no_cache,
                        heal_scope,
                        heal_prompt,
                        attempt,
                    ) {
                        Ok(js) => {
                            healed = Some(js);
                            break;
                        }
                        Err(heal_err) => {
                            eprintln!("error: self-heal compile failed: {heal_err:#}");
                        }
                    }
                }
                match healed {
                    Some(js) => js,
                    None => {
                        eprintln!("error: {initial_error}");
                        continue;
                    }
                }
            }
        };

        if resolved.verbose || resolved.print_js {
            println!("/* ===== generated JavaScript ===== */");
            println!("{}", candidate_js);
            println!("/* ===== end generated JavaScript ===== */");
        }

        let mut eval_output = None;
        let mut final_runtime_error: Option<String> = None;
        for attempt in 0..=REPL_SELF_HEAL_ATTEMPTS {
            match engine.as_mut().eval_script(&candidate_js, "<repl>") {
                Ok(output) => {
                    eval_output = Some(output);
                    break;
                }
                Err(err) => {
                    let err_text = format!("{err:#}");
                    if attempt >= REPL_SELF_HEAL_ATTEMPTS {
                        final_runtime_error = Some(err_text);
                        break;
                    }
                    eprintln!(
                        "[beeno] repl runtime failed, attempting self-heal ({}/{})",
                        attempt + 1,
                        REPL_SELF_HEAL_ATTEMPTS
                    );
                    let heal_prompt = build_repl_self_heal_request(
                        trimmed,
                        Some(&candidate_js),
                        &err_text,
                        attempt,
                    );
                    let heal_scope = build_repl_scope_context(
                        &known_bindings,
                        &statement_history,
                        &js_history,
                        Some(&web_server_scope_text(&web_server)),
                    );
                    match compile_repl_heal_candidate(
                        &compiler,
                        &repl_lang,
                        provider_selection,
                        cli_overrides.model.clone(),
                        resolved.no_cache,
                        heal_scope,
                        heal_prompt,
                        attempt,
                    ) {
                        Ok(healed_js) => {
                            candidate_js = healed_js;
                            if resolved.verbose || resolved.print_js {
                                println!("/* ===== healed JavaScript ===== */");
                                println!("{}", candidate_js);
                                println!("/* ===== end healed JavaScript ===== */");
                            }
                        }
                        Err(heal_err) => {
                            eprintln!("error: self-heal compile failed: {heal_err:#}");
                            final_runtime_error = Some(err_text);
                            break;
                        }
                    }
                }
            }
        }

        if let Some(output) = eval_output {
            push_bounded(
                &mut statement_history,
                trimmed.to_string(),
                REPL_HISTORY_LIMIT,
            );
            push_bounded(&mut js_history, candidate_js.clone(), REPL_HISTORY_LIMIT);

            if let Some(value) = output.value {
                println!("{value}");
            }
            if let Ok(current) = read_global_names(engine.as_mut()) {
                known_bindings = current
                    .difference(&baseline_globals)
                    .filter(|name| !name.starts_with("__beeno_"))
                    .cloned()
                    .collect();
            }
        } else if let Some(err) = final_runtime_error {
            eprintln!("error: {err}");
        }

        match drain_repl_web_commands(engine.as_mut()) {
            Ok(commands) => {
                if let Err(err) = apply_repl_web_commands(commands, &mut web_server) {
                    eprintln!("error: {err:#}");
                }
            }
            Err(err) => eprintln!("error: failed reading JS web command queue: {err:#}"),
        }
        if let Err(err) = write_repl_web_status(engine.as_mut(), &web_server) {
            eprintln!("error: failed refreshing JS web status: {err:#}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_WEB_HOST, DEFAULT_WEB_PORT, REPL_HISTORY_LIMIT, backup_path_for,
        build_repl_scope_context, build_self_heal_request, is_self_heal_supported_source,
        parse_web_start, push_bounded, route_path,
    };
    use std::collections::{HashSet, VecDeque};
    use std::path::Path;

    #[test]
    fn repl_scope_context_includes_bindings_and_history() {
        let mut bindings = HashSet::new();
        bindings.insert("hello".to_string());
        bindings.insert("count".to_string());

        let statements = VecDeque::from(vec![
            "store 2 in hello variable".to_string(),
            "print hello variable".to_string(),
        ]);
        let js = VecDeque::from(vec![
            "const hello = 2;".to_string(),
            "console.log(hello);".to_string(),
        ]);

        let context = build_repl_scope_context(&bindings, &statements, &js, None).expect("context");
        assert!(context.contains("Bindings currently defined"));
        assert!(context.contains("Previously run REPL statements"));
        assert!(context.contains("Previously generated JavaScript snippets"));
    }

    #[test]
    fn web_start_parser_applies_defaults() {
        let (config, open_override, ask_open) = parse_web_start(&[]).expect("parse");
        assert_eq!(config.host, DEFAULT_WEB_HOST);
        assert_eq!(config.port, DEFAULT_WEB_PORT);
        assert!(open_override.is_none());
        assert!(ask_open);
    }

    #[test]
    fn web_start_parser_supports_flags() {
        let (config, open_override, ask_open) = parse_web_start(&[
            "--host", "0.0.0.0", "--port", "8080", "--dir", "web", "--open",
        ])
        .expect("parse");
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert_eq!(config.root_dir.to_string_lossy(), "web");
        assert_eq!(open_override, Some(true));
        assert!(!ask_open);
    }

    #[test]
    fn route_path_normalizes_missing_leading_slash() {
        let normalized = route_path("api/health").expect("path");
        assert_eq!(normalized, "/api/health");
    }

    #[test]
    fn route_path_rejects_parent_segments() {
        let err = route_path("../escape").expect_err("should reject");
        assert!(err.to_string().contains("invalid route path"));
    }

    #[test]
    fn push_bounded_trims_old_entries() {
        let mut history = VecDeque::new();
        for i in 0..=(REPL_HISTORY_LIMIT + 2) {
            push_bounded(&mut history, format!("entry-{i}"), REPL_HISTORY_LIMIT);
        }
        assert_eq!(history.len(), REPL_HISTORY_LIMIT);
        assert_eq!(history.front().expect("front"), "entry-3");
    }

    #[test]
    fn self_heal_supported_extensions_are_limited() {
        assert!(is_self_heal_supported_source(Path::new("a.js")));
        assert!(is_self_heal_supported_source(Path::new("a.mjs")));
        assert!(is_self_heal_supported_source(Path::new("a.cjs")));
        assert!(is_self_heal_supported_source(Path::new("a.jsx")));
        assert!(!is_self_heal_supported_source(Path::new("a.ts")));
        assert!(!is_self_heal_supported_source(Path::new("a.pseudo")));
    }

    #[test]
    fn backup_path_is_derived_from_file_name() {
        let backup = backup_path_for(Path::new("/tmp/demo.js"));
        assert_eq!(backup.to_string_lossy(), "/tmp/demo.js.beeno.bak");
    }

    #[test]
    fn self_heal_prompt_contains_error_and_source() {
        let prompt =
            build_self_heal_request(Path::new("demo.js"), "console.log(1)", "ReferenceError");
        assert!(prompt.contains("demo.js"));
        assert!(prompt.contains("ReferenceError"));
        assert!(prompt.contains("console.log(1)"));
        assert!(prompt.contains("Return ONLY complete JavaScript source"));
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run {
            file,
            config,
            lang,
            print_js,
            no_cache,
            force_llm,
            self_heal,
            max_heal_attempts,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        }) => {
            if let Some(path) = file {
                run_command(
                    path,
                    config,
                    lang,
                    print_js,
                    no_cache,
                    force_llm,
                    self_heal,
                    max_heal_attempts,
                    no_progress,
                    verbose,
                    provider,
                    ollama_url,
                    model,
                )
            } else {
                repl_command(
                    config,
                    lang,
                    print_js,
                    no_cache,
                    no_progress,
                    verbose,
                    provider,
                    ollama_url,
                    model,
                )
            }
        }
        Some(Commands::Eval { code }) => eval_command(code),
        Some(Commands::Bundle {
            file,
            output,
            config,
            lang,
            no_cache,
            force_llm,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        }) => bundle_command(
            file,
            output,
            config,
            lang,
            no_cache,
            force_llm,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        ),
        Some(Commands::Repl {
            config,
            lang,
            print_js,
            no_cache,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        }) => repl_command(
            config,
            lang,
            print_js,
            no_cache,
            no_progress,
            verbose,
            provider,
            ollama_url,
            model,
        ),
        None => repl_command(None, None, false, false, false, false, None, None, None),
    }
}
