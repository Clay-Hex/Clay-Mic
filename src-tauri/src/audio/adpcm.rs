//! IMA/DVI ADPCM decoding for the ATVV voice stream.
//!
//! Every nibble is a signed 4-bit step against a running predictor: the low
//! three bits hold a magnitude, the high bit its sign. The quantiser step is
//! itself adaptive — after each sample the step index shifts by a fixed amount
//! from the IMA step-index table. Both tables below are the ones defined by the
//! IMA Digital Audio Interchange specification and reused by essentially every
//! ADPCM codec.

/// Adaptive step sizes, indexed by the current step index (0..=88).
const STEP_SIZE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// Movement of the step index per nibble, looked up by the nibble's low bits.
const STEP_ADJUST: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

const MIN_STEP_INDEX: i32 = 0;

/// Sample amplitude bounds (16-bit signed).
const SAMPLE_MIN: i32 = i16::MIN as i32;
const SAMPLE_MAX: i32 = i16::MAX as i32;

/// Stateful ADPCM decoder.
///
/// `value` and `step_index` carry across frames, so one decoder instance follows
/// a whole utterance; an ATVV audio-sync report can reseed them mid-stream.
#[derive(Debug, Clone, Default)]
pub struct AdpcmDecoder {
    value: i32,
    step_index: i32,
}

impl AdpcmDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn max_step_index() -> i32 {
        STEP_SIZE.len() as i32 - 1
    }

    /// Reseed the predictor and step index (stream start / audio-sync).
    pub fn reset(&mut self, value: i32, step_index: i32) {
        self.set_value(value);
        self.step_index = step_index.clamp(MIN_STEP_INDEX, Self::max_step_index());
    }

    /// Decode a raw payload into PCM, high nibble of each byte first.
    pub fn decode(&mut self, payload: &[u8]) -> Vec<i16> {
        let mut pcm = Vec::with_capacity(payload.len() * 2);
        for &byte in payload {
            self.decode_nibble((byte >> 4) & 0x0F, &mut pcm);
            self.decode_nibble(byte & 0x0F, &mut pcm);
        }
        pcm
    }

    /// Reconstruct one sample from a nibble.
    ///
    /// The magnitude is the quantiser base (`step / 8`) plus a full, half or
    /// quarter step for each set magnitude bit — the divisions spell out the
    /// spec's `>> 3 / >> 1 / >> 2` terms without relying on shift semantics.
    fn decode_nibble(&mut self, nibble: u8, pcm: &mut Vec<i16>) {
        let step = STEP_SIZE[self.step_index as usize];

        let magnitude = step / 8
            + if nibble & 0x04 != 0 { step } else { 0 }
            + if nibble & 0x02 != 0 { step / 2 } else { 0 }
            + if nibble & 0x01 != 0 { step / 4 } else { 0 };

        let delta = if nibble & 0x08 != 0 {
            -magnitude
        } else {
            magnitude
        };
        self.set_value(self.value + delta);

        let adjust = STEP_ADJUST[(nibble & 0x07) as usize];
        self.step_index = (self.step_index + adjust).clamp(MIN_STEP_INDEX, Self::max_step_index());

        pcm.push(self.value as i16);
    }

    fn set_value(&mut self, value: i32) {
        self.value = value.clamp(SAMPLE_MIN, SAMPLE_MAX);
    }

    /// Wrap 16-bit mono PCM in a minimal RIFF/WAVE container.
    pub fn pcm_to_wav(pcm: &[i16], sample_rate: u32) -> Vec<u8> {
        const CHANNELS: u16 = 1;
        const BITS_PER_SAMPLE: u16 = 16;
        const FORMAT_PCM: u16 = 1;
        const HEADER_BYTES: u32 = 44;

        let block_align = CHANNELS * BITS_PER_SAMPLE / 8;
        let byte_rate = sample_rate * block_align as u32;
        let data_bytes = (pcm.len() * std::mem::size_of::<i16>()) as u32;

        let mut wav = Vec::with_capacity(HEADER_BYTES as usize + data_bytes as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(HEADER_BYTES - 8 + data_bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&FORMAT_PCM.to_le_bytes());
        wav.extend_from_slice(&CHANNELS.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());

        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());
        for sample in pcm {
            wav.extend_from_slice(&sample.to_le_bytes());
        }

        wav
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_stays_at_zero() {
        let mut decoder = AdpcmDecoder::new();
        let pcm = decoder.decode(&[0u8; 128]);
        assert_eq!(pcm.len(), 256);
        assert!(pcm.iter().all(|&sample| sample == 0));
    }

    #[test]
    fn sign_bit_moves_the_predictor_both_ways() {
        let mut decoder = AdpcmDecoder::new();

        let up = decoder.decode(&[0x70])[0]; // magnitude bits set, sign clear
        assert!(up > 0);

        let down = decoder.decode(&[0x90])[0]; // sign set, one magnitude bit
        assert!(down < up);
    }

    #[test]
    fn reset_reseeds_both_fields() {
        let mut decoder = AdpcmDecoder::new();
        decoder.reset(1000, 200);
        assert_eq!(decoder.value, 1000);
        assert_eq!(decoder.step_index, 88); // clamped to the table's last index
    }

    #[test]
    fn wav_container_has_expected_chunks() {
        let pcm = vec![0i16; 16_000];
        let wav = AdpcmDecoder::pcm_to_wav(&pcm, 16_000);

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 32_000);
    }
}
