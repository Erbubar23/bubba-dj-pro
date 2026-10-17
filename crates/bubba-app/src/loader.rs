use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use serde_json::json;

use bubba_analyzer::{Analyzer, AnalyzerConfig, AudioDecoder};
use bubba_core::AudioCommand;

use crate::analysis::{self, TrackInfo};
use crate::app::App;
use crate::config::log_line;
use crate::library::split_artist_title;

const DECK_NAMES: [&str; 2] = ["1", "2"];

/// Decodes + analyzes a track on a worker thread and loads it into a deck.
/// Refuses to replace a playing deck unless `force` is set (protects the live mix).
pub fn load_track(app: &Arc<App>, deck: usize, path: PathBuf, force: bool) {
    if deck > 1 {
        return;
    }
    // Absolute, normalized path: keys the analysis cache and matches library listings
    let path = std::path::absolute(&path).unwrap_or(path);
    if !path.is_file() {
        app.toast("error", format!("No se encontró el archivo: {}", path.display()));
        return;
    }
    if !force {
        if let Some(s) = app.snapshot() {
            let d = if deck == 0 { s.deck1 } else { s.deck2 };
            if d.is_playing {
                app.toast(
                    "warn",
                    format!("El Deck {} está sonando. Páusalo antes de cargar (o mantén Shift para forzar).", DECK_NAMES[deck]),
                );
                return;
            }
        }
    }

    let generation = app.load_generation[deck].fetch_add(1, Ordering::SeqCst) + 1;
    let file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    app.hub.broadcast(&json!({ "type": "loading", "deck": deck, "name": file_name }));

    let app = Arc::clone(app);
    thread::spawn(move || {
        if let Err(e) = load_worker(&app, deck, &path, generation) {
            log_line(&format!("Error cargando {:?}: {:#}", path, e));
            app.hub.broadcast(&json!({ "type": "loading_failed", "deck": deck }));
            app.toast("error", format!("No se pudo cargar \"{}\": {}", file_name, e));
        }
    });
}

/// 1. Decode, then the quick analysis (or the cached precise grid): the deck is playable in < 1 s.
/// 2. If the grid is not cached yet, the precise neural grid is computed on this same thread
///    (2-4 s) and swapped in while the track is already loaded.
fn load_worker(app: &Arc<App>, deck: usize, path: &Path, generation: u64) -> anyhow::Result<()> {
    let current = || app.load_generation[deck].load(Ordering::SeqCst) == generation;

    let (audio, tags) = AudioDecoder::decode_file_with_tags(path)?;
    if !current() {
        return Ok(()); // a newer load for this deck superseded us
    }

    let cfg = AnalyzerConfig::default();
    let mono = audio.to_mono();
    let cached = analysis::cached_grid(app, path);
    let needs_refine = cached.is_none();
    let mut result = match cached {
        Some(grid) => Analyzer::analyze_with_grid(&mono, &audio, grid),
        None => Analyzer::analyze_mono(&mono, &audio, &cfg),
    };
    if !current() {
        return Ok(());
    }

    let (artist_guess, title_guess) = split_artist_title(path);
    let title = tags.title.clone().unwrap_or(title_guess);
    let artist = tags.artist.clone().or(artist_guess);
    let info = TrackInfo { path, title: &title, artist: artist.as_deref() };

    log_line(&format!(
        "Deck {} <- {} | {:.2} BPM ({:?}{}) | {} ({}) | 1er beat {:.3}s | {} ms",
        DECK_NAMES[deck], title, result.beatgrid.bpm, result.beatgrid.source,
        if needs_refine { ", refinando" } else { ", caché" },
        result.key.camelot, result.key.key_name, result.beatgrid.cue_seconds(), result.analysis_time_ms
    ));

    let mut msg = analysis::track_message(deck, &info, &result, needs_refine);
    analysis::store(app, &info, &result, !needs_refine);

    let sample_rate = audio.sample_rate;
    app.send(AudioCommand::LoadTrack {
        deck,
        audio,
        beatgrid: Some(result.beatgrid.clone()),
        key: Some(result.key.clone()),
    });
    let text: Arc<str> = Arc::from(msg.to_string());
    if let Ok(mut t) = app.tracks.write() {
        t[deck] = Some(Arc::clone(&text));
    }
    app.hub.broadcast_text(text);

    if !needs_refine {
        return Ok(());
    }

    // ---- Precise grid (background, the deck is already playable) ----
    let refined = Analyzer::refine_beatgrid(&mono, sample_rate, &cfg);
    drop(mono);
    log_line(&format!(
        "Deck {} rejilla refinada: {:.2} BPM ({:?}, confianza {:.2}) | compás 1 {:.3}s | 1er beat {:.3}s | {} cues | {} ms",
        DECK_NAMES[deck], refined.beatgrid.bpm, refined.beatgrid.source, refined.beatgrid.confidence,
        refined.beatgrid.first_downbeat_seconds, refined.beatgrid.cue_seconds(), refined.cues.len(), refined.time_ms
    ));

    result.beatgrid = refined.beatgrid;
    result.cues = refined.cues;
    // Cache even if the deck moved on: the work is done and the next load is instant
    analysis::store(app, &info, &result, true);
    if !current() {
        return Ok(());
    }
    app.send(AudioCommand::SetBeatGrid { deck, grid: result.beatgrid.clone() });
    analysis::publish_grid(app, deck, &mut msg, &result.beatgrid, &result.cues);
    Ok(())
}
