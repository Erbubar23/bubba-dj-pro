import math
import struct
import wave

sample_rate = 44100
bpm = 128.0
beat_interval = 60.0 / bpm
duration = 15.0 # 15 seconds
total_samples = int(sample_rate * duration)

audio_l = [0.0] * total_samples
audio_r = [0.0] * total_samples

# 1. Intro (0.0s to 1.8s): Ambient synth sweep & noise (NO kick)
for i in range(int(sample_rate * 1.8)):
    t = i / sample_rate
    # Sweep from 1000 Hz to 3500 Hz
    freq = 1000.0 + 1500.0 * (t / 1.8)
    sweep = math.sin(2.0 * math.pi * freq * t) * 0.4
    audio_l[i] += sweep
    audio_r[i] += sweep

# 2. Kick drum and rhythm starts exactly at 2.0s (128.0 BPM)
start_offset = 2.0
beat_time = start_offset
beat_count = 0

while beat_time < duration:
    start_sample = int(beat_time * sample_rate)
    kick_len = int(sample_rate * 0.08) # 80ms punchy kick
    
    # Kick drum (40Hz to 80Hz pitch drop)
    for k in range(kick_len):
        idx = start_sample + k
        if idx < total_samples:
            t = k / sample_rate
            env = (1.0 - (k / kick_len)) ** 2
            inst_freq = 120.0 * (1.0 - (k / kick_len)) + 48.0
            kick = math.sin(2.0 * math.pi * inst_freq * t) * env * 0.95
            audio_l[idx] += kick
            audio_r[idx] += kick
            
    # Off-beat Hi-Hat (on the "&" between beats)
    hat_sample = start_sample + int((beat_interval * 0.5) * sample_rate)
    hat_len = int(sample_rate * 0.04)
    for h in range(hat_len):
        idx = hat_sample + h
        if idx < total_samples:
            env = (1.0 - (h / hat_len)) ** 3
            # White noise burst for hi-hat
            import random
            noise = (random.random() * 2.0 - 1.0) * env * 0.25
            audio_l[idx] += noise * 0.8
            audio_r[idx] += noise * 1.0
            
    beat_time += beat_interval
    beat_count += 1

# Write WAV file
output_path = "test_track_128bpm.wav"
with wave.open(output_path, "wb") as wav:
    wav.setnchannels(2)
    wav.setsampwidth(2) # 16-bit
    wav.setframerate(sample_rate)
    
    frames = bytearray()
    for l, r in zip(audio_l, audio_r):
        # Clamp to -1.0 .. 1.0
        val_l = max(-1.0, min(1.0, l))
        val_r = max(-1.0, min(1.0, r))
        int_l = int(val_l * 32767.0)
        int_r = int(val_r * 32767.0)
        frames.extend(struct.pack("<hh", int_l, int_r))
        
    wav.writeframes(frames)

print(f"Generated test track: {output_path} (128 BPM, kick starts at 2.0s)")
