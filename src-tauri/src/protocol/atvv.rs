//! ATVV transport used by the remote.
//!
//! The host writes commands to the TX characteristic and receives reports on
//! the control and audio characteristics. UUIDs, opcodes and the capability
//! report layout are fixed by the transport, so they are spelled out here as
//! constants rather than derived from anything.

use serde::{Deserialize, Serialize};

pub const ATVV_SERVICE: &str = "AB5E0001-5A21-4F05-BC7D-AF01F617B664";
pub const ATVV_TX: &str = "AB5E0002-5A21-4F05-BC7D-AF01F617B664";
pub const ATVV_RX_AUDIO: &str = "AB5E0003-5A21-4F05-BC7D-AF01F617B664";
pub const ATVV_CONTROL: &str = "AB5E0004-5A21-4F05-BC7D-AF01F617B664";

/// Host → remote: request the capability report.
pub const GET_CAPS: [u8; 6] = [0x0A, 0x01, 0x00, 0x00, 0x03, 0x03];

// Opcodes carried in the first byte of a control-channel notification.
/// The remote asks the host to open the microphone for the voice key.
pub const OP_REMOTE_MIC_REQUEST: u8 = 0x08;
/// The remote started an audio stream.
pub const OP_STREAM_START: u8 = 0x04;
/// The remote stopped the audio stream.
pub const OP_STREAM_STOP: u8 = 0x00;
/// Decoder resync: carries predictor + step index for the ADPCM decoder.
pub const OP_AUDIO_SYNC: u8 = 0x0A;
/// Capability report.
pub const OP_CAPS: u8 = 0x0B;
/// The remote refused to open the microphone.
pub const OP_MIC_OPEN_ERROR: u8 = 0x0C;

/// ADPCM 16 kHz codec bit (capability mask) / sub-codec value (stream start).
pub const CODEC_ADPCM_16K: u8 = 0x02;
/// ADPCM 8 kHz codec bit (capability mask) / sub-codec value (stream start).
pub const CODEC_ADPCM_8K: u8 = 0x01;

/// Any codec bit this host can decode.
const KNOWN_CODECS: u8 = CODEC_ADPCM_16K | CODEC_ADPCM_8K;

// Field offsets inside a capability report.
const CAPS_VERSION_AT: usize = 1; // u16, big-endian
const CAPS_CODEC_V10_AT: usize = 3;
const CAPS_INTERACTION_V10_AT: usize = 4;
const CAPS_CODEC_V04_AT: usize = 4;
const CAPS_FRAME_AT: usize = 5; // u16, big-endian
const CAPS_V10_MIN_LEN: usize = 7;
const CAPS_V04_MIN_LEN: usize = 9;

/// Frame size assumed when the report omits one.
const FALLBACK_FRAME_BYTES: u16 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtvvVersion {
    V04,
    V10,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtvvCapabilities {
    pub version: AtvvVersion,
    pub sample_rate: u32,
    pub adpcm_bits: u8,
    pub frame_size: usize,
}

impl Default for AtvvCapabilities {
    fn default() -> Self {
        Self {
            version: AtvvVersion::V10,
            sample_rate: 16_000,
            adpcm_bits: 4,
            frame_size: FALLBACK_FRAME_BYTES as usize,
        }
    }
}

/// Parse a capability report, dispatching on the version field.
///
/// Layout by version:
/// - `>= 0x0100` (v1.0): `[op, ver_hi, ver_lo, codec_mask, interaction, frame_hi, frame_lo, …]`
/// - otherwise (v0.4):   `[op, ver_hi, ver_lo, ?, codec_mask, frame_hi, frame_lo, …]`, at least 9 bytes
pub fn parse_caps_response(report: &[u8]) -> Option<AtvvCapabilities> {
    if report.first().copied()? != OP_CAPS {
        return None;
    }
    let version_field = read_be_u16(report, CAPS_VERSION_AT)?;

    let (version, codec_mask) = if version_field >= 0x0100 {
        if report.len() < CAPS_V10_MIN_LEN {
            return None;
        }
        (AtvvVersion::V10, v10_codec_mask(report))
    } else {
        if report.len() < CAPS_V04_MIN_LEN {
            return None;
        }
        (AtvvVersion::V04, report[CAPS_CODEC_V04_AT])
    };

    let frame_bytes = read_be_u16(report, CAPS_FRAME_AT)?;

    Some(AtvvCapabilities {
        version,
        sample_rate: sample_rate_for(codec_mask),
        adpcm_bits: 4,
        frame_size: if frame_bytes == 0 {
            FALLBACK_FRAME_BYTES as usize
        } else {
            frame_bytes as usize
        },
    })
}

/// Codec mask of a v1.0 report, tolerating firmware that transposes it.
///
/// Some builds put the interaction byte where the codec mask belongs and vice
/// versa. The documented position wins; the other byte is only trusted when it
/// is long enough to carry it and the documented one has no codec bit at all.
fn v10_codec_mask(report: &[u8]) -> u8 {
    let documented = report[CAPS_CODEC_V10_AT];
    let transposed_has_codec = report.len() >= CAPS_V04_MIN_LEN
        && report[CAPS_INTERACTION_V10_AT] & KNOWN_CODECS != 0;

    if documented == 0 && transposed_has_codec {
        report[CAPS_INTERACTION_V10_AT]
    } else {
        documented
    }
}

fn sample_rate_for(codec_mask: u8) -> u32 {
    if codec_mask & CODEC_ADPCM_16K != 0 {
        16_000
    } else if codec_mask & CODEC_ADPCM_8K != 0 {
        8_000
    } else {
        16_000
    }
}

fn read_be_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let pair = bytes.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([pair[0], pair[1]]))
}

/// Host → remote: open the microphone (reply to an `OP_REMOTE_MIC_REQUEST`).
pub fn mic_open_message(version: AtvvVersion) -> Vec<u8> {
    const OP_MIC_OPEN: u8 = 0x0C;
    match version {
        AtvvVersion::V04 => vec![OP_MIC_OPEN, 0x00, CODEC_ADPCM_16K],
        AtvvVersion::V10 => vec![OP_MIC_OPEN, 0x00],
    }
}

/// Host → remote: close the microphone for `stream_id`.
pub fn mic_close_message(version: AtvvVersion, stream_id: u8) -> Vec<u8> {
    const OP_MIC_CLOSE: u8 = 0x0D;
    match version {
        AtvvVersion::V04 => vec![OP_MIC_CLOSE],
        AtvvVersion::V10 => vec![OP_MIC_CLOSE, stream_id],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v10_report(codec_mask: u8, frame: u16) -> Vec<u8> {
        let [frame_hi, frame_lo] = frame.to_be_bytes();
        vec![OP_CAPS, 0x01, 0x00, codec_mask, 0x01, frame_hi, frame_lo, 0x00, 0x00]
    }

    #[test]
    fn v10_report_picks_16k_and_frame_size() {
        let caps = parse_caps_response(&v10_report(CODEC_ADPCM_16K, 120)).unwrap();
        assert_eq!(caps.version, AtvvVersion::V10);
        assert_eq!(caps.sample_rate, 16_000);
        assert_eq!(caps.frame_size, 120);
    }

    #[test]
    fn v10_report_accepts_a_transposed_codec_byte() {
        // Documented codec byte empty, interaction byte carries the codec.
        let report = vec![OP_CAPS, 0x01, 0x00, 0x00, CODEC_ADPCM_8K, 0x00, 0x78, 0x00, 0x00];
        let caps = parse_caps_response(&report).unwrap();
        assert_eq!(caps.sample_rate, 8_000);
    }

    #[test]
    fn v10_short_report_is_rejected() {
        assert!(parse_caps_response(&[OP_CAPS, 0x01, 0x00, 0x02, 0x01, 0x00]).is_none());
    }

    #[test]
    fn v04_report_reads_codec_at_the_other_offset() {
        let report = vec![OP_CAPS, 0x00, 0x04, 0x00, CODEC_ADPCM_8K, 0x00, 0x50, 0x00, 0x00];
        let caps = parse_caps_response(&report).unwrap();
        assert_eq!(caps.version, AtvvVersion::V04);
        assert_eq!(caps.sample_rate, 8_000);
        assert_eq!(caps.frame_size, 80);
    }

    #[test]
    fn zero_frame_size_falls_back() {
        let caps = parse_caps_response(&v10_report(CODEC_ADPCM_16K, 0)).unwrap();
        assert_eq!(caps.frame_size, FALLBACK_FRAME_BYTES as usize);
    }

    #[test]
    fn non_caps_opcode_is_rejected() {
        assert!(parse_caps_response(&[0x04, 0x01, 0x00, 0x02, 0x01, 0x00, 0x78]).is_none());
    }

    #[test]
    fn mic_commands_differ_by_version() {
        assert_eq!(mic_open_message(AtvvVersion::V04), vec![0x0C, 0x00, CODEC_ADPCM_16K]);
        assert_eq!(mic_open_message(AtvvVersion::V10), vec![0x0C, 0x00]);
        assert_eq!(mic_close_message(AtvvVersion::V04, 7), vec![0x0D]);
        assert_eq!(mic_close_message(AtvvVersion::V10, 7), vec![0x0D, 7]);
    }
}
