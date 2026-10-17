//! Beat This! (Foscarin, Schlüter, Widmer — ISMIR 2024) beat + downbeat tracker,
//! via the pure-Rust `beat-this` port (rten backend, MIT). The small model (~10 MB)
//! is embedded in the executable and extracted next to the app data on first use.
//!
//! Inference costs ~50 ms per second of audio on a desktop CPU and does not scale
//! with threads, so only two 30 s excerpts are analyzed: enough for the tempo octave
//! and the bar position. The DSP tracker then refines the exact tempo over the whole track.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use beat_this::{BeatThis, RtenRuntime, Runtime};

use super::onset::OnsetEnvelope;

const MEL_MODEL: &[u8] = include_bytes!("../../models/mel_spectrogram.onnx");
const BEAT_MODEL: &[u8] = include_bytes!("../../models/beat_this_small.onnx");

/// Length of each analyzed excerpt: fits in ONE model chunk (1500 frames at 50 fps,
/// stride 1488). Exactly 30 s would spill into a second chunk and double the cost.
const EXCERPT_SECONDS: f64 = 29.5;
/// Tracks shorter than this are analyzed whole
const WHOLE_TRACK_SECONDS: f64 = 75.0;

type Model = <RtenRuntime as Runtime>::Model;

static MODEL_DIR: OnceLock<PathBuf> = OnceLock::new();
static TRACKER: OnceLock<Mutex<Option<BeatThis<Model>>>> = OnceLock::new();

/// Beats and downbeats in seconds (track time)
#[derive(Debug, Clone, Default)]
pub struct NeuralBeats {
    pub beats: Vec<f64>,
    pub downbeats: Vec<f64>,
}

/// Where the model files are extracted (call once at startup; defaults to the temp dir)
pub fn set_model_dir(dir: &Path) {
    let _ = MODEL_DIR.set(dir.to_path_buf());
}

/// Loads the model ahead of time so the first track does not pay for it
pub fn warm_up() -> Result<()> {
    with_tracker(|_| Ok(()))
}

fn with_tracker<T>(f: impl FnOnce(&mut BeatThis<Model>) -> Result<T>) -> Result<T> {
    let cell = TRACKER.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().map_err(|_| anyhow!("beat model lock poisoned"))?;
    if guard.is_none() {
        let dir = MODEL_DIR.get().cloned().unwrap_or_else(|| std::env::temp_dir().join("BubbaDJPro"));
        let models = dir.join("models");
        let mel = extract(&models, "mel_spectrogram.onnx", MEL_MODEL)?;
        let beat = extract(&models, "beat_this_small.onnx", BEAT_MODEL)?;
        *guard = Some(BeatThis::new(&RtenRuntime, &mel, &beat).context("no se pudo cargar el modelo de beats")?);
    }
    f(guard.as_mut().unwrap())
}

fn extract(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    let path = dir.join(name);
    if fs::metadata(&path).map(|m| m.len() == bytes.len() as u64).unwrap_or(false) {
        return Ok(path);
    }
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Runs the model on the busiest excerpts of the track
pub fn detect(mono: &[f32], sample_rate: u32, env: &OnsetEnvelope) -> Result<NeuralBeats> {
    let sr = sample_rate as f64;
    let duration = mono.len() as f64 / sr;
    let windows = if duration <= WHOLE_TRACK_SECONDS {
        vec![(0.0, duration)]
    } else {
        pick_excerpts(env, duration)
    };

    with_tracker(|bt| {
        let mut out = NeuralBeats::default();
        for (start, end) in windows {
            let (s, e) = ((start * sr) as usize, ((end * sr) as usize).min(mono.len()));
            let a = bt.analyze_audio(&mono[s..e], sample_rate)?;
            out.beats.extend(a.beats.iter().map(|&b| b as f64 + start));
            out.downbeats.extend(a.downbeats.iter().map(|&b| b as f64 + start));
        }
        Ok(out)
    })
}

/// Two non-overlapping excerpts with the most kick energy (skips beatless intros / breakdowns)
fn pick_excerpts(env: &OnsetEnvelope, duration: f64) -> Vec<(f64, f64)> {
    const BLOCK: f64 = 5.0;
    let blocks = env.block_energy(BLOCK);
    let per_window = (EXCERPT_SECONDS / BLOCK) as usize;
    if blocks.len() <= per_window {
        return vec![(0.0, duration.min(EXCERPT_SECONDS))];
    }
    let scores: Vec<f32> = (0..=blocks.len() - per_window).map(|i| blocks[i..i + per_window].iter().sum()).collect();
    let best = |exclude: Option<usize>| {
        scores
            .iter()
            .enumerate()
            .filter(|(i, _)| exclude.map_or(true, |x| i.abs_diff(x) >= per_window))
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
    };
    let mut out = Vec::new();
    let first = best(None);
    for i in [first, first.and_then(|f| best(Some(f)))].into_iter().flatten() {
        let start = i as f64 * BLOCK;
        out.push((start, (start + EXCERPT_SECONDS).min(duration)));
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}
