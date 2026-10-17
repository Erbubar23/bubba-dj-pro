//! Compares the quick DSP grid with the refined (neural + DSP) grid.
//!
//!   cargo run --release -p bubba-analyzer --example grid_eval -- "C:\Music\track.mp3" ...

use bubba_analyzer::{Analyzer, AnalyzerConfig, AudioDecoder, BeatTracker};

fn main() -> anyhow::Result<()> {
    let cfg = AnalyzerConfig::default();
    println!("{:<44} {:>8} {:>9} {:>8} {:>9} {:>9} {:>6} {:>7}  cues", "pista", "DSP", "DSP beat", "final", "1er beat", "compás 1", "conf", "ms");
    for path in std::env::args().skip(1) {
        let audio = match AudioDecoder::decode_file(&path) {
            Ok(a) => a,
            Err(e) => {
                println!("{}: {:#}", path, e);
                continue;
            }
        };
        let mono = audio.to_mono();
        let quick = BeatTracker::analyze_range(&mono, audio.sample_rate, cfg.bpm_range);
        let refined = Analyzer::refine_beatgrid(&mono, audio.sample_rate, &cfg);
        let g = &refined.beatgrid;
        let name = std::path::Path::new(&path).file_name().unwrap().to_string_lossy().to_string();
        let name: String = name.chars().take(44).collect();
        let cues: Vec<String> = refined.cues.iter().map(|c| format!("{:?}@{:.1}", c.kind, c.seconds)).collect();
        println!(
            "{:<44} {:>8.2} {:>9.3} {:>8.2} {:>9.3} {:>9.3} {:>6.2} {:>7}  {:?} {}",
            name, quick.bpm, quick.cue_seconds(), g.bpm, g.cue_seconds(), g.first_downbeat_seconds, g.confidence, refined.time_ms, g.source, cues.join(" ")
        );
    }
    Ok(())
}
