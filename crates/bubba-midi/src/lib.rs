pub mod events;
pub mod flx4;
pub mod manager;

pub use events::DjAction;
pub use flx4::Flx4Parser;
pub use manager::{MidiManager, ActiveMidiDevice};

/// Controllers that share the DDJ-FLX4 / DDJ-400 MIDI layout
pub fn is_supported_controller(port_name: &str) -> bool {
    let name = port_name.to_uppercase();
    name.contains("FLX4") || name.contains("DDJ") || name.contains("PIONEER")
}
