use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use anyhow::{Context, Result};
use bubba_analyzer::DecodedAudio;
use crate::commands::AudioCommand;
use crate::mixer::{Mixer, MixerSnapshot};

/// Description of the active audio routing, sent to the UI
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RoutingInfo {
    pub master_device: String,
    pub headphone_device: Option<String>,
    pub sample_rate: u32,
    pub master_channels: u16,
    /// Headphones on channels 3-4 of the master device (e.g. DDJ-FLX4 internal sound card)
    pub four_channel_cue: bool,
    pub error: Option<String>,
}

/// Headphone frames travelling from the master callback to the headphone device callback
type CueQueue = Arc<Mutex<VecDeque<(f32, f32)>>>;

pub struct AudioEngine {
    mixer: Arc<Mutex<Mixer>>,
    cmd_tx: Sender<AudioCommand>,
    cmd_rx: Receiver<AudioCommand>,
    trash_tx: Sender<DecodedAudio>,
    pub state: Arc<RwLock<MixerSnapshot>>,
    master_stream: Option<cpal::Stream>,
    hp_stream: Option<cpal::Stream>,
    pub info: RoutingInfo,
    failed: Arc<AtomicBool>,
}

impl AudioEngine {
    /// Creates the engine without opening any device (see `set_routing`)
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (trash_tx, trash_rx) = crossbeam_channel::unbounded::<DecodedAudio>();
        // Large track buffers are freed here, never in the real-time callback
        std::thread::Builder::new()
            .name("audio-trash".into())
            .spawn(move || while trash_rx.recv().is_ok() {})
            .ok();

        let mixer = Mixer::new(48000.0);
        let state = Arc::new(RwLock::new(mixer.get_snapshot()));
        Self {
            mixer: Arc::new(Mutex::new(mixer)),
            cmd_tx,
            cmd_rx,
            trash_tx,
            state,
            master_stream: None,
            hp_stream: None,
            info: RoutingInfo::default(),
            failed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn command_sender(&self) -> Sender<AudioCommand> {
        self.cmd_tx.clone()
    }

    /// True when a stream reported an error (device unplugged, driver reset...)
    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// Lists all available physical and virtual audio output devices
    pub fn list_output_devices() -> Vec<String> {
        let host = cpal::default_host();
        let mut devices = Vec::new();
        if let Ok(dev_iter) = host.output_devices() {
            for dev in dev_iter {
                if let Ok(name) = dev.name() {
                    devices.push(name);
                }
            }
        }
        devices
    }

    /// Picks a sensible default: pro interface for master, onboard card for headphones
    pub fn auto_routing() -> (Option<String>, Option<String>) {
        let devices = Self::list_output_devices();
        let master = devices
            .iter()
            .find(|d| {
                let upper = d.to_uppercase();
                upper.contains("BEHRINGER") || upper.contains("FOCUSRITE") || upper.contains("FLX4") || upper.contains("DDJ")
            })
            .cloned();
        let hp = devices
            .iter()
            .find(|d| {
                let upper = d.to_uppercase();
                (upper.contains("REALTEK") || upper.contains("HIGH DEFINITION") || upper.contains("HEADPHONE") || upper.contains("AURICULAR"))
                    && master.as_ref() != Some(*d)
            })
            .cloned();
        (master, hp)
    }

    fn find_device(host: &cpal::Host, name: &str) -> Option<cpal::Device> {
        let devices: Vec<cpal::Device> = host.output_devices().ok()?.collect();
        let exact = devices.iter().position(|d| d.name().map(|n| n == name).unwrap_or(false));
        let idx = exact.or_else(|| devices.iter().position(|d| d.name().map(|n| n.contains(name)).unwrap_or(false)))?;
        devices.into_iter().nth(idx)
    }

    fn f32_config(device: &cpal::Device) -> Result<cpal::StreamConfig> {
        let default = device.default_output_config().context("No se pudo leer la configuración del dispositivo")?;
        if default.sample_format() == cpal::SampleFormat::F32 {
            return Ok(default.into());
        }
        let rate = default.sample_rate();
        let supported = device
            .supported_output_configs()?
            .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
            .find(|c| c.min_sample_rate() <= rate && c.max_sample_rate() >= rate)
            .context("El dispositivo no soporta audio en formato float")?;
        Ok(supported.with_sample_rate(rate).into())
    }

    /// (Re)opens the output streams. Mixer state (tracks, positions, EQ...) is preserved.
    pub fn set_routing(&mut self, master_name: Option<&str>, headphone_name: Option<&str>) -> Result<()> {
        // Stop the current streams first so only one callback touches the mixer
        self.master_stream = None;
        self.hp_stream = None;
        self.failed.store(false, Ordering::Relaxed);

        let host = cpal::default_host();
        let master_dev = match master_name {
            Some(n) => Self::find_device(&host, n).or_else(|| host.default_output_device()),
            None => host.default_output_device(),
        }
        .context("No se encontró ninguna salida de audio")?;

        let master_dev_name = master_dev.name().unwrap_or_else(|_| "Salida predeterminada".to_string());
        let master_config = Self::f32_config(&master_dev)?;
        let sample_rate = master_config.sample_rate.0;
        let master_channels = master_config.channels as usize;

        if let Ok(mut mix) = self.mixer.lock() {
            if (mix.sample_rate - sample_rate as f32).abs() > 0.5 {
                mix.set_sample_rate(sample_rate as f32);
            }
        }

        // Optional independent headphone device
        let mut hp_setup = None;
        if let Some(hp_name) = headphone_name {
            if let Some(hp_dev) = Self::find_device(&host, hp_name) {
                let hp_dev_name = hp_dev.name().unwrap_or_default();
                if hp_dev_name != master_dev_name {
                    match Self::f32_config(&hp_dev) {
                        Ok(cfg) => hp_setup = Some((hp_dev, hp_dev_name, cfg)),
                        Err(e) => log::warn!("Audífonos no disponibles: {}", e),
                    }
                }
            }
        }

        let cue_queue: Option<CueQueue> = hp_setup
            .as_ref()
            .map(|_| Arc::new(Mutex::new(VecDeque::with_capacity(sample_rate as usize))));

        // ---------- Master stream ----------
        let mixer = Arc::clone(&self.mixer);
        let cmd_rx = self.cmd_rx.clone();
        let state = Arc::clone(&self.state);
        let trash = self.trash_tx.clone();
        let queue_master = cue_queue.clone();
        let queue_cap = sample_rate as usize / 2;
        let mut pending_cue: Vec<(f32, f32)> = Vec::with_capacity(32768);
        let failed = Arc::clone(&self.failed);

        let master_stream = master_dev.build_output_stream(
            &master_config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let Ok(mut mix) = mixer.lock() else {
                    data.fill(0.0);
                    return;
                };
                while let Ok(cmd) = cmd_rx.try_recv() {
                    Self::apply_command(&mut mix, cmd, &trash);
                }

                for frame in data.chunks_mut(master_channels.max(1)) {
                    let (l, r) = mix.process_master();
                    match frame.len() {
                        0 => {}
                        1 => frame[0] = (l + r) * 0.5,
                        n => {
                            frame[0] = l;
                            frame[1] = r;
                            if n >= 4 {
                                // 4-channel device (e.g. FLX4 internal DAC): Ch 3-4 = headphones
                                let (hp_l, hp_r) = mix.process_headphones();
                                frame[2] = hp_l;
                                frame[3] = hp_r;
                                for s in frame.iter_mut().skip(4) {
                                    *s = 0.0;
                                }
                            } else {
                                for s in frame.iter_mut().skip(2) {
                                    *s = 0.0;
                                }
                            }
                        }
                    }
                    if queue_master.is_some() && pending_cue.len() < pending_cue.capacity() {
                        pending_cue.push(mix.process_headphones());
                    }
                }

                if let Ok(mut snap) = state.try_write() {
                    *snap = mix.get_snapshot();
                }
                drop(mix);

                if let Some(ref q) = queue_master {
                    if let Ok(mut q) = q.lock() {
                        let space = queue_cap.saturating_sub(q.len());
                        q.extend(pending_cue.iter().take(space).copied());
                    }
                    pending_cue.clear();
                }
            },
            move |err| {
                eprintln!("Error en salida master: {}", err);
                failed.store(true, Ordering::Relaxed);
            },
            None,
        )?;
        master_stream.play().context("No se pudo iniciar la salida master")?;

        // ---------- Headphone stream (separate sound card) ----------
        let mut hp_name_active = None;
        if let (Some((hp_dev, hp_dev_name, hp_config)), Some(queue)) = (hp_setup, cue_queue) {
            let hp_channels = hp_config.channels as usize;
            let mut resampler = CueResampler::new(sample_rate as f64, hp_config.sample_rate.0 as f64);
            let failed = Arc::clone(&self.failed);
            let stream = hp_dev.build_output_stream(
                &hp_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let Ok(mut q) = queue.lock() else {
                        data.fill(0.0);
                        return;
                    };
                    resampler.begin_block(&mut q);
                    for frame in data.chunks_mut(hp_channels.max(1)) {
                        let (l, r) = resampler.next(&mut q);
                        match frame.len() {
                            0 => {}
                            1 => frame[0] = (l + r) * 0.5,
                            _ => {
                                frame[0] = l;
                                frame[1] = r;
                                for s in frame.iter_mut().skip(2) {
                                    *s = 0.0;
                                }
                            }
                        }
                    }
                },
                move |err| {
                    eprintln!("Error en salida de audífonos: {}", err);
                    failed.store(true, Ordering::Relaxed);
                },
                None,
            );
            match stream {
                Ok(s) => {
                    if s.play().is_ok() {
                        self.hp_stream = Some(s);
                        hp_name_active = Some(hp_dev_name);
                    }
                }
                Err(e) => log::warn!("No se pudo abrir la salida de audífonos: {}", e),
            }
        }

        self.master_stream = Some(master_stream);
        self.info = RoutingInfo {
            master_device: master_dev_name,
            headphone_device: hp_name_active,
            sample_rate,
            master_channels: master_channels as u16,
            four_channel_cue: master_channels >= 4,
            error: None,
        };
        Ok(())
    }

    /// Closes all streams (used after a fatal error)
    pub fn stop(&mut self) {
        self.master_stream = None;
        self.hp_stream = None;
    }


    fn apply_command(mixer: &mut Mixer, cmd: AudioCommand, trash: &Sender<DecodedAudio>) {
        match cmd {
            AudioCommand::LoadTrack { deck, audio, beatgrid, key } => {
                // Loading a deck breaks any sync relation pointing at it
                mixer.release_sync_to(deck.min(1));
                if let Some(old) = mixer.deck_mut(deck).load_track(audio, beatgrid, key) {
                    let _ = trash.send(old);
                }
            }
            AudioCommand::PlayPause { deck } => mixer.deck_mut(deck).toggle_play_pause(),
            AudioCommand::Cue { deck, pressed } => mixer.deck_mut(deck).cue(pressed),
            AudioCommand::Jog { deck, delta } => mixer.deck_mut(deck).jog_rotate(delta),
            AudioCommand::JogTouch { deck, touched } => mixer.deck_mut(deck).jog_touch(touched),
            AudioCommand::Scratch { deck, delta } => mixer.deck_mut(deck).scratch(delta),
            AudioCommand::Search { deck, delta } => mixer.deck_mut(deck).search(delta),
            AudioCommand::SetTempo { deck, value } => mixer.set_tempo(deck.min(1), value),
            AudioCommand::SetPitchRange { deck, range } => mixer.deck_mut(deck).set_pitch_range(range),
            AudioCommand::SetVolume { deck, volume } => mixer.deck_mut(deck).volume = volume.clamp(0.0, 1.0),
            AudioCommand::SetTrim { deck, value } => mixer.deck_mut(deck).set_trim(value),
            AudioCommand::SetEq { deck, band, value } => mixer.deck_mut(deck).set_eq(band, value),
            AudioCommand::SetStemGain { deck, stem, gain } => mixer.deck_mut(deck).set_stem_gain(stem, gain),
            AudioCommand::Sync { deck } => mixer.trigger_sync(deck),
            AudioCommand::SetCrossfader { value } => mixer.crossfader = value.clamp(-1.0, 1.0),
            AudioCommand::SetMasterVolume { value } => mixer.set_master_volume(value),
            AudioCommand::HotCue { deck, pad_index, pressed } => mixer.deck_mut(deck).trigger_hot_cue(pad_index, pressed),
            AudioCommand::DeleteHotCue { deck, pad_index } => mixer.deck_mut(deck).delete_hot_cue(pad_index),
            AudioCommand::Seek { deck, position_seconds } => mixer.deck_mut(deck).seek_seconds(position_seconds),
            AudioCommand::HeadphoneCue { deck, active } => {
                let d = mixer.deck_mut(deck);
                d.headphone_cue = active.unwrap_or(!d.headphone_cue);
            }
            AudioCommand::SetHeadphoneMix { mix } => mixer.headphone_mix = mix.clamp(0.0, 1.0),
            AudioCommand::SetHeadphoneVolume { volume } => mixer.headphone_volume = volume.clamp(0.0, 1.0),

            // PRO DJ TOOLS HANDLERS
            AudioCommand::SetColorFx { deck, value } => mixer.deck_mut(deck).color_fx.set_value(value),
            AudioCommand::SetLoop { deck, beats } => mixer.deck_mut(deck).set_loop(beats),
            AudioCommand::ExitLoop { deck } => mixer.deck_mut(deck).exit_loop(),
            AudioCommand::LoopIn { deck } => mixer.deck_mut(deck).loop_in(),
            AudioCommand::LoopOut { deck } => mixer.deck_mut(deck).loop_out(),
            AudioCommand::ReloopExit { deck } => mixer.deck_mut(deck).reloop_exit(),
            AudioCommand::LoopResize { deck, factor } => mixer.deck_mut(deck).loop_resize(factor),
            AudioCommand::ToggleSlip { deck } => mixer.deck_mut(deck).toggle_slip(),
            AudioCommand::BeatJump { deck, beats } => mixer.deck_mut(deck).beat_jump(beats),
            AudioCommand::SetEchoFx { deck, active, wet, beats } => {
                let d = mixer.deck_mut(deck);
                d.echo_fx.set_active(active);
                d.echo_fx.wet = wet.clamp(0.0, 1.0);
                d.echo_fx.beat_fraction = beats.clamp(0.125, 4.0);
            }
            AudioCommand::ReassignMasterTempo { deck } => mixer.reassign_zero(deck.min(1)),
            AudioCommand::SetSyncMode { mode } => mixer.set_sync_mode(mode),
            AudioCommand::SetBeatGrid { deck, grid } => mixer.set_beatgrid(deck.min(1), grid),
        }
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Linear resampler between two independent sound card clocks.
/// Keeps the queue around a target fill level by gently adjusting the ratio (drift compensation).
struct CueResampler {
    base_step: f64,
    step: f64,
    frac: f64,
    prev: (f32, f32),
    next: (f32, f32),
    primed: bool,
    target_fill: usize,
}

impl CueResampler {
    fn new(in_rate: f64, out_rate: f64) -> Self {
        let step = in_rate / out_rate.max(1.0);
        Self {
            base_step: step,
            step,
            frac: 0.0,
            prev: (0.0, 0.0),
            next: (0.0, 0.0),
            primed: false,
            target_fill: (in_rate * 0.04) as usize, // ~40 ms
        }
    }

    fn begin_block(&mut self, q: &mut VecDeque<(f32, f32)>) {
        let fill = q.len();
        if !self.primed {
            if fill >= self.target_fill {
                self.primed = true;
            }
            return;
        }
        if fill > self.target_fill * 4 {
            // Way too much latency: drop to the target
            let drop = fill - self.target_fill;
            q.drain(..drop);
        }
        let error = (q.len() as f64 - self.target_fill as f64) / self.target_fill.max(1) as f64;
        let correction = (error * 0.005).clamp(-0.01, 0.01);
        self.step = self.step * 0.9 + self.base_step * (1.0 + correction) * 0.1;
    }

    #[inline]
    fn next(&mut self, q: &mut VecDeque<(f32, f32)>) -> (f32, f32) {
        if !self.primed {
            return (0.0, 0.0);
        }
        while self.frac >= 1.0 {
            self.prev = self.next;
            match q.pop_front() {
                Some(f) => self.next = f,
                None => {
                    // Underrun: fade to silence and re-prime
                    self.primed = false;
                    self.frac = 0.0;
                    return (0.0, 0.0);
                }
            }
            self.frac -= 1.0;
        }
        let t = self.frac as f32;
        let out = (
            self.prev.0 + (self.next.0 - self.prev.0) * t,
            self.prev.1 + (self.next.1 - self.prev.1) * t,
        );
        self.frac += self.step;
        out
    }
}
