import math
import struct
import wave
import random

sample_rate = 44100
bpm = 126.0
beat_interval = 60.0 / bpm
duration = 15.0 # 15 seconds
total_samples = int(sample_rate * duration)

audio_l = [0.0] * total_samples
audio_r = [0.0] * total_samples

# Track 2: 126 BPM Deep House groove with center lead vocal/melody
beat_time = 0.0
while beat_time < duration:
    start_sample = int(beat_time * sample_rate)
    kick_len = int(sample_rate * 0.09)
    
    # 1. Kick Drum (Sub-bass transient: 50Hz)
    for k in range(kick_len):
        idx = start_sample + k
        if idx < total_samples:
            t = k / sample_rate
            env = (1.0 - (k / kick_len)) ** 2
            kick = math.sin(2.0 * math.pi * 55.0 * t) * env * 0.9
            audio_l[idx] += kick
            audio_r[idx] += kick

    # 2. Lead Vocal / Synth Melody in Center Channel (M = L+R, perfect for Stems testing!)
    lead_len = int(sample_rate * (beat_interval * 0.8))
    for m in range(lead_len):
        idx = start_sample + m
        if idx < total_samples:
            t = m / sample_rate
            # 880 Hz / 1100 Hz alternating melody note
            freq = 880.0 if (int(beat_time / beat_interval) % 2 == 0) else 1174.0
            env = math.sin(math.pi * (m / lead_len))
            lead = math.sin(2.0 * math.pi * freq * t) * env * 0.35
            # Center panned
            audio_l[idx] += lead
            audio_r[idx] += lead

    # 3. Off-beat shaker / hat (Stereo spread)
    hat_sample = start_sample + int((beat_interval * 0.5) * sample_rate)
    hat_len = int(sample_rate * 0.05)
    for h in range(hat_len):
        idx = hat_sample + h
        if idx < total_samples:
            env = (1.0 - (h / hat_len)) ** 2.5
            noise = (random.random() * 2.0 - 1.0) * env * 0.2
            audio_l[idx] += noise * 0.6
            audio_r[idx] += noise * 0.9

    beat_time += beat_interval

output_path = "test_track_126bpm_vocal.wav"
with wave.open(output_path, "wb") as wav:
    wav.setnchannels(2)
    wav.setsampwidth(2)
    wav.setframerate(sample_rate)
    
    frames = bytearray()
    for l, r in zip(audio_l, audio_r):
        val_l = max(-1.0, min(1.0, l))
        val_r = max(-1.0, min(1.0, r))
        int_l = int(val_l * 32767.0)
        int_r = int(val_r * 32767.0)
        frames.extend(struct.pack("<hh", int_l, int_r))
        
    wav.writeframes(frames)

print(f"Generated track: {output_path} (126 BPM, vocal/melody center)")
