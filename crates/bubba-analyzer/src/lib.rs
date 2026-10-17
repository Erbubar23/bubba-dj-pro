pub mod filter;
pub mod decoder;
pub mod beatgrid;
pub mod cues;
pub mod waveform;
pub mod key_detector;

use std::path::Path;
use std::time::Instant;
use anyhow::Result;

pub use decoder::{AudioDecoder, DecodedAudio, TrackTags};
pub use beatgrid::{BeatTracker, BeatGrid, BpmRange, GridSource};
pub use cues::{CueDetector, CueKind, CuePoint};
pub use waveform::{WaveformGenerator, WaveformData, WaveformFrame};
pub use key_detector::{KeyDetector, KeyResult};

/// Bumped whenever the grid / cue analysis changes, so cached results get recomputed
pub const ANALYSIS_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct AnalyzerConfig {
    pub bpm_range: BpmRange,
    /// Use the neural beat tracker when refining (needs the `neural` feature)
    pub neural: bool,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        AnalyzerConfig { bpm_range: BpmRange::default(), neural: true }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrackAnalysisResult {
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub beatgrid: BeatGrid,
    pub cues: Vec<CuePoint>,
    pub waveform: WaveformData,
    pub key: KeyResult,
    /// Time taken to analyze the entire track in milliseconds
    pub analysis_time_ms: u128,
}

/// Result of the slow, precise grid pass
#[derive(Debug, Clone)]
pub struct RefinedGrid {
    pub beatgrid: BeatGrid,
    pub cues: Vec<CuePoint>,
    pub time_ms: u128,
}

pub struct Analyzer;

impl Analyzer {
    /// Decodes and fully analyzes a file (quick pass + precise grid)
    pub fn analyze_file<P: AsRef<Path>>(path: P) -> Result<TrackAnalysisResult> {
        let decoded = AudioDecoder::decode_file(path)?;
        let mono = decoded.to_mono();
        let cfg = AnalyzerConfig::default();
        let mut result = Self::analyze_mono(&mono, &decoded, &cfg);
        let refined = Self::refine_beatgrid(&mono, decoded.sample_rate, &cfg);
        result.beatgrid = refined.beatgrid;
        result.cues = refined.cues;
        result.analysis_time_ms += refined.time_ms;
        Ok(result)
    }

    /// Quick pass on already-decoded audio: DSP grid -> Camelot key -> 3-band waveform (< 1 s)
    pub fn analyze_decoded(decoded: &DecodedAudio) -> TrackAnalysisResult {
        Self::analyze_mono(&decoded.to_mono(), decoded, &AnalyzerConfig::default())
    }

    /// Quick pass with a precomputed mono mix (so the caller can reuse it for `refine_beatgrid`)
    pub fn analyze_mono(mono: &[f32], decoded: &DecodedAudio, cfg: &AnalyzerConfig) -> TrackAnalysisResult {
        let start = Instant::now();
        let beatgrid = BeatTracker::analyze_range(mono, decoded.sample_rate, cfg.bpm_range);
        let mut result = Self::analyze_with_grid(mono, decoded, beatgrid);
        result.analysis_time_ms = start.elapsed().as_millis();
        result
    }

    /// Key + waveform + cues around a grid that is already known (e.g. from the analysis cache)
    pub fn analyze_with_grid(mono: &[f32], decoded: &DecodedAudio, beatgrid: BeatGrid) -> TrackAnalysisResult {
        let start = Instant::now();
        let key = KeyDetector::analyze(mono, decoded.sample_rate);
        // 150 waveform frames per second of audio
        let waveform = WaveformGenerator::generate(mono, decoded.sample_rate, 150);
        let cues = CueDetector::suggest(mono, decoded.sample_rate, &beatgrid);

        TrackAnalysisResult {
            duration_seconds: decoded.duration_seconds,
            sample_rate: decoded.sample_rate,
            channels: decoded.channels,
            beatgrid,
            cues,
            waveform,
            key,
            analysis_time_ms: start.elapsed().as_millis(),
        }
    }

    /// Precise grid (2-4 s): neural beats/downbeats fitted to a constant grid, then
    /// refined over the whole track by the DSP tracker. Never fails: falls back to DSP.
    pub fn refine_beatgrid(mono: &[f32], sample_rate: u32, cfg: &AnalyzerConfig) -> RefinedGrid {
        let start = Instant::now();
        let env = beatgrid::onset::OnsetEnvelope::compute(mono, sample_rate);
        let fit = if cfg.neural { Self::neural_fit(mono, sample_rate, &env, cfg) } else { None };
        let beatgrid = beatgrid::refine::combine(&env, fit.as_ref(), cfg.bpm_range).unwrap_or_else(BeatGrid::fallback);
        let cues = CueDetector::suggest(mono, sample_rate, &beatgrid);
        RefinedGrid { beatgrid, cues, time_ms: start.elapsed().as_millis() }
    }

    #[cfg(feature = "neural")]
    fn neural_fit(mono: &[f32], sample_rate: u32, env: &beatgrid::onset::OnsetEnvelope, cfg: &AnalyzerConfig) -> Option<beatgrid::fit::GridFit> {
        match beatgrid::neural::detect(mono, sample_rate, env) {
            Ok(nb) => beatgrid::fit::fit_constant(&nb.beats, &nb.downbeats, cfg.bpm_range),
            Err(e) => {
                log::warn!("neural beat tracker unavailable: {:#}", e);
                None
            }
        }
    }

    #[cfg(not(feature = "neural"))]
    fn neural_fit(_: &[f32], _: u32, _: &beatgrid::onset::OnsetEnvelope, _: &AnalyzerConfig) -> Option<beatgrid::fit::GridFit> {
        None
    }
}
