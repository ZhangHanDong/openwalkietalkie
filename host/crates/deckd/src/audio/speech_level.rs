//! Level complete generated sentences without changing their dynamics.
//! Kokoro supplies sentence-sized chunks, so one constant gain per chunk
//! avoids pumping on individual syllables or amplifying a trailing pause.

/// Raise quiet speech toward -18 dBFS RMS, capped at +12 dB and -1 dBFS
/// sample peaks. Peak headroom takes precedence over the RMS target.
/// Silence and near-silence are not boosted. The user's volume is applied
/// later by the mixer; this does not change the volume curve or chimes.
pub(super) fn normalize(samples: &[f32]) -> Vec<f32> {
    const TARGET_RMS: f64 = 0.125_892_54;
    const PEAK_LIMIT: f64 = 0.891_250_94;
    const MAX_GAIN: f64 = 3.981_071_7;
    const MIN_RMS: f64 = 0.001; // -60 dBFS: do not lift silence/noise.

    let mut pcm: Vec<f32> = samples
        .iter()
        .map(|&s| if s.is_finite() { s } else { 0.0 })
        .collect();
    if pcm.is_empty() {
        return pcm;
    }
    let peak = pcm.iter().map(|s| f64::from(s.abs())).fold(0.0, f64::max);
    if peak == 0.0 {
        return pcm;
    }
    let rms = (pcm.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt();
    let gain = if (MIN_RMS..TARGET_RMS).contains(&rms) {
        (TARGET_RMS / rms).min(MAX_GAIN)
    } else {
        1.0
    }
    .min(PEAK_LIMIT / peak);
    for sample in &mut pcm {
        *sample = (f64::from(*sample) * gain) as f32;
    }
    pcm
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn quiet_speech_rises_without_changing_its_shape() {
        let input = [0.06, -0.03, 0.0, -0.06, 0.03];
        let out = normalize(&input);
        let gain = out[0] / input[0];
        assert!(gain > 2.0 && gain < 4.0);
        for (a, b) in input.into_iter().zip(out) {
            assert!((b - a * gain).abs() < 1e-7);
        }
    }

    #[test]
    fn a_high_crest_sentence_keeps_peak_headroom() {
        let mut input = vec![0.03; 1000];
        input[0] = -0.8;
        let out = normalize(&input);
        assert!((out[0] + 0.891_250_94).abs() < 1e-6);
        assert!(out.iter().all(|s| s.abs() <= 0.891_251));
        assert!((out[1] / input[1] - out[0] / input[0]).abs() < 1e-6);
    }

    #[test]
    fn gain_is_bounded_and_already_loud_speech_is_preserved() {
        assert!(normalize(&[0.002, -0.002])[0] < 0.008);
        assert_eq!(normalize(&[0.4, -0.3, 0.0]), [0.4, -0.3, 0.0]);
    }

    #[test]
    fn silence_and_invalid_samples_stay_quiet() {
        assert!(normalize(&[]).is_empty());
        assert_eq!(normalize(&[0.0; 100]), [0.0; 100]);
        assert_eq!(normalize(&[0.000_01, -0.000_01]), [0.000_01, -0.000_01]);
        assert_eq!(normalize(&[f32::NAN, f32::INFINITY]), [0.0, 0.0]);
    }
}
