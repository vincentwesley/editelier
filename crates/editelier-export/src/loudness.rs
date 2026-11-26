//! Integrated loudness + true peak (EBU R128 / BS.1770 via the MIT `ebur128` crate) and
//! the gain needed to hit a target (D6). The caller applies the gain and, when
//! `needs_limiter` is set, a true-peak limiter.

use crate::{AUDIO_CHANNELS, AUDIO_SAMPLE_RATE, ExportError, LoudnessTarget};
use ebur128::{EbuR128, Mode};

/// Social preset: -14 LUFS / -1 dBTP (a convention, not a published platform spec).
pub const SOCIAL: LoudnessTarget = LoudnessTarget {
    target_lufs: -14.0,
    true_peak_dbtp: -1.0,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessReport {
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
    /// Gain that moves integrated loudness to the target.
    pub gain_db: f64,
    /// True peak after `gain_db` would exceed the ceiling.
    pub needs_limiter: bool,
}

/// Measure 48 kHz stereo f32 interleaved audio. Silence yields 0 dB gain.
pub fn measure(samples: &[f32], target: LoudnessTarget) -> Result<LoudnessReport, ExportError> {
    let err = |e: ebur128::Error| ExportError::Media(format!("ebur128: {e:?}"));
    let mut m =
        EbuR128::new(AUDIO_CHANNELS, AUDIO_SAMPLE_RATE, Mode::I | Mode::TRUE_PEAK).map_err(err)?;
    m.add_frames_f32(samples).map_err(err)?;
    let integrated = m.loudness_global().map_err(err)?;
    let peak = (0..AUDIO_CHANNELS)
        .map(|c| m.true_peak(c))
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?
        .into_iter()
        .fold(0.0f64, f64::max);
    let tp_db = if peak > 0.0 {
        20.0 * peak.log10()
    } else {
        f64::NEG_INFINITY
    };
    let gain_db = if integrated.is_finite() {
        target.target_lufs - integrated
    } else {
        0.0
    };
    Ok(LoudnessReport {
        integrated_lufs: integrated,
        true_peak_dbtp: tp_db,
        gain_db,
        needs_limiter: tp_db + gain_db > target.true_peak_dbtp,
    })
}

/// Linear factor for a gain in dB.
pub fn db_to_linear(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amp: f32, secs: usize) -> Vec<f32> {
        (0..AUDIO_SAMPLE_RATE as usize * secs)
            .flat_map(|i| {
                let s = amp * (i as f32 * 1000.0 * std::f32::consts::TAU / 48_000.0).sin();
                [s, s]
            })
            .collect()
    }

    #[test]
    fn gain_hits_target() {
        let quiet = sine(0.05, 5);
        let r = measure(&quiet, SOCIAL).unwrap();
        assert!(r.gain_db > 0.0);
        assert!(!r.needs_limiter);
        let g = db_to_linear(r.gain_db);
        let fixed: Vec<f32> = quiet.iter().map(|s| s * g).collect();
        let after = measure(&fixed, SOCIAL).unwrap();
        assert!((after.integrated_lufs + 14.0).abs() < 0.1, "{after:?}");
        assert!(after.true_peak_dbtp <= -1.0, "{after:?}");
    }

    #[test]
    fn loud_peaky_audio_flags_limiter() {
        // Full-scale sine is about -3 LUFS: gain is negative, no limiter needed.
        let r = measure(&sine(1.0, 3), SOCIAL).unwrap();
        assert!(r.gain_db < -5.0 && !r.needs_limiter);
        // Quiet sine with one full-scale spike: raising it would clip.
        let mut s = sine(0.02, 5);
        s[1000] = 1.0;
        assert!(measure(&s, SOCIAL).unwrap().needs_limiter);
    }

    #[test]
    fn silence_is_unchanged() {
        let r = measure(&vec![0.0; 96_000], SOCIAL).unwrap();
        assert_eq!(r.gain_db, 0.0);
    }
}
