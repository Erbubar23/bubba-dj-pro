// Release builds run as a windowed app (no console window behind the UI)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod app;
mod commands;
mod config;
mod library;
mod loader;
mod midi;
mod server;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use crossbeam_channel::{unbounded, RecvTimeoutError};
use serde_json::{json, Value};

use bubba_core::{AudioCommand, AudioEngine};

use app::{App, Hub, MidiStatus, RoutingRequest};
use config::{log_line, Config};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Close the app when the window has been gone for this long
const WINDOW_GONE_GRACE: Duration = Duration::from_secs(6);

fn main() {
    config::rotate_log();
    log_line(&format!("==== Bubba DJ Pro v{} ====", VERSION));

    // Single instance: if Bubba is already running just bring up another window
    if server::is_bubba_running(8080) {
        log_line("Ya hay una instancia en ejecución; abriendo su ventana.");
        launch_desktop_window("http://127.0.0.1:8080");
        return;
    }

    let mut config = Config::load();

    // Neural beat model: extracted under %APPDATA%\BubbaDJPro\models, loaded ahead of the first track
    bubba_analyzer::beatgrid::neural::set_model_dir(&config::data_dir());
    std::thread::spawn(|| {
        if let Err(e) = bubba_analyzer::beatgrid::neural::warm_up() {
            log_line(&format!("Modelo de beats no disponible (se usa solo DSP): {:#}", e));
        }
    });

    // 1. Audio engine
    let mut engine = AudioEngine::new();
    let routing_error = apply_routing(&mut engine, config.master_device.clone(), config.headphone_device.clone(), true).err();
    if routing_error.is_none() && config.master_device.is_none() {
        config.master_device = Some(engine.info.master_device.clone());
    }

    // 2. Shared application state
    let (routing_tx, routing_rx) = unbounded::<RoutingRequest>();
    let exe_dir = env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let app = Arc::new(App {
        cmd_tx: engine.command_sender(),
        state: Arc::clone(&engine.state),
        hub: Hub::default(),
        token: app::make_token(),
        exe_dir: exe_dir.clone(),
        routing_tx,
        routing_msg: RwLock::new(Value::Null),
        midi: RwLock::new(MidiStatus::default()),
        tracks: RwLock::new([None, None]),
        load_generation: [AtomicU64::new(0), AtomicU64::new(0)],
        config: Mutex::new(config),
        analysis_cache: Mutex::new(config::load_analysis_cache()),
    });
    publish_routing(&app, &engine, routing_error.map(|e| format!("{:#}", e)));
    let sync_mode = app.config.lock().map(|c| c.sync_mode).unwrap_or_default();
    app.send(AudioCommand::SetSyncMode { mode: sync_mode });

    // 3. Servers (HTTP UI + WebSocket telemetry)
    let ws_listener = match server::bind_port(8081) {
        Ok(l) => l,
        Err(e) => fatal(&format!("No se pudo abrir el puerto del WebSocket: {}", e)),
    };
    let ws_port = ws_listener.local_addr().map(|a| a.port()).unwrap_or(8081);
    server::start_ws(Arc::clone(&app), ws_listener);

    let http_listener = match server::bind_port(8080) {
        Ok(l) => l,
        Err(e) => fatal(&format!("No se pudo abrir el puerto de la interfaz: {}", e)),
    };
    let http_port = server::start_http(Arc::clone(&app), http_listener, ws_port);
    log_line(&format!("Interfaz: http://127.0.0.1:{}  |  WebSocket: {}", http_port, ws_port));

    // 4. DDJ-FLX4 (with hot-plug)
    midi::start(Arc::clone(&app));

    // 5. Tracks passed on the command line (e.g. dragged onto the .exe)
    let args: Vec<PathBuf> = env::args().skip(1).map(PathBuf::from).filter(|p| p.is_file()).collect();
    for (deck, path) in args.into_iter().take(2).enumerate() {
        loader::load_track(&app, deck, path, true);
    }

    // 6. Native app window (BUBBA_NO_WINDOW=1 skips it, for testing in a regular browser)
    if env::var_os("BUBBA_NO_WINDOW").is_none() {
        launch_desktop_window(&format!("http://127.0.0.1:{}", http_port));
    }

    // 7. Main loop: owns the audio streams (re-routing, device recovery) and the window watchdog
    let mut window_gone_since: Option<Instant> = None;
    let mut last_recovery = Instant::now();
    loop {
        match app_routing_request(&routing_rx) {
            Some(RoutingRequest::Set { master, headphone }) => {
                let err = apply_routing(&mut engine, master, headphone.clone(), false).err();
                if err.is_none() {
                    if let Ok(mut cfg) = app.config.lock() {
                        cfg.master_device = Some(engine.info.master_device.clone());
                        cfg.headphone_device = headphone;
                        cfg.save();
                    }
                    app.toast("ok", format!("Audio: {} a {} Hz", engine.info.master_device, engine.info.sample_rate));
                }
                publish_routing(&app, &engine, err.map(|e| format!("{:#}", e)));
            }
            None => {}
        }

        // A device vanished (unplugged / driver reset): try to bring audio back
        if engine.has_failed() && last_recovery.elapsed() > Duration::from_secs(2) {
            last_recovery = Instant::now();
            log_line("Fallo en el dispositivo de audio, reintentando...");
            let (m, h) = app.config.lock().map(|c| (c.master_device.clone(), c.headphone_device.clone())).unwrap_or_default();
            let err = apply_routing(&mut engine, m, h, true).err();
            if err.is_none() {
                app.toast("warn", format!("Audio recuperado en {}", engine.info.master_device));
            }
            publish_routing(&app, &engine, err.map(|e| format!("{:#}", e)));
        }

        // Closing the window closes the app
        if app.hub.ever_connected.load(Ordering::Relaxed) && app.hub.count() == 0 {
            let since = *window_gone_since.get_or_insert_with(Instant::now);
            if since.elapsed() > WINDOW_GONE_GRACE {
                log_line("Ventana cerrada, saliendo.");
                if let Ok(cache) = app.analysis_cache.lock() {
                    config::save_analysis_cache(&cache);
                }
                engine.stop();
                std::process::exit(0);
            }
        } else {
            window_gone_since = None;
        }
    }
}

fn app_routing_request(rx: &crossbeam_channel::Receiver<RoutingRequest>) -> Option<RoutingRequest> {
    match rx.recv_timeout(Duration::from_millis(500)) {
        Ok(r) => Some(r),
        Err(RecvTimeoutError::Timeout) => None,
        Err(RecvTimeoutError::Disconnected) => {
            std::thread::sleep(Duration::from_millis(500));
            None
        }
    }
}

/// Opens the requested devices; with `fallback` tries automatic/default routing if they fail
/// `headphone == Some("")` means "no separate headphone device".
fn apply_routing(engine: &mut AudioEngine, master: Option<String>, headphone: Option<String>, fallback: bool) -> anyhow::Result<()> {
    let (auto_master, auto_hp) = if fallback { AudioEngine::auto_routing() } else { (None, None) };
    let master = master.filter(|m| !m.is_empty()).or(auto_master);
    let headphone = match headphone {
        Some(h) if h.is_empty() => None,
        Some(h) => Some(h),
        None => auto_hp,
    };

    match engine.set_routing(master.as_deref(), headphone.as_deref()) {
        Ok(()) => {
            log_line(&format!(
                "Audio OK -> Master: {} ({} Hz, {} canales) | Audífonos: {}",
                engine.info.master_device,
                engine.info.sample_rate,
                engine.info.master_channels,
                engine.info.headphone_device.as_deref().unwrap_or("canales 3-4 / ninguno")
            ));
            Ok(())
        }
        Err(e) if fallback => {
            log_line(&format!("No se pudo abrir {:?}: {:#}. Probando salida predeterminada.", master, e));
            engine.set_routing(None, None).map_err(|e2| {
                log_line(&format!("Sin audio: {:#}", e2));
                e2
            })
        }
        Err(e) => {
            log_line(&format!("Error de enrutamiento: {:#}", e));
            // Keep sound going on the default device
            let _ = engine.set_routing(None, None);
            Err(e)
        }
    }
}

fn publish_routing(app: &App, engine: &AudioEngine, error: Option<String>) {
    let msg = json!({
        "type": "audio_routing",
        "master_device": engine.info.master_device,
        "headphone_device": engine.info.headphone_device,
        "sample_rate": engine.info.sample_rate,
        "master_channels": engine.info.master_channels,
        "four_channel_cue": engine.info.four_channel_cue,
        "devices": AudioEngine::list_output_devices(),
        "error": error,
        "version": VERSION,
    });
    if let Ok(mut r) = app.routing_msg.write() {
        *r = msg.clone();
    }
    app.hub.broadcast(&msg);
}

fn fatal(message: &str) -> ! {
    log_line(message);
    let script = format!(
        "Add-Type -AssemblyName PresentationFramework; [System.Windows.MessageBox]::Show('{}','Bubba DJ Pro','OK','Error')",
        message.replace('\'', "''")
    );
    let _ = Command::new("powershell").args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script]).status();
    std::process::exit(1);
}

/// Launches the UI as a standalone app window (Edge / Chrome app mode)
fn launch_desktop_window(url: &str) {
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
    ];
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join(r"Google\Chrome\Application\chrome.exe"));
    }

    let profile = config::data_dir().join("window-profile");
    for path in candidates.iter().filter(|p| p.exists()) {
        let res = Command::new(path)
            .arg(format!("--app={}", url))
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--start-maximized")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-features=Translate,msEdgeTranslate")
            .arg("--autoplay-policy=no-user-gesture-required")
            .spawn();
        if res.is_ok() {
            return;
        }
    }
    let _ = Command::new("cmd").args(["/c", "start", "", url]).spawn();
}

