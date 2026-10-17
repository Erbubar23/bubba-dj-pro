use std::fs::File;
use std::path::Path;
use anyhow::{bail, Context, Result};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;

#[derive(Debug, Clone)]
pub struct DecodedAudio {
    /// Interleaved audio samples (Normalized to -1.0 .. 1.0). Mono or stereo.
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f64,
}

impl DecodedAudio {
    /// Converts the audio buffer to a mono representation for fast analysis
    pub fn to_mono(&self) -> Vec<f32> {
        if self.channels == 1 {
            return self.samples.clone();
        }

        let ch = self.channels as usize;
        self.samples
            .chunks_exact(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    }

    pub fn total_frames(&self) -> usize {
        self.samples.len() / (self.channels as usize).max(1)
    }
}

/// Text tags read from the file (ID3, Vorbis comments, MP4 atoms...)
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
}

impl TrackTags {
    fn absorb(&mut self, rev: &MetadataRevision) {
        for tag in rev.tags() {
            let value = tag.value.to_string().trim().to_string();
            if value.is_empty() {
                continue;
            }
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) if self.title.is_none() => self.title = Some(value),
                Some(StandardTagKey::Artist) if self.artist.is_none() => self.artist = Some(value),
                _ => {}
            }
        }
    }
}

pub struct AudioDecoder;

impl AudioDecoder {
    /// High-performance audio file decode into contiguous float samples
    pub fn decode_file<P: AsRef<Path>>(path: P) -> Result<DecodedAudio> {
        Self::decode_file_with_tags(path).map(|(audio, _)| audio)
    }

    /// Decodes any format supported by Symphonia (MP3, WAV, FLAC, AAC/M4A, ALAC, OGG, AIFF)
    /// and returns the title/artist tags alongside the audio.
    pub fn decode_file_with_tags<P: AsRef<Path>>(path: P) -> Result<(DecodedAudio, TrackTags)> {
        let file = File::open(path.as_ref())
            .with_context(|| format!("No se pudo abrir el archivo: {:?}", path.as_ref()))?;

        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.as_ref().extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let mut probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .context("Formato de audio no soportado o archivo dañado")?;

        let mut tags = TrackTags::default();
        if let Some(meta) = probed.metadata.get() {
            if let Some(rev) = meta.current() {
                tags.absorb(rev);
            }
        }

        let mut format = probed.format;
        if let Some(rev) = format.metadata().current() {
            tags.absorb(rev);
        }

        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .context("El archivo no contiene una pista de audio compatible")?;

        let track_id = track.id;
        let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
        let mut src_channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .context("No se pudo inicializar el decodificador")?;

        let mut all_samples: Vec<f32> = Vec::new();
        let mut sample_buf: Option<SampleBuffer<f32>> = None;
        let mut out_channels = src_channels.min(2);

        loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(SymphoniaError::IoError(_)) => break, // End of stream
                Err(SymphoniaError::ResetRequired) => break,
                Err(e) => bail!("Error leyendo el archivo: {}", e),
            };
            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    if sample_buf.is_none() {
                        sample_rate = spec.rate;
                        src_channels = spec.channels.count().max(1);
                        out_channels = src_channels.min(2);
                    }
                    let buf = sample_buf.get_or_insert_with(|| {
                        SampleBuffer::<f32>::new(decoded.capacity() as u64, spec)
                    });
                    if buf.capacity() < decoded.capacity() * spec.channels.count() {
                        *buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                    }
                    buf.copy_interleaved_ref(decoded);

                    if src_channels <= 2 {
                        all_samples.extend_from_slice(buf.samples());
                    } else {
                        // Surround files: keep front left/right only
                        for frame in buf.samples().chunks_exact(src_channels) {
                            all_samples.push(frame[0]);
                            all_samples.push(frame[1]);
                        }
                    }
                }
                // Skip bad frames gracefully (standard in DJ software)
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => bail!("Error fatal de decodificación: {}", e),
            }
        }

        if all_samples.is_empty() {
            bail!("El archivo no contiene audio decodificable");
        }

        let channels = out_channels.max(1) as u16;
        let total_frames = all_samples.len() / channels as usize;
        let duration_seconds = total_frames as f64 / sample_rate as f64;

        Ok((
            DecodedAudio {
                samples: all_samples,
                sample_rate,
                channels,
                duration_seconds,
            },
            tags,
        ))
    }
}
