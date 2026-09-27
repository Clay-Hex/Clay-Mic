use super::adpcm::AdpcmDecoder;

/// Gain applied to the decoded PCM: the remote's microphone is quiet, so the
/// signal is amplified before recognition.
const OUTPUT_GAIN_DB: f64 = 12.0;

/// Accumulates ATVV audio notifications into fixed-size ADPCM frames, decodes
/// them to PCM, and applies a light smoothing + gain pass.
pub struct AudioPipeline {
    decoder: AdpcmDecoder,
    sample_rate: u32,
    frame_size: usize,
    pending: Vec<u8>,
    pcm: Vec<i16>,
    pending_sync: Option<(i32, i32)>,
}

impl AudioPipeline {
    pub fn new(sample_rate: u32, frame_size: usize) -> Self {
        Self {
            decoder: AdpcmDecoder::new(),
            sample_rate,
            frame_size: frame_size.max(1),
            pending: Vec::new(),
            pcm: Vec::new(),
            pending_sync: None,
        }
    }

    /// Drop any partial frame and reset the decoder (stream start).
    pub fn reset_stream(&mut self) {
        self.pending.clear();
        self.pending_sync = None;
        self.decoder.reset(0, 0);
    }

    /// Queue a decoder resync (ATVV audio-sync), applied at the next frame.
    pub fn set_sync(&mut self, predictor: i16, step_index: u8) {
        self.pending.clear();
        self.pending_sync = Some((predictor as i32, step_index as i32));
    }

    /// Feed a raw audio notification; decodes every complete frame it contains.
    pub fn feed(&mut self, data: &[u8]) {
        self.pending.extend_from_slice(data);
        while self.pending.len() >= self.frame_size {
            let frame: Vec<u8> = self.pending.drain(..self.frame_size).collect();
            if let Some((predictor, step)) = self.pending_sync.take() {
                self.decoder.reset(predictor, step);
            }
            let samples = self.decoder.decode(&frame);
            self.pcm.extend_from_slice(&samples);
        }
    }

    pub fn sample_count(&self) -> usize {
        self.pcm.len()
    }

    /// Finalize into a 16-bit mono WAV buffer with smoothing + gain applied.
    pub fn to_wav(&self) -> Vec<u8> {
        AdpcmDecoder::pcm_to_wav(&postprocess(&self.pcm), self.sample_rate)
    }
}

/// Three-point smoothing followed by a bounded gain, so the remote's quiet
/// signal is usable for recognition.
fn postprocess(input: &[i16]) -> Vec<i16> {
    if input.is_empty() {
        return Vec::new();
    }
    let mut smoothed: Vec<i32> = input.iter().map(|&s| s as i32).collect();
    if input.len() >= 3 {
        for index in 1..input.len() - 1 {
            smoothed[index] = (input[index - 1] as i32
                + 2 * input[index] as i32
                + input[index + 1] as i32)
                >> 2;
        }
    }
    let gain = 10f64.powf(OUTPUT_GAIN_DB / 20.0);
    smoothed
        .into_iter()
        .map(|value| {
            let scaled = (value as f64 * gain).round();
            scaled.clamp(-32768.0, 32767.0) as i16
        })
        .collect()
}
