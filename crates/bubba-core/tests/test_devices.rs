use bubba_core::AudioEngine;

#[test]
fn test_print_audio_devices() {
    let devices = AudioEngine::list_output_devices();
    println!("\n=== AUDIO OUTPUT DEVICES DETECTED ===");
    for (i, d) in devices.iter().enumerate() {
        println!("[{}] {}", i, d);
    }
}
