//! Keep the selected device image stable while the conversation changes.
use super::Core;
use crate::chat_images::{self, CHUNK};
use base64::{engine::general_purpose::STANDARD, Engine};
use deck_model::{AgentId, Control, DeviceOp, Slot};
use deck_proto::HostLine;
use std::time::{Duration, Instant};

// Longer than a full PTT hold, which pauses image requests on the device.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) struct Transfer {
    agent: AgentId,
    slot: u8,
    rev: u16,
    index: u8,
    width: u16,
    height: u16,
    // Copy only the <=16.5 KiB device preview, not the much larger app PNG.
    pixels: Vec<u8>,
    last_request: Instant,
}

impl Core {
    pub(super) fn on_device_image(&mut self, slot: u8, rev: u16, index: u8, offset: u16, now: Instant) {
        let snapshot = self.store.snapshot();
        if self.device_image.as_ref().is_some_and(|t| {
            now.saturating_duration_since(t.last_request) >= IDLE_TIMEOUT
                || snapshot.agents.get(&t.agent).is_none_or(|a| a.slot != Some(Slot(t.slot)))
        }) {
            self.device_image = None;
        }
        let matches = |t: &Transfer| (t.slot, t.rev, t.index) == (slot, rev, index);
        if offset == 0 && !self.device_image.as_ref().is_some_and(matches) {
            // Resolve the image against the displayed revision exactly once.
            // Later chunks and retries use this immutable copy, even if older
            // messages roll out of history or the global preview cache evicts it.
            if let Some((agent, preview)) = snapshot.agents.values()
                .find(|a| a.slot == Some(Slot(slot)) && a.chat_rev == rev)
                .and_then(|a| a.messages.iter().flat_map(|m| &m.message.images)
                    .nth(usize::from(index)).and_then(|i| chat_images::get(&i.id))
                    .map(|p| (a.id.clone(), p)))
            {
                self.device_image = Some(Transfer {
                    agent, slot, rev, index, width: preview.width, height: preview.height,
                    pixels: preview.deck.clone(), last_request: now,
                });
            }
        }
        let mut response = HostLine::Image {
            slot, rev, index, offset, total: 0, width: 0, height: 0, data: String::new(),
        };
        if let Some(t) = self.device_image.as_mut().filter(|t| matches(t)) {
            let start = usize::from(offset);
            if start < t.pixels.len() && start.is_multiple_of(CHUNK) {
                let end = (start + CHUNK).min(t.pixels.len());
                t.last_request = now;
                response = HostLine::Image {
                    slot, rev, index, offset, total: t.pixels.len() as u16,
                    width: t.width, height: t.height,
                    data: STANDARD.encode(&t.pixels[start..end]),
                };
            }
        }
        self.send_op(DeviceOp::Control(Control::Image(response)));
    }
}
