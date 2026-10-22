//! The voice link's decisions, without the queues and drivers around them:
//! what a deck event or a host frame asks of the microphone, and which
//! speaker frames get played. Pure and `no_std`, so the host tests mount it.
use super::codec;
use crate::deck::msg::Event;
#[cfg(test)]
use crate::deck::msg::PttCtx;

/// What the microphone should do next. `host` says who asked: the host
/// (a MIC frame) or the deck itself (a PTT press or release, a lost link).
/// A press during a stream the host started takes that stream over, and the
/// host cannot end a stream a press owns: an API capture closing (MIC off)
/// must not cut off the operator who has just pressed PTT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicRequest {
    /// `mode` is `codec::MIC_ON` or `codec::MIC_LOOPBACK`; the stream ends
    /// by itself after `limit_s` seconds.
    Start { mode: u8, limit_s: u8, host: bool },
    Stop { reason: u8, host: bool },
}

/// The deck's own backstop for a PTT stream: the UI ends a press at 60 s
/// (`global::PTT_CAP_MS`), which arrives as a release; this only matters if
/// that release is lost.
pub const PTT_LIMIT_S: u8 = 62;

/// A deck event from the UI. A PTT press opens the microphone when a deck
/// host is `online` to hear what it is for; its release closes it online or
/// not, so a host that went away mid-press cannot leave the microphone open.
pub fn ptt(event: &Event, online: bool) -> Option<MicRequest> {
    let Event::Ptt { act, abort, .. } = *event else {
        return None;
    };
    if act != 0 {
        online.then_some(MicRequest::Start {
            mode: codec::MIC_ON,
            limit_s: PTT_LIMIT_S,
            host: false,
        })
    } else {
        Some(MicRequest::Stop {
            reason: if abort {
                codec::END_ABORT
            } else {
                codec::END_RELEASE
            },
            host: false,
        })
    }
}

/// Who a live stream answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    /// Started by a host MIC frame, and not taken over by a press since.
    pub host: bool,
}

/// What a request does to a live stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Live {
    /// Keep streaming; `extend_s` pushes the end out to that many seconds
    /// from now (a press taking the stream over brings its own limit).
    Continue { extend_s: Option<u8> },
    /// End it with this reason.
    End(u8),
}

impl Owner {
    pub fn request(&mut self, request: MicRequest) -> Live {
        match request {
            MicRequest::Start { host: false, limit_s, .. } => {
                let taken = self.host;
                self.host = false;
                Live::Continue {
                    extend_s: taken.then_some(limit_s),
                }
            }
            MicRequest::Start { host: true, .. } => Live::Continue { extend_s: None },
            MicRequest::Stop { host: true, .. } if !self.host => Live::Continue { extend_s: None },
            MicRequest::Stop { reason, .. } => Live::End(reason),
        }
    }
}

/// What a value written to the speaker characteristic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostFrame {
    /// A MIC request.
    Mic(MicRequest),
    /// START, AUDIO or END, for the speaker.
    Speaker,
    /// Unknown, short, or a MIC mode from a newer host: not ours to guess at.
    Ignore,
}

pub fn host_frame(frame: &codec::Frame) -> HostFrame {
    match *frame {
        codec::Frame::Mic { mode, seconds } => match mode {
            codec::MIC_OFF => HostFrame::Mic(MicRequest::Stop {
                reason: codec::END_ABORT,
                host: true,
            }),
            codec::MIC_ON | codec::MIC_LOOPBACK => HostFrame::Mic(MicRequest::Start {
                mode,
                limit_s: seconds.clamp(1, 60),
                host: true,
            }),
            _ => HostFrame::Ignore,
        },
        codec::Frame::Start { .. } | codec::Frame::Audio { .. } | codec::Frame::End { .. } => {
            HostFrame::Speaker
        }
        // LISTEN is the link's business (answered before this is asked);
        // INFO is ours to send, not to receive.
        codec::Frame::Listen { .. } | codec::Frame::Info { .. } | codec::Frame::Other => {
            HostFrame::Ignore
        }
    }
}

/// What the speaker does with a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Play,
    Drop,
    /// A START this speaker cannot play (reported once per stream); that
    /// stream's frames are dropped until its END.
    Refuse,
}

/// Which streams the speaker plays: a START for a codec, rate or frame
/// length it does not know shuts that stream out until its END, or a good
/// START with the same number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpeakerGate {
    refused: Option<u8>,
    /// A high-rate stream needs an explicit USB START; orphan frames stay 16 kHz.
    high_rate_stream: Option<u8>,
}

impl SpeakerGate {
    #[cfg(test)]
    pub fn admit(&mut self, frame: &codec::Frame) -> Verdict {
        self.admit_on(frame, false)
    }

    pub fn rate_khz(&self) -> u8 {
        if self.high_rate_stream.is_some() { 48 } else { codec::RATE_KHZ }
    }

    pub fn finish(&mut self) {
        self.high_rate_stream = None;
    }

    pub fn admit_on(&mut self, frame: &codec::Frame, usb: bool) -> Verdict {
        // BLE traffic cannot change an active USB stream's rate.
        if self.high_rate_stream.is_some() && !usb { return Verdict::Drop; }
        match *frame {
            codec::Frame::Start {
                stream,
                codec,
                rate_khz,
                frame_ms,
            } => {
                if codec == codec::CODEC_IMA_ADPCM
                    && (rate_khz == codec::RATE_KHZ
                        || (usb && rate_khz == 48))
                    && frame_ms == codec::FRAME_MS
                {
                    self.high_rate_stream = (rate_khz == 48).then_some(stream);
                    if self.refused == Some(stream) {
                        self.refused = None;
                    }
                    Verdict::Play
                } else if self.refused == Some(stream) {
                    Verdict::Drop
                } else {
                    self.refused = Some(stream);
                    Verdict::Refuse
                }
            }
            codec::Frame::Audio { stream, .. } => {
                if self.refused == Some(stream) || self.high_rate_stream.is_some_and(|id| id != stream) {
                    Verdict::Drop
                } else {
                    Verdict::Play
                }
            }
            codec::Frame::End { stream, .. } => {
                if self.high_rate_stream.is_some_and(|id| id != stream) { return Verdict::Drop; }
                self.high_rate_stream = None;
                if self.refused == Some(stream) {
                    self.refused = None;
                    Verdict::Drop
                } else {
                    Verdict::Play
                }
            }
            _ => Verdict::Drop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(act: u8, ctx: PttCtx, abort: bool) -> Event {
        Event::Ptt {
            act,
            agent: 1,
            ctx,
            card: 0,
            abort, press: 0,
        }
    }

    #[test]
    fn a_press_opens_the_microphone_only_for_a_present_host() {
        let start = Some(MicRequest::Start {
            mode: codec::MIC_ON,
            limit_s: PTT_LIMIT_S,
            host: false,
        });
        assert_eq!(ptt(&press(1, PttCtx::Talk, false), true), start);
        assert_eq!(ptt(&press(1, PttCtx::Talk, false), false), None);
    }

    #[test]
    fn a_release_always_closes_it_with_its_reason() {
        for online in [true, false] {
            assert_eq!(
                ptt(&press(0, PttCtx::Talk, false), online),
                Some(MicRequest::Stop {
                    reason: codec::END_RELEASE,
                    host: false
                })
            );
            assert_eq!(
                ptt(&press(0, PttCtx::Answer, true), online),
                Some(MicRequest::Stop {
                    reason: codec::END_ABORT,
                    host: false
                })
            );
        }
    }

    #[test]
    fn mic_requests_are_clamped_and_unknown_modes_ignored() {
        let mic = |mode, seconds| host_frame(&codec::parse(&[codec::KIND_MIC, mode, seconds]));
        assert_eq!(
            mic(codec::MIC_OFF, 9),
            HostFrame::Mic(MicRequest::Stop {
                reason: codec::END_ABORT,
                host: true
            })
        );
        assert_eq!(
            mic(codec::MIC_ON, 0),
            HostFrame::Mic(MicRequest::Start {
                mode: codec::MIC_ON,
                limit_s: 1,
                host: true
            })
        );
        assert_eq!(
            mic(codec::MIC_LOOPBACK, 200),
            HostFrame::Mic(MicRequest::Start {
                mode: codec::MIC_LOOPBACK,
                limit_s: 60,
                host: true
            })
        );
        assert_eq!(mic(3, 5), HostFrame::Ignore);
        assert_eq!(host_frame(&codec::parse(&[0x7f, 1, 2, 3])), HostFrame::Ignore);
        assert_eq!(host_frame(&codec::parse(&[codec::KIND_LISTEN, 1])), HostFrame::Ignore);
        assert_eq!(host_frame(&codec::parse(&[codec::KIND_INFO, 1, 1, 16, 20, 3])), HostFrame::Ignore);
        assert_eq!(host_frame(&codec::parse(&[codec::KIND_END, 1, 2, 0])), HostFrame::Speaker);
    }

    #[test]
    fn a_press_takes_a_host_stream_over_and_the_host_cannot_end_it() {
        let press = MicRequest::Start {
            mode: codec::MIC_ON,
            limit_s: PTT_LIMIT_S,
            host: false,
        };
        let release = MicRequest::Stop {
            reason: codec::END_RELEASE,
            host: false,
        };
        let mic_off = MicRequest::Stop {
            reason: codec::END_ABORT,
            host: true,
        };
        let mic_on = MicRequest::Start {
            mode: codec::MIC_ON,
            limit_s: 10,
            host: true,
        };
        // A host stream: the host may end it, a second MIC on changes nothing.
        let mut owner = Owner { host: true };
        assert_eq!(owner.request(mic_on), Live::Continue { extend_s: None });
        assert_eq!(owner.request(mic_off), Live::End(codec::END_ABORT));
        // A press takes it over (with the press's own limit)...
        let mut owner = Owner { host: true };
        assert_eq!(
            owner.request(press),
            Live::Continue {
                extend_s: Some(PTT_LIMIT_S)
            }
        );
        // ...so the host's MIC off no longer ends it; the release does.
        assert_eq!(owner.request(mic_off), Live::Continue { extend_s: None });
        assert_eq!(owner.request(release), Live::End(codec::END_RELEASE));
        // A press's own stream: a second press keeps its limit.
        let mut owner = Owner { host: false };
        assert_eq!(owner.request(press), Live::Continue { extend_s: None });
        assert_eq!(owner.request(mic_off), Live::Continue { extend_s: None });
        assert_eq!(owner.request(release), Live::End(codec::END_RELEASE));
    }

    fn start(stream: u8, codec_id: u8) -> [u8; 5] {
        [codec::KIND_START, stream, codec_id, codec::RATE_KHZ, codec::FRAME_MS]
    }

    #[test]
    fn the_speaker_refuses_a_stream_it_cannot_decode_until_its_end() {
        let mut gate = SpeakerGate::default();
        let audio = [codec::KIND_AUDIO, 4, 0, 0, 0, 0, 0x11];
        let end = [codec::KIND_END, 4, 1, codec::END_RELEASE];
        assert_eq!(gate.admit(&codec::parse(&start(4, 9))), Verdict::Refuse);
        // Reported once per stream.
        assert_eq!(gate.admit(&codec::parse(&start(4, 9))), Verdict::Drop);
        assert_eq!(gate.admit(&codec::parse(&audio)), Verdict::Drop);
        assert_eq!(gate.admit(&codec::parse(&end)), Verdict::Drop);
        // The refusal ends with the stream.
        assert_eq!(gate.admit(&codec::parse(&audio)), Verdict::Play);
        // A good START for the same number lifts it too.
        assert_eq!(gate.admit(&codec::parse(&start(4, 9))), Verdict::Refuse);
        assert_eq!(gate.admit(&codec::parse(&start(4, codec::CODEC_IMA_ADPCM))), Verdict::Play);
        assert_eq!(gate.admit(&codec::parse(&audio)), Verdict::Play);
        assert_eq!(gate.admit(&codec::parse(&end)), Verdict::Play);
    }

    #[test]
    fn high_rate_audio_requires_an_explicit_usb_start() {
        let start = codec::parse(&[codec::KIND_START, 7, codec::CODEC_IMA_ADPCM, 48, 20]);
        let audio = codec::parse(&[codec::KIND_AUDIO, 7, 0, 0, 0, 0, 0x11]);
        let mut gate = SpeakerGate::default();
        assert_eq!(gate.admit_on(&start, false), Verdict::Refuse);
        assert_eq!(gate.rate_khz(), 16);
        assert_eq!(gate.admit_on(&audio, false), Verdict::Drop);
        {
            assert_eq!(gate.admit_on(&start, true), Verdict::Play);
            assert_eq!(gate.rate_khz(), 48);
            assert_eq!(gate.admit_on(&audio, false), Verdict::Drop);
            assert_eq!(gate.admit_on(&audio, true), Verdict::Play);
            assert_eq!(gate.admit_on(&codec::parse(&[codec::KIND_AUDIO, 8, 0, 0, 0, 0, 0x11]), true), Verdict::Drop);
            assert_eq!(gate.admit_on(&codec::parse(&[codec::KIND_END, 8, 0, 0]), true), Verdict::Drop);
            assert_eq!(gate.rate_khz(), 48);
            assert_eq!(gate.admit_on(&codec::parse(&[codec::KIND_END, 7, 0, 0]), true), Verdict::Play);
            assert_eq!(gate.rate_khz(), 16);
            assert_eq!(gate.admit_on(&audio, false), Verdict::Play);
            gate.admit_on(&start, true);
            gate.finish();
            assert_eq!(gate.rate_khz(), 16);
        }
        for rate in [24, 32, 44, 96] {
            let mut gate = SpeakerGate::default();
            assert_eq!(gate.admit_on(&codec::parse(&[codec::KIND_START, 7, 1, rate, 20]), true), Verdict::Refuse);
        }
    }
}
