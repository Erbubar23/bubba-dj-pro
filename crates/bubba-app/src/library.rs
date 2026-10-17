use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde_json::{json, Value};

use crate::config::CachedAnalysis;

pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "flac", "aif", "aiff", "ogg", "oga", "m4a", "mp4", "aac", "alac", "caf", "mkv", "webm"];

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Quick access locations shown in the library sidebar
pub fn roots(exe_dir: Option<&Path>) -> Value {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        for (name, sub, icon) in [
            ("Música", "Music", "music"),
            ("Descargas", "Downloads", "download"),
            ("Escritorio", "Desktop", "desktop"),
            ("Documentos", "Documents", "folder"),
            ("OneDrive · Música", "OneDrive\\Music", "cloud"),
            ("OneDrive · Música", "OneDrive\\Música", "cloud"),
        ] {
            let p = home.join(sub);
            if p.is_dir() {
                out.push(json!({ "name": name, "path": p.to_string_lossy(), "icon": icon }));
            }
        }
    }
    if let Some(dir) = exe_dir {
        let has_audio = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().any(|e| is_audio_file(&e.path())))
            .unwrap_or(false);
        if has_audio {
            out.push(json!({ "name": "Pistas de prueba", "path": dir.to_string_lossy(), "icon": "flask" }));
        }
    }
    for letter in b'C'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        if Path::new(&drive).exists() {
            out.push(json!({ "name": format!("Unidad {}:", letter as char), "path": drive, "icon": "drive" }));
        }
    }
    Value::Array(out)
}

/// Lists sub-folders and audio files of a folder
pub fn browse(path: &str, cache: &HashMap<String, CachedAnalysis>) -> Result<Value, String> {
    let dir = std::path::absolute(PathBuf::from(path)).unwrap_or_else(|_| PathBuf::from(path));
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("No se puede abrir la carpeta: {}", e))?;

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.starts_with('$') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            dirs.push((name.to_lowercase(), json!({ "name": name, "path": p.to_string_lossy() })));
        } else if is_audio_file(&p) {
            let key = p.to_string_lossy().to_string();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_uppercase();
            let (artist_guess, title_guess) = split_artist_title(&p);
            let mut item = json!({
                "name": name,
                "path": key,
                "ext": ext,
                "size": meta.len(),
                "title": title_guess,
                "artist": artist_guess,
            });
            if let Some(c) = cache.get(&key) {
                item["title"] = json!(c.title);
                if c.artist.is_some() {
                    item["artist"] = json!(c.artist);
                }
                item["bpm"] = json!(c.bpm);
                item["camelot"] = json!(c.camelot);
                item["key"] = json!(c.key);
                item["duration"] = json!(c.duration);
            }
            files.push((name.to_lowercase(), item));
        }
    }
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    files.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(json!({
        "path": dir.to_string_lossy(),
        "parent": dir.parent().map(|p| p.to_string_lossy().to_string()),
        "dirs": dirs.into_iter().map(|d| d.1).collect::<Vec<_>>(),
        "files": files.into_iter().map(|f| f.1).collect::<Vec<_>>(),
    }))
}

/// "Artist - Title.mp3" -> (Some("Artist"), "Title")
pub fn split_artist_title(path: &Path) -> (Option<String>, String) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let cleaned = stem.replace('_', " ");
    if let Some((a, t)) = cleaned.split_once(" - ") {
        let (a, t) = (a.trim(), t.trim());
        if !a.is_empty() && !t.is_empty() {
            return (Some(a.to_string()), t.to_string());
        }
    }
    (None, cleaned.trim().to_string())
}

/// Decodes %XX sequences (UTF-8) from an encodeURIComponent string
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Keeps only a safe file name component
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control() { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() { "pista.wav".to_string() } else { trimmed }
}

pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_and_base64() {
        assert_eq!(percent_decode("Canci%C3%B3n%20(1).mp3"), "Canción (1).mp3");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(sanitize_file_name("..\\..\\evil:name?.mp3"), "evil_name_.mp3");
    }

    #[test]
    fn artist_title_split() {
        let (a, t) = split_artist_title(Path::new("C:\\x\\Daft Punk - One More Time.mp3"));
        assert_eq!(a.as_deref(), Some("Daft Punk"));
        assert_eq!(t, "One More Time");
    }
}
