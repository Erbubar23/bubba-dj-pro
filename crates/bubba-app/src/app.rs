use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use crossbeam_channel::Sender;
use serde_json::{json, Value};

use bubba_core::{AudioCommand, MixerSnapshot};

use crate::config::{CachedAnalysis, Config};

/// Requests handled by the main thread, which owns the audio streams
pub enum RoutingRequest {
    Set { master: Option<String>, headphone: Option<String> },
}

/// Fan-out of messages to every connected UI window
#[derive(Default)]
pub struct Hub {
    clients: Mutex<Vec<(u64, Sender<Arc<str>>)>>,
    next_id: AtomicU64,
    pub ever_connected: AtomicBool,
}

impl Hub {
    pub fn add(&self, tx: Sender<Arc<str>>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.ever_connected.store(true, Ordering::Relaxed);
        if let Ok(mut c) = self.clients.lock() {
            c.push((id, tx));
        }
        id
    }

    pub fn remove(&self, id: u64) {
        if let Ok(mut c) = self.clients.lock() {
            c.retain(|(cid, _)| *cid != id);
        }
    }

    pub fn count(&self) -> usize {
        self.clients.lock().map(|c| c.len()).unwrap_or(0)
    }

    pub fn broadcast(&self, msg: &Value) {
        self.broadcast_text(Arc::from(msg.to_string()));
    }

    pub fn broadcast_text(&self, text: Arc<str>) {
        if let Ok(mut c) = self.clients.lock() {
            c.retain(|(_, tx)| tx.send(Arc::clone(&text)).is_ok());
        }
    }
}

#[derive(Default, Clone, serde::Serialize)]
pub struct MidiStatus {
    pub connected: bool,
    pub name: Option<String>,
}

pub struct App {
    pub cmd_tx: Sender<AudioCommand>,
    pub state: Arc<RwLock<MixerSnapshot>>,
    pub hub: Hub,
    pub token: String,
    pub exe_dir: Option<PathBuf>,
    pub routing_tx: Sender<RoutingRequest>,
    /// Latest `audio_routing` message (sent to new windows)
    pub routing_msg: RwLock<Value>,
    pub midi: RwLock<MidiStatus>,
    /// Latest `track` message per deck (includes the waveform)
    pub tracks: RwLock<[Option<Arc<str>>; 2]>,
    pub load_generation: [AtomicU64; 2],
    pub config: Mutex<Config>,
    pub analysis_cache: Mutex<HashMap<String, CachedAnalysis>>,
}

impl App {
    pub fn send(&self, cmd: AudioCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn snapshot(&self) -> Option<MixerSnapshot> {
        self.state.read().ok().map(|s| *s)
    }

    pub fn toast(&self, level: &str, message: impl Into<String>) {
        self.hub.broadcast(&json!({ "type": "toast", "level": level, "message": message.into() }));
    }

    pub fn midi_msg(&self) -> Value {
        let status = self.midi.read().map(|m| m.clone()).unwrap_or_default();
        json!({ "type": "midi_status", "connected": status.connected, "name": status.name })
    }
}

/// Random-enough session token (std's RandomState is seeded from the OS)
pub fn make_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}
