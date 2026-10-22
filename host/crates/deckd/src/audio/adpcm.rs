//! Host-side IMA ADPCM encoder with a small trellis search. The wire format
//! and firmware decoder are unchanged. Keeping several predictor/step paths
//! avoids committing to a locally cheap nibble that causes noisy later samples.
use crate::ble::codec;

const WIDTH: usize = 8;

#[derive(Clone, Copy)]
struct Node {
    state: codec::State,
    error: u64,
    parent: u8,
    code: u8,
}

const EMPTY: Node = Node {
    state: codec::State {
        predictor: 0,
        index: 0,
    },
    error: u64::MAX,
    parent: 0,
    code: 0,
};

/// Encodes at most a normal frame. A greedy reference is kept as a fallback:
/// search pruning can never increase this frame's squared reconstruction error.
pub(super) fn audio_frame(
    stream: u8,
    seq: u8,
    state: &mut codec::State,
    pcm: &[i16],
    out: &mut [u8],
) -> usize {
    let count = pcm
        .len()
        .min(codec::FRAME_SAMPLES)
        .min(out.len().saturating_sub(codec::AUDIO_HEADER) * 2)
        & !1;
    let pcm = &pcm[..count];
    let initial = *state;
    let bytes = codec::audio_frame(stream, seq, state, pcm, out);
    if count == 0 {
        return bytes;
    }
    let mut reference = initial;
    let mut greedy_error = 0u64;
    for (i, &sample) in pcm.iter().enumerate() {
        let code = (out[codec::AUDIO_HEADER + i / 2] >> (4 * (i % 2))) & 15;
        let error = i64::from(sample) - i64::from(reference.decode(code));
        greedy_error += (error * error) as u64;
    }
    let mut beam = [EMPTY; WIDTH];
    beam[0] = Node {
        state: initial,
        error: 0,
        ..EMPTY
    };
    let mut live = 1;
    let mut history = [[(0u8, 0u8); WIDTH]; codec::FRAME_SAMPLES];
    for (i, &sample) in pcm.iter().enumerate() {
        let mut next = [EMPTY; WIDTH];
        for (parent, node) in beam[..live].iter().enumerate() {
            for code in 0..16 {
                let mut candidate = node.state;
                let error = i64::from(sample) - i64::from(candidate.decode(code));
                let error = node.error + (error * error) as u64;
                if error >= next[WIDTH - 1].error {
                    continue;
                }
                if next
                    .iter()
                    .any(|n| n.error <= error && n.state == candidate)
                {
                    continue;
                }
                let at = next.iter().position(|n| n.error > error).unwrap();
                next.copy_within(at..WIDTH - 1, at + 1);
                next[at] = Node {
                    state: candidate,
                    error,
                    parent: parent as u8,
                    code,
                };
            }
        }
        live = next.iter().take_while(|n| n.error != u64::MAX).count();
        for (slot, node) in history[i].iter_mut().zip(next) {
            *slot = (node.parent, node.code);
        }
        beam = next;
    }
    if beam[0].error < greedy_error {
        *state = beam[0].state;
        out[codec::AUDIO_HEADER..bytes].fill(0);
        let mut path = 0;
        for i in (0..count).rev() {
            let (parent, code) = history[i][path];
            out[codec::AUDIO_HEADER + i / 2] |= code << (4 * (i % 2));
            path = usize::from(parent);
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_preserves_decoder_state_and_reduces_error_in_a_changing_signal() {
        let mut state = codec::State::default();
        let mut greedy = state;
        let mut improved_error = 0f64;
        let mut greedy_error = 0f64;
        for frame in 0..24 {
            let pcm: Vec<i16> = (0..110)
                .map(|i| {
                    let t = (frame * 110 + i) as f64 / 16000.0;
                    (6000.0 * (std::f64::consts::TAU * 370.0 * t).sin()
                        + 2000.0 * (std::f64::consts::TAU * 2600.0 * t).sin())
                        as i16
                })
                .collect();
            let mut improved = [0; codec::MAX_FRAME];
            let mut reference = improved;
            let n = audio_frame(7, frame as u8, &mut state, &pcm, &mut improved);
            let m = codec::audio_frame(7, frame as u8, &mut greedy, &pcm, &mut reference);
            assert_eq!(n, m);
            for (encoded, expected, error) in [
                (&improved[..n], state, &mut improved_error),
                (&reference[..m], greedy, &mut greedy_error),
            ] {
                let codec::Frame::Audio {
                    mut state, data, ..
                } = codec::parse(encoded)
                else {
                    panic!()
                };
                let mut decoded = [0; 110];
                assert_eq!(codec::decode(&mut state, data, &mut decoded), pcm.len());
                assert_eq!(state, expected);
                *error += pcm
                    .iter()
                    .zip(decoded)
                    .map(|(&a, b)| (f64::from(a) - f64::from(b)).powi(2))
                    .sum::<f64>();
            }
        }
        assert!(
            improved_error < greedy_error * 0.8,
            "{improved_error} vs {greedy_error}"
        );
    }
}
