use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use bubba_analyzer::{BeatGrid, CuePoint, GridSource, ANALYSIS_VERSION};
use bubba_core::SyncMode;

/// Persistent user settings (%APPDATA%\BubbaDJPro\config.json)
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Config {
    pub master_device: Option<String>,
    pub headphone_device: Option<String>,
    pub last_folder: Option<String>,
    /// rekordbox-style or Serato-style ("punto 0") sync
    #[serde(default)]
    pub sync_mode: SyncMode,
}

/// Summary of an analyzed track, shown in the library (BPM / key columns)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedAnalysis {
    pub title: String,
    pub artist: Option<String>,
    pub bpm: f32,
    pub camelot: String,
    pub key: String,
    pub duration: f64,
    /// Precise grid + cue suggestions (absent in caches written before v0.5)
    #[serde(default)]
    pub beatgrid: Option<BeatGrid>,
    #[serde(default)]
    pub cues: Vec<CuePoint>,
    /// File identity: a re-saved / replaced file is analyzed again
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub modified: u64,
    #[serde(default)]
    pub analysis_version: u32,
}

/// (size, modification time in seconds) of a file
pub fn file_stamp(path: &Path) -> (u64, u64) {
    fs::metadata(path)
        .map(|m| {
            let modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (m.len(), modified)
        })
        .unwrap_or((0, 0))
}

impl CachedAnalysis {
    /// The stored grid, if it was made for this exact file by the current analyzer
    /// (a grid edited by the user is kept across analyzer versions)
    pub fn grid_for(&self, stamp: (u64, u64)) -> Option<&BeatGrid> {
        let same_file = (self.file_size, self.modified) == stamp;
        self.beatgrid
            .as_ref()
            .filter(|g| same_file && (self.analysis_version == ANALYSIS_VERSION || g.source == GridSource::Manual))
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = base.join("BubbaDJPro");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn load_json<T: serde::de::DeserializeOwned + Default>(name: &str) -> T {
    fs::read_to_string(data_dir().join(name))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_json<T: serde::Serialize>(name: &str, value: &T) {
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let path = data_dir().join(name);
        let tmp = path.with_extension("tmp");
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

impl Config {
    pub fn load() -> Self {
        load_json("config.json")
    }

    pub fn save(&self) {
        save_json("config.json", self);
    }
}

pub fn load_analysis_cache() -> HashMap<String, CachedAnalysis> {
    load_json("library.json")
}

pub fn save_analysis_cache(cache: &HashMap<String, CachedAnalysis>) {
    save_json("library.json", cache);
}

/// Appends a line to %APPDATA%\BubbaDJPro\bubba.log (the release build has no console)
pub fn log_line(line: &str) {
    println!("{}", line);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(data_dir().join("bubba.log")) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", secs, line);
    }
}

/// Starts a fresh log each session, keeping it small
pub fn rotate_log() {
    let path = data_dir().join("bubba.log");
    if fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = fs::rename(&path, data_dir().join("bubba.old.log"));
    }
}
