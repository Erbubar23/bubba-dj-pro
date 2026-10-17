//! Analysis cache and the messages that carry analysis results to the UI.
//! The loader decides *when* to analyze; this module knows what is stored and sent.

use std::path::Path;
use std::sync::Arc;
use serde_json::{json, Value};

use bubba_analyzer::{BeatGrid, CuePoint, TrackAnalysisResult, ANALYSIS_VERSION};

use crate::app::App;
use crate::config::{file_stamp, save_analysis_cache, CachedAnalysis};
use crate::library::base64_encode;

/// Grid stored for this exact file by the current analyzer (or edited by the user)
pub fn cached_grid(app: &App, path: &Path) -> Option<BeatGrid> {
    let key = path.to_string_lossy();
    let stamp = file_stamp(path);
    let cache = app.analysis_cache.lock().ok()?;
    cache.get(key.as_ref()).and_then(|c| c.grid_for(stamp)).cloned()
}

pub struct TrackInfo<'a> {
    pub path: &'a Path,
    pub title: &'a str,
    pub artist: Option<&'a str>,
}

/// Stores the library summary; `final_grid` marks the grid as precise (skips re-analysis)
pub fn store(app: &App, info: &TrackInfo, analysis: &TrackAnalysisResult, final_grid: bool) {
    let (file_size, modified) = file_stamp(info.path);
    let entry = CachedAnalysis {
        title: info.title.to_string(),
        artist: info.artist.map(String::from),
        bpm: analysis.beatgrid.bpm,
        camelot: analysis.key.camelot.clone(),
        key: analysis.key.key_name.clone(),
        duration: analysis.duration_seconds,
        beatgrid: final_grid.then(|| analysis.beatgrid.clone()),
        cues: if final_grid { analysis.cues.clone() } else { Vec::new() },
        file_size,
        modified,
        analysis_version: ANALYSIS_VERSION,
    };
    if let Ok(mut cache) = app.analysis_cache.lock() {
        cache.insert(info.path.to_string_lossy().to_string(), entry);
        save_analysis_cache(&cache);
    }
}

/// `track` message: everything the UI draws for a deck (waveform included)
pub fn track_message(deck: usize, info: &TrackInfo, analysis: &TrackAnalysisResult, refining: bool) -> Value {
    // Compact waveform: 4 bytes per frame (low, mid, high, peak)
    let mut wf = Vec::with_capacity(analysis.waveform.frames.len() * 4);
    for f in &analysis.waveform.frames {
        wf.extend_from_slice(&[f.low, f.mid, f.high, f.peak]);
    }
    let spf = (analysis.sample_rate / analysis.waveform.frames_per_second.max(1)).max(1);
    let fps = analysis.sample_rate as f64 / spf as f64;

    json!({
        "type": "track",
        "deck": deck,
        "path": info.path.to_string_lossy(),
        "title": info.title,
        "artist": info.artist,
        "bpm": analysis.beatgrid.bpm,
        "key": analysis.key.key_name,
        "camelot": analysis.key.camelot,
        "key_confidence": analysis.key.confidence,
        "duration": analysis.duration_seconds,
        "sample_rate": analysis.sample_rate,
        "fps": fps,
        "waveform": base64_encode(&wf),
        "beatgrid": analysis.beatgrid,
        "cues": analysis.cues,
        "refining": refining,
        "analysis_ms": analysis.analysis_time_ms,
    })
}

/// Sends the refined grid to every window and patches the stored `track` message
/// so windows opened later get it too.
pub fn publish_grid(app: &App, deck: usize, track_msg: &mut Value, grid: &BeatGrid, cues: &[CuePoint]) {
    let path = track_msg["path"].clone();
    track_msg["bpm"] = json!(grid.bpm);
    track_msg["beatgrid"] = json!(grid);
    track_msg["cues"] = json!(cues);
    track_msg["refining"] = json!(false);
    let text: Arc<str> = Arc::from(track_msg.to_string());
    if let Ok(mut t) = app.tracks.write() {
        t[deck] = Some(text);
    }
    app.hub.broadcast(&json!({
        "type": "grid",
        "deck": deck,
        "path": path,
        "bpm": grid.bpm,
        "beatgrid": grid,
        "cues": cues,
    }));
}
