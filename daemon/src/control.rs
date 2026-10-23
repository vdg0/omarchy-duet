//! Control socket (JSON-lines) for integrating with the Omarchy panel/bar.
//!
//! Listens on $XDG_RUNTIME_DIR/duet.sock by default. Supports:
//! - start/stop/status/list_devices
//! - emitting turn events: {"type":"turn", ...}
//!
//! Compatible with the current CLI mode: without --control-socket nothing starts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    UserToPeer,
    PeerToUser,
}

#[derive(Clone, Debug, Serialize)]
pub struct TurnEvent {
    #[serde(rename = "type")]
    pub kind: &'static str, // "turn"
    pub dir: Dir,
    pub src_lang: String,
    pub dst_lang: String,
    pub text_src: String,
    pub text_dst: String,
    pub duration_ms: u64,
    pub ts: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Idle,
    Starting,
    Running,
    Stopping,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub running: bool,
    pub mode: Option<String>,
    pub state: State,
    pub pid: u32,
    pub last_turn: Option<TurnEvent>,
    pub log_tail: Vec<String>,
}

#[derive(Clone)]
pub struct Hub {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    status: Status,
    clients: Vec<UnixStream>,
    log_tail: VecDeque<String>,
    stop_requested: bool,
}

#[derive(Deserialize, Debug)]
struct Cmd {
    cmd: String,
    #[serde(flatten)]
    rest: Value,
}

impl Hub {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                status: Status {
                    running: false,
                    mode: None,
                    state: State::Idle,
                    pid: std::process::id(),
                    last_turn: None,
                    log_tail: vec![],
                },
                clients: vec![],
                log_tail: VecDeque::with_capacity(200),
                stop_requested: false,
            })),
        }
    }

    pub fn log(&self, s: impl Into<String>) {
        let s = s.into();
        let mut inner = self.inner.lock().unwrap();
        inner.log_tail.push_back(s.clone());
        while inner.log_tail.len() > 200 {
            inner.log_tail.pop_front();
        }
        inner.status.log_tail = inner.log_tail.iter().cloned().collect();
        // broadcast
        let msg = serde_json::json!({"type":"log","line":s}).to_string() + "\n";
        inner
            .clients
            .retain_mut(|c| c.write_all(msg.as_bytes()).is_ok());
    }

    pub fn set_state(&self, st: State) {
        let mut inner = self.inner.lock().unwrap();
        inner.status.state = st.clone();
        inner.status.running = matches!(st, State::Running);
        let msg = serde_json::json!({"type":"state","state":st}).to_string() + "\n";
        inner
            .clients
            .retain_mut(|c| c.write_all(msg.as_bytes()).is_ok());
    }

    pub fn set_mode(&self, m: Option<String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.status.mode = m;
    }

    pub fn emit_turn(&self, ev: TurnEvent) {
        let mut inner = self.inner.lock().unwrap();
        inner.status.last_turn = Some(ev.clone());
        let msg = serde_json::to_string(&ev).unwrap() + "\n";
        inner
            .clients
            .retain_mut(|c| c.write_all(msg.as_bytes()).is_ok());
    }

    pub fn status_snapshot(&self) -> Status {
        self.inner.lock().unwrap().status.clone()
    }

    /// The panel requests a graceful stop; the audio loop checks it and exits
    /// after cleaning up the virtual nodes (we do not kill the process forcefully).
    pub fn request_stop(&self) {
        self.inner.lock().unwrap().stop_requested = true;
    }

    pub fn stop_requested(&self) -> bool {
        self.inner.lock().unwrap().stop_requested
    }

    pub fn clear_log(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.log_tail.clear();
        inner.status.log_tail.clear();
    }

    pub fn start_server(self: Arc<Self>, path: PathBuf, cmd_tx: Sender<CmdReq>) {
        thread::spawn(move || {
            let _ = std::fs::remove_file(&path);
            let listener = match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[control] bind {}: {e}", path.display());
                    return;
                }
            };
            eprintln!("[control] listening on {}", path.display());
            for stream in listener.incoming() {
                match stream {
                    Ok(s) => {
                        let hub = self.clone();
                        let tx = cmd_tx.clone();
                        thread::spawn(move || handle_client(hub, s, tx));
                    }
                    Err(_) => break,
                }
            }
        });
    }
}

#[derive(Clone, Debug)]
pub struct CmdReq {
    pub cmd: String,
    pub rest: Value,
    pub reply: Arc<Mutex<Option<String>>>,
}

/// Handle commands from the panel. Runs in the daemon process, which is the
/// only one that knows the real pipeline state.
pub fn run_commands(rx: Receiver<CmdReq>, hub: Arc<Hub>) {
    for req in rx {
        let reply = match req.cmd.as_str() {
            "status" => serde_json::to_string(&serde_json::json!({
                "type": "status",
                "payload": hub.status_snapshot(),
            }))
            .unwrap_or_else(|_| r#"{"type":"error","msg":"status"}"#.to_string()),
            "list_devices" => match crate::devices::snapshot() {
                Ok(snapshot) => {
                    let mut value = serde_json::to_value(snapshot).unwrap_or(Value::Null);
                    if let Value::Object(ref mut map) = value {
                        map.insert("type".to_string(), Value::String("devices".to_string()));
                    }
                    value.to_string()
                }
                Err(e) => serde_json::json!({"type":"error","msg":format!("{e:#}")}).to_string(),
            },
            "stop" => {
                hub.request_stop();
                hub.set_state(State::Stopping);
                serde_json::json!({"type":"ok","cmd":"stop"}).to_string()
            }
            "clear_log" => {
                hub.clear_log();
                serde_json::json!({"type":"ok","cmd":"clear_log"}).to_string()
            }
            "start" => serde_json::json!({
                "type": "error",
                "msg": "start is handled by duet-toggle; the socket only controls an already running daemon",
            })
            .to_string(),
            other => serde_json::json!({
                "type": "error",
                "msg": format!("unknown command: {other}"),
            })
            .to_string(),
        };
        *req.reply.lock().unwrap() = Some(reply);
        let _ = req.rest;
        let _ = &hub;
    }
}

fn handle_client(hub: Arc<Hub>, mut stream: UnixStream, cmd_tx: Sender<CmdReq>) {
    // send hello/status
    let st = hub.status_snapshot();
    let _ = writeln!(
        stream,
        "{}",
        serde_json::json!({"type":"hello","pid":st.pid})
    );
    let _ = writeln!(
        stream,
        "{}",
        serde_json::to_string(&serde_json::json!({"type":"status", "payload": st})).unwrap()
    );
    // register for broadcasts
    if let Ok(clone) = stream.try_clone() {
        hub.inner.lock().unwrap().clients.push(clone);
    }
    let reader = BufReader::new(stream.try_clone().unwrap());
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let c: Cmd = match serde_json::from_str(&line) {
            Ok(x) => x,
            Err(e) => {
                let _ = writeln!(
                    stream,
                    "{}",
                    serde_json::json!({"type":"error","msg":format!("{e}")})
                );
                continue;
            }
        };
        let reply_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let req = CmdReq {
            cmd: c.cmd.clone(),
            rest: c.rest,
            reply: reply_slot.clone(),
        };
        if cmd_tx.send(req).is_err() {
            break;
        }
        // Wait briefly for a reply (list_devices runs pw-dump, a few ms).
        for _ in 0..200 {
            thread::sleep(std::time::Duration::from_millis(10));
            if let Some(r) = reply_slot.lock().unwrap().clone() {
                let _ = writeln!(stream, "{}", r);
                break;
            }
        }
    }
}

pub fn default_socket_path() -> PathBuf {
    if let Ok(rd) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(rd).join("duet.sock")
    } else {
        std::env::temp_dir().join("duet.sock")
    }
}
