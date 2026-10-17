use midir::{MidiInput, MidiInputConnection};
use crossbeam_channel::Sender;
use anyhow::{Context, Result};
use crate::events::DjAction;
use crate::flx4::Flx4Parser;

pub struct MidiManager {
    midi_in: MidiInput,
}

pub struct ActiveMidiDevice {
    pub name: String,
    _connection: MidiInputConnection<()>,
}

impl MidiManager {
    pub fn new(client_name: &str) -> Result<Self> {
        let midi_in = MidiInput::new(client_name)
            .context("Failed to initialize MIDI subsystem")?;
        Ok(Self { midi_in })
    }

    /// Lists all detected MIDI input ports
    pub fn list_ports(&self) -> Vec<(usize, String)> {
        let mut result = Vec::new();
        for (i, port) in self.midi_in.ports().iter().enumerate() {
            let port_name = self.midi_in.port_name(port).unwrap_or_else(|_| "Unknown Port".to_string());
            result.push((i, port_name));
        }
        result
    }

    /// Auto-connects to a Pioneer DDJ-FLX4 or first available DJ controller
    pub fn auto_connect(self, action_tx: Sender<DjAction>) -> Result<Option<ActiveMidiDevice>> {
        let ports = self.midi_in.ports();
        
        let target_port_index = ports
            .iter()
            .position(|p| crate::is_supported_controller(&self.midi_in.port_name(p).unwrap_or_default()));

        if let Some(idx) = target_port_index {
            self.connect_port(idx, action_tx).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Connects to a specific port by index
    pub fn connect_port(self, port_index: usize, action_tx: Sender<DjAction>) -> Result<ActiveMidiDevice> {
        let ports = self.midi_in.ports();
        let port = ports.get(port_index).context("Invalid MIDI port index")?;
        let name = self.midi_in.port_name(port).unwrap_or_else(|_| "Unknown".to_string());

        let connection = self.midi_in.connect(
            port,
            "bubba-flx4-input",
            move |_timestamp, message, _| {
                if let Some(action) = Flx4Parser::parse(message) {
                    let _ = action_tx.send(action);
                }
            },
            (),
        ).map_err(|e| anyhow::anyhow!("Failed to connect to MIDI port: {}", e))?;

        Ok(ActiveMidiDevice {
            name,
            _connection: connection,
        })
    }
}
