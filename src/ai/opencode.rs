//! OpenCode's ACP (Agent Client Protocol) stdio runtime.
//!
//! A runtime owns one `opencode acp` process and one ACP session. Create one
//! runtime per Fastty tab so that the session cwd stays explicit.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct OpencodeModel {
    pub provider_id: String,
    pub model_id: String,
    pub variant: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: Option<String>,
}

#[derive(Debug, Clone)]
pub enum OpencodeEvent {
    Text(String),
    Thinking(String),
    ToolUpdate(Value),
    PermissionRequest {
        request_id: Value,
        tool_call: Value,
        options: Vec<PermissionOption>,
    },
    UsageUpdate { used: u64, size: u64 },
    AvailableCommands(Vec<crate::ai::conversations::AvailableCommand>),
    OtherUpdate(Value),
    PromptFinished(Value),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct OpencodeInfo {
    pub protocol_version: Option<u64>,
    pub agent_info: Option<Value>,
    pub auth_methods: Vec<Value>,
    pub server_capabilities: Value,
    /// Present when the ACP server returns model choices in `session/new`.
    pub available_models: Value,
    pub config_options: Value,
    pub mode: Value,
}

pub fn discover_catalog(
    command: impl AsRef<std::ffi::OsStr>,
    cwd: impl AsRef<Path>,
) -> anyhow::Result<(Vec<String>, HashMap<String, Vec<String>>)> {
    discover_catalog_with_args(command, &["acp".to_string()], cwd, true)
}

pub fn discover_catalog_with_args(
    command: impl AsRef<std::ffi::OsStr>,
    args: &[String],
    cwd: impl AsRef<Path>,
    opencode_config: bool,
) -> anyhow::Result<(Vec<String>, HashMap<String, Vec<String>>)> {
    let discovery_deadline = std::time::Instant::now() + Duration::from_secs(30);
    let runtime = OpencodeRuntime::spawn_with_args_deadline(command, args, cwd, discovery_deadline)?;
    let mut models = Vec::new();
    let mut variants = HashMap::<String, Vec<String>>::new();
    let options = runtime
        .info()
        .config_options
        .as_array()
        .cloned()
        .unwrap_or_default();
    let model_option = options
        .iter()
        .find(|option| option.get("id").and_then(Value::as_str) == Some("model"));
    if let Some(model_option) = model_option {
        for option in model_option
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(value) = option.get("value").and_then(Value::as_str) else {
                continue;
            };
            let base = if opencode_config {
                let Some((provider, model)) = value.split_once('/') else { continue };
                if provider.is_empty() || model.is_empty() { continue; }
                format!("{provider}/{model}")
            } else {
                value.to_string()
            };
            if !models.contains(&base) {
                models.push(base.clone());
            }
        }
    }
    if let Some(entries) = runtime.info().available_models.as_array()
        .or_else(|| runtime.info().available_models.get("availableModels").and_then(Value::as_array)) {
        for entry in entries {
            let value = entry.get("value").or_else(|| entry.get("modelId")).or_else(|| entry.get("id"))
                .and_then(Value::as_str);
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                if !models.contains(&value.to_string()) { models.push(value.to_string()); }
            }
        }
    }
    if !opencode_config { return Ok((models, variants)); }
    for model in models.clone() {
        let remaining = discovery_deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(anyhow::anyhow!("ACP model catalog discovery timed out"));
        }
        let response = match runtime.select_model_value_with_timeout(&model, remaining) {
            Ok(response) => response,
            Err(error) if std::time::Instant::now() >= discovery_deadline => return Err(error),
            Err(_) => continue,
        };
        let model_options = response
            .get("configOptions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(efforts) = model_options
            .iter()
            .find(|option| option.get("id").and_then(Value::as_str) == Some("effort"))
            .and_then(|option| option.get("options"))
            .and_then(Value::as_array)
        {
            let values = efforts
                .iter()
                .filter_map(|option| {
                    option
                        .get("value")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>();
            if !values.is_empty() {
                variants.insert(model, values);
            }
        }
    }
    Ok((models, variants))
}

pub fn catalog_model_key(
    model_id: &str,
    variants: &HashMap<String, Vec<String>>,
) -> Option<String> {
    if variants.contains_key(model_id) {
        return Some(model_id.to_string());
    }
    variants
        .iter()
        .filter(|(base, choices)| {
            model_id
                .strip_prefix(&format!("{base}/"))
                .is_some_and(|effort| choices.iter().any(|choice| choice == effort))
        })
        .max_by_key(|(base, _)| base.len())
        .map(|(base, _)| base.clone())
}

struct Transport {
    stdin: Mutex<ChildStdin>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>,
    events: mpsc::Sender<OpencodeEvent>,
    next_id: AtomicU64,
}

pub struct OpencodeRuntime {
    command: std::ffi::OsString,
    args: Vec<String>,
    cwd: PathBuf,
    session_id: String,
    info: OpencodeInfo,
    transport: Arc<Transport>,
    events: Mutex<mpsc::Receiver<OpencodeEvent>>,
    child: Mutex<Option<Child>>,
}

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl OpencodeRuntime {
    /// Start and initialize `opencode acp`, then create an ACP session at cwd.
    pub fn spawn(cwd: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::spawn_with_command("opencode", cwd)
    }

    pub fn spawn_with_command(
        command: impl AsRef<std::ffi::OsStr>,
        cwd: impl AsRef<Path>,
    ) -> anyhow::Result<Self> {
        Self::start(command.as_ref(), &["acp".to_string()], cwd.as_ref(), None, None)
    }

    pub fn spawn_with_args(
        command: impl AsRef<std::ffi::OsStr>,
        args: &[String],
        cwd: impl AsRef<Path>,
    ) -> anyhow::Result<Self> {
        Self::start(command.as_ref(), args, cwd.as_ref(), None, None)
    }

    fn spawn_with_args_deadline(
        command: impl AsRef<std::ffi::OsStr>,
        args: &[String],
        cwd: impl AsRef<Path>,
        deadline: std::time::Instant,
    ) -> anyhow::Result<Self> {
        Self::start(command.as_ref(), args, cwd.as_ref(), None, Some(deadline))
    }

    /// Start and initialize `opencode acp`, then load a saved ACP session.
    pub fn load(cwd: impl AsRef<Path>, session_id: impl Into<String>) -> anyhow::Result<Self> {
        Self::load_with_command("opencode", cwd, session_id)
    }

    pub fn load_with_command(
        command: impl AsRef<std::ffi::OsStr>,
        cwd: impl AsRef<Path>,
        session_id: impl Into<String>,
    ) -> anyhow::Result<Self> {
        Self::start(command.as_ref(), &["acp".to_string()], cwd.as_ref(), Some(session_id.into()), None)
    }

    pub fn load_with_args(
        command: impl AsRef<std::ffi::OsStr>,
        args: &[String],
        cwd: impl AsRef<Path>,
        session_id: impl Into<String>,
    ) -> anyhow::Result<Self> {
        Self::start(command.as_ref(), args, cwd.as_ref(), Some(session_id.into()), None)
    }

    fn start(
        command: &std::ffi::OsStr,
        args: &[String],
        cwd: &Path,
        saved_session_id: Option<String>,
        deadline: Option<std::time::Instant>,
    ) -> anyhow::Result<Self> {
        let cwd = absolute_directory(cwd.as_ref())?;
        let preparation_started = std::time::Instant::now();
        let mut launch = super::acp_launcher::prepare(command, args, &cwd)?;
        let deadline = deadline.map(|value| value + preparation_started.elapsed());
        let child = launch
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| anyhow::anyhow!("Cannot start ACP command '{}': {error}", command.to_string_lossy()))?;
        let mut child = ChildGuard(Some(child));

        let process = child.0.as_mut().expect("new child guard has a process");
        let stdin = process
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("ACP stdin unavailable"))?;
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("ACP stdout unavailable"))?;
        if let Some(stderr) = process.stderr.take() {
            thread::Builder::new()
                .name("opencode-acp-stderr".into())
                .spawn(move || {
                    let mut reader = BufReader::new(stderr);
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 0 {
                        line.clear();
                    }
                })?;
        }

        let (event_tx, event_rx) = mpsc::channel();
        let transport = Arc::new(Transport {
            stdin: Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            events: event_tx,
            next_id: AtomicU64::new(1),
        });
        spawn_reader(stdout, Arc::clone(&transport))?;

        let init_params = json!({
                "protocolVersion": 1,
                "clientInfo": { "name": "fastty", "version": env!("CARGO_PKG_VERSION") },
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false
                }
            });
        let init = match deadline {
            Some(deadline) => request_before(&transport, "initialize", init_params, deadline)?,
            None => request(&transport, "initialize", init_params)?,
        };
        let info = OpencodeInfo {
            protocol_version: init.get("protocolVersion").and_then(Value::as_u64),
            agent_info: init.get("agentInfo").cloned(),
            auth_methods: init
                .get("authMethods")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            server_capabilities: init
                .get("agentCapabilities")
                .cloned()
                .unwrap_or(Value::Null),
            available_models: Value::Null,
            config_options: Value::Null,
            mode: Value::Null,
        };
        write_message(
            &transport,
            json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
        )?;

        let saved_session_id = saved_session_id.filter(|_| {
            info.server_capabilities
                .get("loadSession")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
        let (method, params, requested_session_id) = match saved_session_id {
            Some(session_id) => (
                "session/load",
                json!({ "cwd": cwd, "sessionId": session_id.clone(), "mcpServers": [] }),
                Some(session_id),
            ),
            None => ("session/new", json!({ "cwd": cwd, "mcpServers": [] }), None),
        };
        let session = match deadline {
            Some(deadline) => request_before(&transport, method, params, deadline)?,
            None => request(&transport, method, params)?,
        };
        let session_id = requested_session_id
            .or_else(|| session.get("sessionId").and_then(Value::as_str).map(str::to_string))
            .ok_or_else(|| anyhow::anyhow!("ACP {method} response has no sessionId"))?;

        let info = OpencodeInfo {
            available_models: session.get("models").cloned().unwrap_or(Value::Null),
            config_options: session.get("configOptions").cloned().unwrap_or(Value::Null),
            mode: session.get("mode").cloned().unwrap_or(Value::Null),
            ..info
        };
        let child = child.0.take().expect("child guard has a process");
        Ok(Self {
            command: command.to_os_string(),
            args: args.to_vec(),
            cwd,
            session_id,
            info,
            transport,
            events: Mutex::new(event_rx),
            child: Mutex::new(Some(child)),
        })
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
    pub fn command(&self) -> &std::ffi::OsStr {
        &self.command
    }
    pub fn is_alive(&self) -> bool {
        self.child
            .lock()
            .map(|mut child| {
                child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok())
                    .map(|status| status.is_none())
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn info(&self) -> &OpencodeInfo {
        &self.info
    }

    /// Select a provider/model/variant using ACP's model config option.
    pub fn select_model(&self, model: &OpencodeModel) -> anyhow::Result<()> {
        let mut value = format!("{}/{}", model.provider_id, model.model_id);
        if let Some(variant) = model
            .variant
            .as_deref()
            .filter(|variant| !variant.is_empty())
        {
            value.push('/');
            value.push_str(variant);
        }
        self.select_model_value(&value).map(|_| ())
    }

    pub fn select_model_value(&self, value: &str) -> anyhow::Result<Value> {
        self.select_model_value_with_timeout(value, REQUEST_TIMEOUT)
    }

    pub fn select_model_id(&self, model_id: &str) -> anyhow::Result<()> {
        request(
            &self.transport,
            "session/set_model",
            json!({ "sessionId": self.session_id, "modelId": model_id }),
        )?;
        Ok(())
    }

    pub fn select_config_option(&self, config_id: &str, value: &str) -> anyhow::Result<()> {
        request(
            &self.transport,
            "session/set_config_option",
            json!({ "sessionId": self.session_id, "configId": config_id, "value": value }),
        )?;
        Ok(())
    }

    pub fn select_model_value_with_timeout(
        &self,
        value: &str,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        request_inner(
            &self.transport,
            "session/set_config_option",
            json!({ "sessionId": self.session_id, "configId": "model", "value": value }),
            Some(timeout),
        )
    }

    pub fn select_effort(&self, value: &str) -> anyhow::Result<()> {
        request(
            &self.transport,
            "session/set_config_option",
            json!({ "sessionId": self.session_id, "configId": "effort", "value": value }),
        )?;
        Ok(())
    }

    /// Select the OpenCode agent/mode exposed by the server.
    pub fn select_mode(&self, mode_id: &str) -> anyhow::Result<()> {
        request(
            &self.transport,
            "session/set_mode",
            json!({ "sessionId": self.session_id, "modeId": mode_id }),
        )?;
        Ok(())
    }

    /// Start a prompt. Read events with `recv_event`; ACP sends session updates
    /// while this call runs and a `PromptFinished` event when it returns.
    pub fn prompt(&self, text: impl Into<String>) -> anyhow::Result<()> {
        self.prompt_content(vec![json!({ "type": "text", "text": text.into() })])
    }

    pub fn prompt_content(&self, content: Vec<Value>) -> anyhow::Result<()> {
        let transport = Arc::clone(&self.transport);
        let session_id = self.session_id.clone();
        thread::Builder::new()
            .name("opencode-acp-prompt".into())
            .spawn(move || {
                match request_without_timeout(
                    &transport,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": content
                    }),
                ) {
                    Ok(response) => {
                        let _ = transport
                            .events
                            .send(OpencodeEvent::PromptFinished(response));
                    }
                    Err(error) => {
                        let _ = transport
                            .events
                            .send(OpencodeEvent::Error(error.to_string()));
                    }
                }
            })?;
        Ok(())
    }

    pub fn cancel(&self) -> anyhow::Result<()> {
        request(
            &self.transport,
            "session/cancel",
            json!({ "sessionId": self.session_id }),
        )?;
        Ok(())
    }

    /// Reply with an offered ACP option ID. The UI must display the offered
    /// choices and pass the chosen ID unchanged.
    pub fn resolve_permission(
        &self,
        request_id: &Value,
        option_id: Option<&str>,
    ) -> anyhow::Result<()> {
        // ACP nests the decision one level deeper than it first appears:
        // RequestPermissionResponse has a single `outcome` property of type
        // RequestPermissionOutcome, so the wire shape is
        //   result: { outcome: { outcome: "selected", optionId: ... } }
        // Sending the outcome object flat puts a bare string where the agent
        // expects an object, so it finds no optionId and reports the permission
        // as refused no matter which option was chosen.
        let outcome = match option_id {
            Some(option_id) => json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
            None => json!({ "outcome": { "outcome": "cancelled" } }),
        };
        write_message(
            &self.transport,
            json!({ "jsonrpc": "2.0", "id": request_id, "result": outcome }),
        )
    }

    pub fn recv_event(&self) -> anyhow::Result<OpencodeEvent> {
        self.events
            .lock()
            .unwrap()
            .recv()
            .map_err(|_| anyhow::anyhow!("OpenCode ACP connection closed"))
    }

    pub fn recv_event_timeout(&self, timeout: Duration) -> anyhow::Result<Option<OpencodeEvent>> {
        match self.events.lock().unwrap().recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(anyhow::anyhow!("OpenCode ACP connection closed"))
            }
        }
    }

    /// Send an ACP request to query server-specific configuration. This lets
    /// settings integrations use supported ACP methods without owning stdio.
    pub fn discover(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        request(&self.transport, method, params)
    }

    pub fn shutdown(&self) {
        if let Ok(mut child) = self.child.lock() {
            if let Some(mut child) = child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Own persistent ACP sessions by stable Fastty tab ID. A cwd change replaces
/// that tab's process and session. `reset` starts a fresh session for New Chat.
#[derive(Default)]
pub struct OpencodeManager {
    sessions: Mutex<HashMap<String, Arc<OpencodeRuntime>>>,
}

impl OpencodeManager {
    pub fn session(
        &self,
        tab_id: impl Into<String>,
        cwd: impl AsRef<Path>,
    ) -> anyhow::Result<Arc<OpencodeRuntime>> {
        self.session_with_command(tab_id, "opencode", cwd)
    }

    pub fn session_with_command(
        &self,
        tab_id: impl Into<String>,
        command: impl AsRef<std::ffi::OsStr>,
        cwd: impl AsRef<Path>,
    ) -> anyhow::Result<Arc<OpencodeRuntime>> {
        self.session_with_saved_session(tab_id, command, cwd, None)
    }

    pub fn session_with_saved_session(
        &self,
        tab_id: impl Into<String>,
        command: impl AsRef<std::ffi::OsStr>,
        cwd: impl AsRef<Path>,
        saved_session_id: Option<String>,
    ) -> anyhow::Result<Arc<OpencodeRuntime>> {
        self.session_with_saved_args(tab_id, command, &["acp".to_string()], cwd, saved_session_id)
    }

    pub fn session_with_saved_args(
        &self,
        tab_id: impl Into<String>,
        command: impl AsRef<std::ffi::OsStr>,
        args: &[String],
        cwd: impl AsRef<Path>,
        saved_session_id: Option<String>,
    ) -> anyhow::Result<Arc<OpencodeRuntime>> {
        let tab_id = tab_id.into();
        let cwd = absolute_directory(cwd.as_ref())?;
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(runtime) = sessions.get(&tab_id) {
            if runtime.cwd() == cwd && runtime.command() == command.as_ref() && runtime.args == args {
                let session_matches = saved_session_id
                    .as_deref()
                    .map_or(true, |saved| saved == runtime.session_id());
                if runtime.is_alive() && session_matches {
                    return Ok(Arc::clone(runtime));
                }
                let recovered = Arc::new(match saved_session_id.as_deref() {
                    Some(session_id) => OpencodeRuntime::load_with_args(
                        command.as_ref(),
                        args,
                        &cwd,
                        session_id,
                    )?,
                    None => OpencodeRuntime::spawn_with_args(command.as_ref(), args, &cwd)?,
                });
                sessions.insert(tab_id, Arc::clone(&recovered));
                return Ok(recovered);
            }
        }
        if let Some(runtime) = sessions.remove(&tab_id) {
            runtime.shutdown();
        }
        let runtime = Arc::new(match saved_session_id.as_deref() {
            Some(session_id) => OpencodeRuntime::load_with_args(command, args, &cwd, session_id)?,
            None => OpencodeRuntime::spawn_with_args(command, args, &cwd)?,
        });
        sessions.insert(tab_id, Arc::clone(&runtime));
        Ok(runtime)
    }

    pub fn reset(
        &self,
        tab_id: &str,
        cwd: impl AsRef<Path>,
    ) -> anyhow::Result<Arc<OpencodeRuntime>> {
        self.reset_with_command(tab_id, "opencode", cwd)
    }

    pub fn reset_with_command(
        &self,
        tab_id: &str,
        command: impl AsRef<std::ffi::OsStr>,
        cwd: impl AsRef<Path>,
    ) -> anyhow::Result<Arc<OpencodeRuntime>> {
        self.remove(tab_id);
        self.session_with_command(tab_id.to_string(), command, cwd)
    }

    pub fn remove(&self, tab_id: &str) {
        let prefix = format!("{tab_id}::");
        let mut sessions = self.sessions.lock().unwrap();
        let keys = sessions.keys()
            .filter(|key| key.as_str() == tab_id || key.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(runtime) = sessions.remove(&key) {
                runtime.shutdown();
            }
        }
    }
}

impl Drop for OpencodeRuntime {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            if let Some(mut child) = child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn absolute_directory(path: &Path) -> anyhow::Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(path.canonicalize().unwrap_or(path))
}

fn request(transport: &Arc<Transport>, method: &str, params: Value) -> anyhow::Result<Value> {
    request_inner(transport, method, params, Some(REQUEST_TIMEOUT))
}

fn request_without_timeout(
    transport: &Arc<Transport>,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    request_inner(transport, method, params, None)
}

fn request_before(
    transport: &Arc<Transport>,
    method: &str,
    params: Value,
    deadline: std::time::Instant,
) -> anyhow::Result<Value> {
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    if remaining.is_zero() {
        return Err(anyhow::anyhow!("ACP discovery timed out before {method}"));
    }
    request_inner(transport, method, params, Some(remaining))
}

fn request_inner(
    transport: &Arc<Transport>,
    method: &str,
    params: Value,
    timeout: Option<Duration>,
) -> anyhow::Result<Value> {
    let id = transport.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    transport.pending.lock().unwrap().insert(id, tx);
    if let Err(error) = write_message(
        transport,
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
    ) {
        transport.pending.lock().unwrap().remove(&id);
        return Err(error);
    }
    let response = match timeout {
        Some(timeout) => rx.recv_timeout(timeout).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => anyhow::anyhow!("ACP {method} timed out"),
            mpsc::RecvTimeoutError::Disconnected => {
                anyhow::anyhow!("ACP connection closed during {method}")
            }
        }),
        None => rx
            .recv()
            .map_err(|_| anyhow::anyhow!("ACP connection closed during {method}")),
    };
    match response {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(error)) => Err(anyhow::anyhow!("ACP {method} failed: {error}")),
        Err(error) => {
            transport.pending.lock().unwrap().remove(&id);
            Err(error)
        }
    }
}

fn write_message(transport: &Transport, message: Value) -> anyhow::Result<()> {
    let mut stdin = transport.stdin.lock().unwrap();
    serde_json::to_writer(&mut *stdin, &message)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

fn spawn_reader(
    stdout: std::process::ChildStdout,
    transport: Arc<Transport>,
) -> anyhow::Result<()> {
    thread::Builder::new()
        .name("opencode-acp-reader".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let message = match serde_json::from_str::<Value>(&line) {
                            Ok(message) => message,
                            Err(_) => continue,
                        };
                        dispatch_message(&transport, message);
                    }
                }
            }
            for (_, sender) in transport.pending.lock().unwrap().drain() {
                let _ = sender.send(Err("ACP process exited".into()));
            }
            let _ = transport
                .events
                .send(OpencodeEvent::Error("ACP process exited".into()));
        })?;
    Ok(())
}

fn dispatch_message(transport: &Transport, message: Value) {
    if let Some(method) = message.get("method").and_then(Value::as_str) {
        if let Some(id) = message.get("id") {
            if method == "session/request_permission" {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                let options = params
                    .get("options")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|option| {
                        Some(PermissionOption {
                            option_id: option.get("optionId")?.as_str()?.to_string(),
                            name: option
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("Allow")
                                .to_string(),
                            kind: option
                                .get("kind")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        })
                    })
                    .collect();
                let _ = transport.events.send(OpencodeEvent::PermissionRequest {
                    request_id: id.clone(),
                    tool_call: params.get("toolCall").cloned().unwrap_or(Value::Null),
                    options,
                });
            } else {
                let _ = write_message(
                    transport,
                    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "Unsupported ACP server request" } }),
                );
            }
        } else if method == "session/update" {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            dispatch_update(
                transport,
                params.get("update").cloned().unwrap_or(Value::Null),
            );
        }
        return;
    }
    let Some(id) = message.get("id").and_then(Value::as_u64) else {
        return;
    };
    if let Some(sender) = transport.pending.lock().unwrap().remove(&id) {
        let result = if let Some(error) = message.get("error") {
            Err(error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown JSON-RPC error")
                .to_string())
        } else {
            Ok(message.get("result").cloned().unwrap_or(Value::Null))
        };
        let _ = sender.send(result);
    }
}

fn dispatch_update(transport: &Transport, update: Value) {
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or("");
    let content = update.get("content");
    let text = content
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let event = match kind {
        "agent_message_chunk" => OpencodeEvent::Text(text),
        "agent_thought_chunk" => OpencodeEvent::Thinking(text),
        "tool_call" | "tool_call_update" => OpencodeEvent::ToolUpdate(update),
        "usage_update" => OpencodeEvent::UsageUpdate {
            used: update.get("used").and_then(Value::as_u64).unwrap_or(0),
            size: update.get("size").and_then(Value::as_u64).unwrap_or(0),
        },
        "available_commands_update" => OpencodeEvent::AvailableCommands(
            update
                .get("availableCommands")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|command| {
                    Some(crate::ai::conversations::AvailableCommand {
                        name: command.get("name")?.as_str()?.to_string(),
                        description: command
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    })
                })
                .collect(),
        ),
        _ => OpencodeEvent::OtherUpdate(update),
    };
    let _ = transport.events.send(event);
}

#[cfg(test)]
mod permission_response_tests {
    use super::*;

    /// Mirrors the `result` object that `resolve_permission` puts on the wire.
    fn result_for(option_id: Option<&str>) -> serde_json::Value {
        match option_id {
            Some(id) => json!({ "outcome": { "outcome": "selected", "optionId": id } }),
            None => json!({ "outcome": { "outcome": "cancelled" } }),
        }
    }

    #[test]
    fn outcome_is_nested_as_the_acp_spec_requires() {
        // RequestPermissionResponse.outcome is a RequestPermissionOutcome
        // object, so `result.outcome` must be an object, not a bare string.
        let result = result_for(Some("once"));
        let outcome = &result["outcome"];
        assert!(
            outcome.is_object(),
            "result.outcome must be an object, got {outcome}"
        );
        assert_eq!(outcome["outcome"], "selected");
        assert_eq!(outcome["optionId"], "once");
    }

    #[test]
    fn a_flat_outcome_is_the_bug_this_replaces() {
        // Guards the exact regression: a flat shape deserialises outcome as a
        // string, so an agent cannot read optionId and treats the request as
        // refused regardless of what the user picked.
        let flat = json!({ "outcome": "selected", "optionId": "once" });
        assert!(flat["outcome"].is_string());
        assert!(flat["outcome"]["optionId"].is_null());
    }

    #[test]
    fn cancel_is_nested_too() {
        let result = result_for(None);
        assert_eq!(result["outcome"]["outcome"], "cancelled");
        assert!(result["outcome"]["optionId"].is_null());
    }

    #[test]
    fn every_offered_option_is_echoed_unchanged() {
        for id in ["once", "always", "reject-once", "allow-once"] {
            let result = result_for(Some(id));
            assert_eq!(result["outcome"]["optionId"], id);
        }
    }
}
