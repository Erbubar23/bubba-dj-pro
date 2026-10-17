use bubba_midi::MidiManager;

/// Hardware test: needs a DDJ-FLX4 plugged in. Run with `cargo test -- --ignored`.
#[test]
#[ignore]
fn test_flx4_detection() {
    let midi_manager = MidiManager::new("bubba-test").expect("Failed to init MIDI manager");
    let ports = midi_manager.list_ports();
    println!("\n=== MIDI PORTS DETECTED ===");
    for (i, name) in &ports {
        println!("[{}] {}", i, name);
    }

    let flx4_found = ports.iter().any(|(_, name)| bubba_midi::is_supported_controller(name));
    println!("FLX4 Found in ports: {}", flx4_found);
    assert!(flx4_found, "Pioneer DDJ-FLX4 must be in the detected MIDI ports!");
}
