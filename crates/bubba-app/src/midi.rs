use std::sync::Arc;
use std::thread;
use std::time::Duration;
use crossbeam_channel::unbounded;

use bubba_midi::{is_supported_controller, ActiveMidiDevice, DjAction, MidiManager};

use crate::app::{App, MidiStatus};
use crate::commands::handle_dj_action;
use crate::config::log_line;

/// Keeps the DDJ-FLX4 connected: connects on startup, detects unplug and re-plug.
pub fn start(app: Arc<App>) {
    let (tx, rx) = unbounded::<DjAction>();

    let app_actions = Arc::clone(&app);
    thread::Builder::new()
        .name("midi-actions".into())
        .spawn(move || {
            while let Ok(action) = rx.recv() {
                handle_dj_action(&app_actions, action);
            }
        })
        .ok();

    thread::Builder::new()
        .name("midi-hotplug".into())
        .spawn(move || {
            let mut active: Option<ActiveMidiDevice> = None;
            loop {
                let ports = MidiManager::new("bubba-dj-pro-scan").map(|m| m.list_ports()).unwrap_or_default();

                match &active {
                    Some(dev) => {
                        if !ports.iter().any(|(_, n)| *n == dev.name) {
                            log_line(&format!("Controlador desconectado: {}", dev.name));
                            app.toast("warn", format!("Controlador desconectado: {}", dev.name));
                            active = None;
                            set_status(&app, MidiStatus::default());
                        }
                    }
                    None => {
                        if let Some((idx, name)) = ports.iter().find(|(_, n)| is_supported_controller(n)).cloned() {
                            match MidiManager::new("bubba-dj-pro").and_then(|m| m.connect_port(idx, tx.clone())) {
                                Ok(dev) => {
                                    log_line(&format!("Controlador conectado: {}", dev.name));
                                    app.toast("ok", format!("{} conectada", name));
                                    set_status(&app, MidiStatus { connected: true, name: Some(dev.name.clone()) });
                                    active = Some(dev);
                                }
                                Err(e) => log_line(&format!("No se pudo abrir {}: {}", name, e)),
                            }
                        }
                    }
                }
                thread::sleep(Duration::from_secs(2));
            }
        })
        .ok();
}

fn set_status(app: &App, status: MidiStatus) {
    if let Ok(mut m) = app.midi.write() {
        *m = status;
    }
    app.hub.broadcast(&app.midi_msg());
}
