use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use tauri::{Manager, State};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MidiPort { id: String, name: String }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceStatus { available: bool, message: String, ports: Vec<MidiPort>, backend: String }

#[derive(Clone, Copy, Deserialize)]
enum MidiBackend {
    #[serde(rename = "teVirtualMidi")]
    TeVirtualMidi,
    #[serde(rename = "windowsMidiServices")]
    WindowsMidiServices,
}

impl MidiBackend {
    fn name(self) -> &'static str {
        match self {
            Self::TeVirtualMidi => "teVirtualMidi",
            Self::WindowsMidiServices => "windowsMidiServices",
        }
    }
}

#[derive(Deserialize)]
struct HostEnvelope { response: Value }

struct MidiHost {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
    _job: Option<WindowsJob>,
}

impl MidiHost {
    fn start(executable: &Path, backend: MidiBackend) -> Result<Self, String> {
        if !executable.is_file() {
            return Err(format!("MIDI host executable was not found at {}", executable.display()));
        }
        let mut command = Command::new(executable);
        command.arg(format!("--backend={}", backend.name()));
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|error| format!("Could not start MIDI host: {error}"))?;
        let input = child.stdin.take().ok_or("MIDI host stdin is unavailable")?;
        let stdout = child.stdout.take().ok_or("MIDI host stdout is unavailable")?;
        let job = WindowsJob::assign(&child);
        Ok(Self { child, input, output: BufReader::new(stdout), next_id: 1, _job: job })
    }

    fn request(&mut self, command: &str, args: Value) -> Result<Value, String> {
        if let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!("MIDI host exited unexpectedly ({status})."));
        }
        let id = self.next_id.to_string();
        self.next_id += 1;
        let mut request = args;
        let object = request.as_object_mut().ok_or("Invalid MIDI host request")?;
        object.insert("id".into(), json!(id));
        object.insert("command".into(), json!(command));
        serde_json::to_writer(&mut self.input, &request).map_err(|error| error.to_string())?;
        self.input.write_all(b"\n").map_err(|error| error.to_string())?;
        self.input.flush().map_err(|error| error.to_string())?;
        let mut line = String::new();
        if self.output.read_line(&mut line).map_err(|error| error.to_string())? == 0 {
            let exit = self.child.try_wait().ok().flatten()
                .map(|status| format!(" The host exited with status {status}."))
                .unwrap_or_default();
            return Err(format!("MIDI host closed its response channel.{exit}"));
        }
        let envelope: HostEnvelope = serde_json::from_str(&line).map_err(|error| format!("Invalid MIDI host response: {error}"))?;
        if envelope.response.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(envelope.response.get("error").and_then(Value::as_str).unwrap_or("MIDI host request failed.").to_string());
        }
        Ok(envelope.response)
    }
}

impl Drop for MidiHost {
    fn drop(&mut self) {
        let _ = self.request("shutdown", json!({}));
        let _ = self.child.wait();
    }
}

#[cfg(windows)]
struct WindowsJob(isize);

#[cfg(windows)]
impl WindowsJob {
    fn assign(child: &Child) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct BasicLimits {
            process_time: i64, job_time: i64, flags: u32, min_working_set: usize,
            max_working_set: usize, active_processes: u32, affinity: usize,
            priority: u32, scheduling_class: u32,
        }
        #[repr(C)]
        struct IoCounters { read_ops: u64, write_ops: u64, other_ops: u64, read_bytes: u64, write_bytes: u64, other_bytes: u64 }
        #[repr(C)]
        struct ExtendedLimits {
            basic: BasicLimits, io: IoCounters, process_memory: usize, job_memory: usize,
            peak_process_memory: usize, peak_job_memory: usize,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateJobObjectW(attributes: *const u8, name: *const u16) -> isize;
            fn SetInformationJobObject(job: isize, class: i32, info: *const u8, length: u32) -> i32;
            fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
            fn CloseHandle(handle: isize) -> i32;
        }
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job == 0 { return None; }
            let mut limits: ExtendedLimits = std::mem::zeroed();
            limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            let configured = SetInformationJobObject(job, 9, &limits as *const _ as *const u8, std::mem::size_of::<ExtendedLimits>() as u32) != 0;
            let assigned = configured && AssignProcessToJobObject(job, child.as_raw_handle() as isize) != 0;
            if assigned { Some(Self(job)) } else { let _ = CloseHandle(job); None }
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        unsafe extern "system" { fn CloseHandle(handle: isize) -> i32; }
        unsafe { let _ = CloseHandle(self.0); }
    }
}

#[cfg(not(windows))]
struct WindowsJob;
#[cfg(not(windows))]
impl WindowsJob { fn assign(_: &Child) -> Option<Self> { None } }

struct MidiHostController {
    executable: PathBuf,
    backend: MidiBackend,
    host: Option<MidiHost>,
    startup_error: Option<String>,
}

impl MidiHostController {
    fn new(executable: PathBuf) -> Self {
        let mut controller = Self {
            executable,
            backend: MidiBackend::TeVirtualMidi,
            host: None,
            startup_error: None,
        };
        controller.start_backend(MidiBackend::TeVirtualMidi);
        controller
    }

    fn start_backend(&mut self, backend: MidiBackend) {
        self.backend = backend;
        match MidiHost::start(&self.executable, backend) {
            Ok(host) => { self.host = Some(host); self.startup_error = None; }
            Err(error) => { self.host = None; self.startup_error = Some(error); }
        }
    }

    fn status(&mut self) -> ServiceStatus {
        let backend = self.backend.name().to_string();
        let Some(host) = self.host.as_mut() else {
            return ServiceStatus {
                available: false,
                message: self.startup_error.clone().unwrap_or_else(|| "MIDI host is unavailable.".into()),
                ports: Vec::new(),
                backend,
            };
        };
        match host.request("status", json!({})) {
            Ok(response) => {
                let available = response.get("available").and_then(Value::as_bool).unwrap_or(false);
                let message = response.get("message").and_then(Value::as_str).unwrap_or("MIDI backend status is unknown.").to_string();
                let ports = host.request("list", json!({})).ok()
                    .and_then(|value| serde_json::from_value(value.get("ports").cloned().unwrap_or(Value::Null)).ok())
                    .unwrap_or_default();
                ServiceStatus { available, message, ports, backend }
            }
            Err(error) => ServiceStatus { available: false, message: error, ports: Vec::new(), backend },
        }
    }
}

#[tauri::command]
fn service_status(state: State<'_, Mutex<MidiHostController>>) -> ServiceStatus {
    state.lock().map(|mut controller| controller.status()).unwrap_or_else(|_| ServiceStatus {
        available: false, message: "MIDI host state is unavailable.".into(), ports: Vec::new(), backend: "unknown".into(),
    })
}

#[tauri::command]
fn set_backend(backend: MidiBackend, state: State<'_, Mutex<MidiHostController>>) -> ServiceStatus {
    let Ok(mut controller) = state.lock() else {
        return ServiceStatus { available: false, message: "MIDI host state is unavailable.".into(), ports: Vec::new(), backend: "unknown".into() };
    };
    if controller.backend.name() != backend.name() {
        drop(controller.host.take());
        controller.startup_error = None;
        controller.start_backend(backend);
    }
    controller.status()
}

#[tauri::command]
fn list_ports(state: State<'_, Mutex<MidiHostController>>) -> Result<Vec<MidiPort>, String> {
    let mut controller = state.lock().map_err(|_| "MIDI host state is unavailable.")?;
    if controller.host.is_none() {
        return Err(controller.startup_error.clone().unwrap_or_else(|| "MIDI host is unavailable.".into()));
    }
    let host = controller.host.as_mut().expect("MIDI host was checked above");
    let response = host.request("list", json!({}))?;
    serde_json::from_value(response.get("ports").cloned().unwrap_or(Value::Null)).map_err(|error| error.to_string())
}

#[tauri::command]
fn create_port(name: String, state: State<'_, Mutex<MidiHostController>>) -> Result<MidiPort, String> {
    let mut controller = state.lock().map_err(|_| "MIDI host state is unavailable.")?;
    if controller.host.is_none() {
        return Err(controller.startup_error.clone().unwrap_or_else(|| "MIDI host is unavailable.".into()));
    }
    let host = controller.host.as_mut().expect("MIDI host was checked above");
    let response = host.request("create", json!({ "name": name }))?;
    serde_json::from_value(response.get("port").cloned().unwrap_or(Value::Null)).map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_port(id: String, state: State<'_, Mutex<MidiHostController>>) -> Result<(), String> {
    let mut controller = state.lock().map_err(|_| "MIDI host state is unavailable.")?;
    if controller.host.is_none() {
        return Err(controller.startup_error.clone().unwrap_or_else(|| "MIDI host is unavailable.".into()));
    }
    let host = controller.host.as_mut().expect("MIDI host was checked above");
    host.request("delete", json!({ "portId": id }))?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let bundled = app.path().resource_dir().ok().map(|dir| dir.join("midi-host.exe"));
            let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.midi-fiesta-build/midi-host/publish/app/MidiHost.exe");
            let executable = if cfg!(debug_assertions) {
                dev.is_file().then_some(dev).or(bundled)
            } else {
                bundled.filter(|path| path.is_file()).or(Some(dev))
            }.expect("MIDI host path is unavailable");
            app.manage(Mutex::new(MidiHostController::new(executable)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![service_status, set_backend, list_ports, create_port, delete_port])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
