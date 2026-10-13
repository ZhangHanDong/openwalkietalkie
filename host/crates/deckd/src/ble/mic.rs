//! The microphone half of the link as a pure fold: mic notification values
//! in, [`MicEvent`]s out.
//!
//! The deck sends START, an AUDIO frame every 20 ms and END, one frame per
//! notification. Each AUDIO frame carries the coder state from before its
//! first sample, so it decodes on its own and a lost frame costs its 20 ms
//! and nothing after it. What this adds is the bookkeeping every consumer
//! would otherwise repeat, and gets subtly wrong once:
//!
//! * **Loss is a `seq` gap.** `seq` counts AUDIO frames within a stream
//!   (mod 256, from 0), so `lost_frames` is the gap since the previous frame
//!   of the same stream, and the gap before END's `next_seq` is the tail.
//! * **A consumer always sees START … END.** A stream whose START went
//!   missing (or that was live before the host subscribed) starts at its
//!   first AUDIO frame; a stream that never got its END is ended — with
//!   [`codec::END_ERROR`] — by the next stream's first frame or by
//!   [`MicDecoder::abandon`] when the link drops.
//! * **Late frames are dropped, not resurrected.** AUDIO or END of a stream
//!   that already ended is stale; a stream announced with a codec or rate
//!   this host cannot decode is ignored whole; unknown kinds are a newer
//!   deck talking and are ignored too.
use super::codec::{self, Frame};

/// What the microphone said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MicEvent {
    /// A stream began (a PTT press, or a MIC request).
    Start { stream: u8 },
    /// One frame of 16 kHz mono PCM: 320 samples (20 ms), fewer when the
    /// link's attribute limit is small. `lost_frames` AUDIO frames of this
    /// stream went missing just before this one (mod 256).
    Audio {
        stream: u8,
        seq: u8,
        lost_frames: u8,
        samples: Vec<i16>,
    },
    /// The stream ended. `next_seq` is the seq the deck would have used
    /// next, so `next_seq - (last seq + 1)` frames were lost at the tail.
    /// `reason` is one of `codec::END_*`; the host itself reports
    /// `END_ERROR` for a stream the link or a newer stream cut short.
    End {
        stream: u8,
        next_seq: u8,
        reason: u8,
    },
}

/// Running totals since the decoder was made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MicStats {
    /// Streams started.
    pub streams: u64,
    /// AUDIO frames decoded.
    pub frames: u64,
    /// AUDIO frames missing: every `seq` gap, tails before END included.
    pub lost: u64,
    /// Values dropped: stale, unsupported or unknown.
    pub ignored: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Live {
    stream: u8,
    /// The `seq` the next AUDIO frame should carry.
    next_seq: u8,
}

/// The fold. One per link; it lives wherever notifications arrive.
#[derive(Clone, Debug, Default)]
pub struct MicDecoder {
    live: Option<Live>,
    /// The stream that ended last: its late frames are stale.
    ended: Option<u8>,
    /// A stream announced with a codec or rate this host cannot decode.
    unsupported: Option<u8>,
    stats: MicStats,
}

impl MicDecoder {
    #[must_use]
    pub fn new() -> MicDecoder {
        MicDecoder::default()
    }

    /// The stream being received, if any.
    #[must_use]
    pub fn live(&self) -> Option<u8> {
        self.live.map(|l| l.stream)
    }

    #[must_use]
    pub fn stats(&self) -> MicStats {
        self.stats
    }

    /// Fold one notification value in; `emit` gets zero to three events
    /// (an END for a stream cut short, a START, the AUDIO).
    pub fn push(&mut self, value: &[u8], mut emit: impl FnMut(MicEvent)) {
        match codec::parse(value) {
            Frame::Start {
                stream,
                codec,
                rate_khz,
                ..
            } => {
                if let Some(end) = self.abandon() {
                    emit(end);
                }
                // A START is always a new stream, even under the id that
                // ended last: a deck that rebooted counts from the start.
                if self.ended == Some(stream) {
                    self.ended = None;
                }
                if codec != codec::CODEC_IMA_ADPCM || rate_khz != codec::RATE_KHZ {
                    self.unsupported = Some(stream);
                    self.stats.ignored += 1;
                    return;
                }
                self.unsupported = None;
                self.begin(stream, &mut emit);
            }
            Frame::Audio {
                stream,
                seq,
                state,
                data,
            } => {
                if self.unsupported == Some(stream) {
                    self.stats.ignored += 1;
                    return;
                }
                let live = match self.live {
                    Some(live) if live.stream == stream => live,
                    _ if self.ended == Some(stream) => {
                        self.stats.ignored += 1;
                        return;
                    }
                    _ => {
                        // Its START never arrived: the stream starts here,
                        // and the frames before `seq` are lost.
                        if let Some(end) = self.abandon() {
                            emit(end);
                        }
                        self.begin(stream, &mut emit)
                    }
                };
                let lost_frames = seq.wrapping_sub(live.next_seq);
                self.live = Some(Live {
                    stream,
                    next_seq: seq.wrapping_add(1),
                });
                let mut samples = vec![0i16; data.len() * 2];
                let mut state = state;
                codec::decode(&mut state, data, &mut samples);
                self.stats.frames += 1;
                self.stats.lost += u64::from(lost_frames);
                emit(MicEvent::Audio {
                    stream,
                    seq,
                    lost_frames,
                    samples,
                });
            }
            Frame::End {
                stream,
                next_seq,
                reason,
            } => match self.live {
                Some(live) if live.stream == stream => {
                    self.stats.lost += u64::from(next_seq.wrapping_sub(live.next_seq));
                    self.live = None;
                    self.ended = Some(stream);
                    emit(MicEvent::End {
                        stream,
                        next_seq,
                        reason,
                    });
                }
                _ => {
                    if self.unsupported == Some(stream) {
                        self.unsupported = None;
                        self.ended = Some(stream);
                    }
                    self.stats.ignored += 1;
                }
            },
            // MIC and LISTEN are host to deck, INFO is the cable's
            // handshake (taken before this is asked); anything else is a
            // newer deck.
            Frame::Mic { .. } | Frame::Listen { .. } | Frame::Info { .. } | Frame::Other => {
                self.stats.ignored += 1;
            }
        }
    }

    /// The link went away (or a newer stream began) while a stream was
    /// live: its END is never coming, so here is one, `END_ERROR`, with the
    /// `next_seq` this host expected. Late frames of it are stale from now.
    pub fn abandon(&mut self) -> Option<MicEvent> {
        let live = self.live.take()?;
        self.ended = Some(live.stream);
        Some(MicEvent::End {
            stream: live.stream,
            next_seq: live.next_seq,
            reason: codec::END_ERROR,
        })
    }

    fn begin(&mut self, stream: u8, emit: &mut impl FnMut(MicEvent)) -> Live {
        let live = Live {
            stream,
            next_seq: 0,
        };
        self.live = Some(live);
        self.stats.streams += 1;
        emit(MicEvent::Start { stream });
        live
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble::codec::{
        audio_frame, end_frame, start_frame, State, END_ABORT, END_ERROR, END_LIMIT, END_RELEASE,
        FRAME_SAMPLES, KIND_AUDIO, KIND_START, MAX_FRAME,
    };

    fn start(stream: u8) -> Vec<u8> {
        let mut out = [0u8; 8];
        let n = start_frame(stream, &mut out);
        out[..n].to_vec()
    }

    fn end(stream: u8, next_seq: u8, reason: u8) -> Vec<u8> {
        let mut out = [0u8; 8];
        let n = end_frame(stream, next_seq, reason, &mut out);
        out[..n].to_vec()
    }

    /// One AUDIO frame of a ramp, so every frame decodes to something.
    fn audio(stream: u8, seq: u8, state: &mut State) -> Vec<u8> {
        let pcm: Vec<i16> = (0..FRAME_SAMPLES as i16).map(|i| i * 40 - 6000).collect();
        let mut out = [0u8; MAX_FRAME];
        let n = audio_frame(stream, seq, state, &pcm, &mut out);
        out[..n].to_vec()
    }

    fn fold(decoder: &mut MicDecoder, values: &[Vec<u8>]) -> Vec<MicEvent> {
        let mut events = Vec::new();
        for value in values {
            decoder.push(value, |e| events.push(e));
        }
        events
    }

    /// The events with the samples left out, for comparing shapes.
    fn shape(events: &[MicEvent]) -> Vec<(char, u8, u8, u8)> {
        events
            .iter()
            .map(|e| match e {
                MicEvent::Start { stream } => ('S', *stream, 0, 0),
                MicEvent::Audio {
                    stream,
                    seq,
                    lost_frames,
                    ..
                } => ('A', *stream, *seq, *lost_frames),
                MicEvent::End {
                    stream,
                    next_seq,
                    reason,
                } => ('E', *stream, *next_seq, *reason),
            })
            .collect()
    }

    #[test]
    fn a_clean_stream_decodes_every_frame_with_the_state_it_carries() {
        let mut encoder = State::default();
        let frames = vec![
            start(4),
            audio(4, 0, &mut encoder),
            audio(4, 1, &mut encoder),
            audio(4, 2, &mut encoder),
            end(4, 3, END_RELEASE),
        ];
        let mut decoder = MicDecoder::new();
        let events = fold(&mut decoder, &frames);
        assert_eq!(
            shape(&events),
            [
                ('S', 4, 0, 0),
                ('A', 4, 0, 0),
                ('A', 4, 1, 0),
                ('A', 4, 2, 0),
                ('E', 4, 3, END_RELEASE)
            ]
        );
        // Each frame carries the state a decoder that saw every frame
        // before it would have reached, so each decodes exactly as that
        // decoder would.
        let mut reference = State::default();
        for (i, frame) in frames[1..4].iter().enumerate() {
            let Frame::Audio { state, data, .. } = codec::parse(frame) else {
                panic!("audio");
            };
            assert_eq!(state, reference, "frame {i} carries the running state");
            let mut expect = vec![0i16; FRAME_SAMPLES];
            codec::decode(&mut reference, data, &mut expect);
            let MicEvent::Audio { samples, .. } = &events[1 + i] else {
                panic!("audio");
            };
            assert_eq!(samples, &expect, "frame {i}");
        }
        assert_eq!(
            decoder.stats(),
            MicStats {
                streams: 1,
                frames: 3,
                lost: 0,
                ignored: 0
            }
        );
        assert_eq!(decoder.live(), None);
    }

    #[test]
    fn a_seq_gap_is_the_lost_frame_count_and_wraps_at_256() {
        let mut encoder = State::default();
        let mut decoder = MicDecoder::new();
        let events = fold(
            &mut decoder,
            &[
                start(1),
                audio(1, 0, &mut encoder),
                audio(1, 3, &mut encoder),
                audio(1, 254, &mut encoder),
                audio(1, 255, &mut encoder),
                audio(1, 1, &mut encoder),
                end(1, 5, END_LIMIT),
            ],
        );
        assert_eq!(
            shape(&events),
            [
                ('S', 1, 0, 0),
                ('A', 1, 0, 0),
                ('A', 1, 3, 2),
                ('A', 1, 254, 250),
                ('A', 1, 255, 0),
                ('A', 1, 1, 1),
                ('E', 1, 5, END_LIMIT)
            ]
        );
        // 2 + 250 + 1 in the middle, and seq 2..=4 at the tail.
        assert_eq!(decoder.stats().lost, 2 + 250 + 1 + 3);
    }

    #[test]
    fn a_missing_start_begins_the_stream_at_its_first_frame() {
        let mut encoder = State::default();
        let mut decoder = MicDecoder::new();
        let events = fold(
            &mut decoder,
            &[audio(9, 2, &mut encoder), audio(9, 3, &mut encoder)],
        );
        assert_eq!(
            shape(&events),
            [('S', 9, 0, 0), ('A', 9, 2, 2), ('A', 9, 3, 0)]
        );
        assert_eq!(decoder.live(), Some(9));
    }

    #[test]
    fn a_stream_without_its_end_is_ended_by_the_next_one_or_by_the_link() {
        let mut encoder = State::default();
        let mut decoder = MicDecoder::new();
        let events = fold(
            &mut decoder,
            &[
                start(1),
                audio(1, 0, &mut encoder),
                // END of 1 lost; stream 2 starts.
                start(2),
                audio(2, 0, &mut encoder),
                // END of 2 and START of 3 lost; 3's audio arrives.
                audio(3, 0, &mut encoder),
            ],
        );
        assert_eq!(
            shape(&events),
            [
                ('S', 1, 0, 0),
                ('A', 1, 0, 0),
                ('E', 1, 1, END_ERROR),
                ('S', 2, 0, 0),
                ('A', 2, 0, 0),
                ('E', 2, 1, END_ERROR),
                ('S', 3, 0, 0),
                ('A', 3, 0, 0)
            ]
        );
        assert_eq!(
            decoder.abandon(),
            Some(MicEvent::End {
                stream: 3,
                next_seq: 1,
                reason: END_ERROR
            })
        );
        assert_eq!(decoder.abandon(), None, "only once");
        // The link dropped: late frames of stream 3 are stale.
        let late = fold(
            &mut decoder,
            &[audio(3, 1, &mut encoder), end(3, 2, END_RELEASE)],
        );
        assert!(late.is_empty(), "{late:?}");
        assert_eq!(decoder.stats().ignored, 2);
    }

    #[test]
    fn frames_after_end_are_stale_but_a_new_start_under_that_id_is_a_new_stream() {
        let mut encoder = State::default();
        let mut decoder = MicDecoder::new();
        let events = fold(
            &mut decoder,
            &[
                start(5),
                audio(5, 0, &mut encoder),
                end(5, 1, END_ABORT),
                audio(5, 1, &mut encoder),
                end(5, 2, END_ABORT),
                // The deck rebooted and counts from the same id again.
                start(5),
                audio(5, 0, &mut encoder),
            ],
        );
        assert_eq!(
            shape(&events),
            [
                ('S', 5, 0, 0),
                ('A', 5, 0, 0),
                ('E', 5, 1, END_ABORT),
                ('S', 5, 0, 0),
                ('A', 5, 0, 0)
            ]
        );
        assert_eq!(decoder.stats().ignored, 2);
        assert_eq!(decoder.stats().streams, 2);
    }

    #[test]
    fn unknown_kinds_short_frames_and_unsupported_streams_are_ignored() {
        let mut encoder = State::default();
        let mut decoder = MicDecoder::new();
        let events = fold(
            &mut decoder,
            &[
                vec![0x7F, 1, 2, 3],
                vec![KIND_AUDIO, 1],
                vec![],
                // MIC is host to deck.
                vec![0x10, 1, 5],
                // A stream at 8 kHz: none of it is ours to decode.
                vec![KIND_START, 6, codec::CODEC_IMA_ADPCM, 8, 20],
                audio(6, 0, &mut encoder),
                end(6, 1, END_RELEASE),
                // A stream in a codec this host does not know.
                vec![KIND_START, 7, 9, 16, 20],
                audio(7, 0, &mut encoder),
            ],
        );
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(decoder.stats().ignored, 9);
        assert_eq!(decoder.live(), None);
        // And the next supported stream is decoded as usual.
        let events = fold(&mut decoder, &[start(8), audio(8, 0, &mut encoder)]);
        assert_eq!(shape(&events), [('S', 8, 0, 0), ('A', 8, 0, 0)]);
    }

    #[test]
    fn a_small_attribute_limit_gives_shorter_frames_of_the_same_layout() {
        // ATT_MTU 23: 20-byte values carry 28 samples each.
        let per_frame = codec::samples_per_frame(20);
        assert_eq!(per_frame, 28);
        let pcm: Vec<i16> = (0..per_frame as i16).map(|i| i * 300).collect();
        let mut encoder = State::default();
        let mut out = [0u8; 20];
        let n = audio_frame(2, 0, &mut encoder, &pcm, &mut out);
        assert_eq!(n, 20);
        let mut decoder = MicDecoder::new();
        let events = fold(&mut decoder, &[start(2), out[..n].to_vec()]);
        let MicEvent::Audio { samples, .. } = &events[1] else {
            panic!("{events:?}");
        };
        assert_eq!(samples.len(), per_frame);
    }
}
