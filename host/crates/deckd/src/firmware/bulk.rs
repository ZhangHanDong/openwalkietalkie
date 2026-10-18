//! Optional recovery bulk v1. HID remains the discovery/control fallback.
use super::{
    transport::{crc32, preflight},
    APP_BASE, BONDS_END, BONDS_START,
};
use crate::hid::{PID, VID};
use nusb::transfer::{Queue, RequestBuffer};
use std::{
    collections::VecDeque,
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

const SECTOR: usize = 4096;
const HEADER: usize = 32;
const WINDOW: usize = 2;
const TIMEOUT: Duration = Duration::from_secs(10);

struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

// Called on the updater's blocking worker, independent of any async runtime.
// Dropping a timed-out queue cancels its pending nusb transfers.
fn wait_usb<T>(future: impl Future<Output = T>, timeout: Duration) -> Result<T, String> {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    let deadline = Instant::now() + timeout;
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return Ok(value);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(
                "USB bulk transfer timed out. Keep USB connected and retry from recovery.".into(),
            );
        }
        std::thread::park_timeout(remaining);
    }
}

pub(super) struct Usb {
    output: Queue<Vec<u8>>,
    input: Queue<RequestBuffer>,
    lengths: VecDeque<(usize, [u8; 4])>,
}

impl Usb {
    pub(super) fn open(serial: &str) -> Result<Self, String> {
        let info = nusb::list_devices()
            .map_err(|e| e.to_string())?
            .find(|d| {
                d.vendor_id() == VID
                    && d.product_id() == PID
                    && d.serial_number() == Some(serial)
                    && d.interfaces().any(|i| {
                        i.interface_number() == 1
                            && i.class() == 0xff
                            && i.subclass() == 0x57
                            && i.protocol() == 1
                    })
            })
            .ok_or("This deck's recovery bulk interface is unavailable.")?;
        let device = info.open().map_err(|e| e.to_string())?;
        let interface = device.claim_interface(1).map_err(|e| e.to_string())?;
        if !interface.descriptors().any(|d| {
            [0x03, 0x85].iter().all(|address| {
                d.endpoints().any(|e| {
                    e.address() == *address
                        && e.max_packet_size() == 64
                        && e.transfer_type() == nusb::transfer::EndpointType::Bulk
                })
            })
        }) {
            return Err("Unexpected recovery bulk endpoints.".into());
        }
        Ok(Self {
            output: interface.bulk_out_queue(0x03),
            input: interface.bulk_in_queue(0x85),
            lengths: VecDeque::new(),
        })
    }
}

trait Io {
    fn submit(&mut self, frame: Vec<u8>) -> Result<(), String>;
    fn receive(&mut self) -> Result<Vec<u8>, String>;
}

impl Io for Usb {
    fn submit(&mut self, frame: Vec<u8>) -> Result<(), String> {
        self.lengths
            .push_back((frame.len(), frame[16..20].try_into().unwrap()));
        self.input.submit(RequestBuffer::new(64));
        self.output.submit(frame);
        Ok(())
    }
    fn receive(&mut self) -> Result<Vec<u8>, String> {
        let (expected, token) = self
            .lengths
            .pop_front()
            .ok_or("No pending bulk transfer.")?;
        let sent = wait_usb(self.output.next_complete(), TIMEOUT)?
            .into_result()
            .map_err(|e| format!("USB bulk write failed: {e}"))?;
        if sent.actual_length() != expected {
            return Err("Incomplete USB bulk write. Retry from recovery.".into());
        }
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let reply = wait_usb(
                self.input.next_complete(),
                deadline.saturating_duration_since(Instant::now()),
            )?
            .into_result()
            .map_err(|e| format!("USB bulk reply failed: {e}"))?;
            // Closing a previous host can leave an ACK in the device FIFO.
            // Consume only CRC-valid replies from another session. Matching
            // session replies must still pass strict sequence/address checks.
            if stale_reply(&reply, token) {
                self.input.submit(RequestBuffer::new(64));
                if Instant::now() >= deadline {
                    return Err("Timed out draining old bulk acknowledgments.".into());
                }
                continue;
            }
            return Ok(reply);
        }
    }
}

fn frame(op: u8, addr: u32, seq: u32, token: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; HEADER];
    bytes[..4].copy_from_slice(b"OWTF");
    bytes[4] = op;
    bytes[5] = 1;
    bytes[6..8].copy_from_slice(&(data.len() as u16).to_le_bytes());
    bytes[8..12].copy_from_slice(&addr.to_le_bytes());
    bytes[12..16].copy_from_slice(&seq.to_le_bytes());
    bytes[16..20].copy_from_slice(&token.to_le_bytes());
    bytes[20..24].copy_from_slice(&crc32(data).to_le_bytes());
    let crc = crc32(&bytes[..24]);
    bytes[24..28].copy_from_slice(&crc.to_le_bytes());
    if op == 4 {
        bytes.extend_from_slice(data);
        bytes.resize(HEADER + data.len().div_ceil(64) * 64, 0xff);
    }
    bytes
}

fn check_reply(reply: &[u8], expected: &[u8]) -> Result<(), String> {
    if reply.len() != 24
        || &reply[..4] != b"OWTA"
        || reply[4] != expected[4]
        || reply[6..8] != [1, 0]
        || reply[8..16] != expected[12..20]
        || reply[16..20] != expected[8..12]
        || reply[20..24] != crc32(&reply[..20]).to_le_bytes()
    {
        return Err("Invalid or stale bulk acknowledgment. Retry from recovery.".into());
    }
    if reply[5] != 0 {
        return Err(format!(
            "Recovery bulk command {} failed (status {}). Retry from recovery.",
            reply[4], reply[5]
        ));
    }
    Ok(())
}

fn stale_reply(reply: &[u8], token: [u8; 4]) -> bool {
    reply.len() == 24
        && &reply[..4] == b"OWTA"
        && reply[6..8] == [1, 0]
        && reply[12..16] != token
        && reply[20..24] == crc32(&reply[..20]).to_le_bytes()
}

pub(super) fn transfer(
    usb: &mut Usb,
    image: &[u8],
    protocol: u8,
    progress: &mut dyn FnMut(u8),
) -> Result<(), String> {
    let mut random = [0; 4];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    transfer_io(
        usb,
        image,
        protocol,
        u32::from_le_bytes(random).max(1),
        progress,
    )
}

fn transfer_io(
    io: &mut impl Io,
    image: &[u8],
    protocol: u8,
    token: u32,
    mut progress: impl FnMut(u8),
) -> Result<(), String> {
    preflight(image, protocol)?;
    let begin = frame(2, image.len() as u32, 0, token, &[]);
    io.submit(begin.clone())?;
    let reply = match io.receive() {
        Ok(reply) => reply,
        Err(_) => {
            // A cancelled OUT may have ended mid-frame. BEGIN's short packet
            // flushes that fragment; one fresh BEGIN is safe before any DATA.
            // Never retry uncertain sector writes or FINISH this way.
            io.submit(begin.clone())?;
            io.receive()?
        }
    };
    check_reply(&reply, &begin)?;
    let mut sectors = image
        .chunks(SECTOR)
        .enumerate()
        .filter(|(i, _)| !(BONDS_START..BONDS_END).contains(&(APP_BASE + (i * SECTOR) as u32)));
    let mut pending = VecDeque::new();
    let mut seq = 1;
    loop {
        while pending.len() < WINDOW {
            let Some((index, sector)) = sectors.next() else {
                break;
            };
            let addr = APP_BASE + (index * SECTOR) as u32;
            let op = if sector.iter().all(|b| *b == 0xff) {
                8
            } else {
                4
            };
            let bytes = frame(op, addr, seq, token, sector);
            pending.push_back((bytes[..HEADER].to_vec(), index * SECTOR + sector.len()));
            io.submit(bytes)?;
            seq += 1;
        }
        let Some((expected, completed)) = pending.pop_front() else {
            break;
        };
        check_reply(&io.receive()?, &expected)?;
        progress((completed * 100 / image.len()).min(99) as u8);
    }
    let finish = frame(5, 0, seq, token, &[]);
    io.submit(finish.clone())?;
    check_reply(&io.receive()?, &finish)?;
    progress(100);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "writes firmware over USB; requires OWT_BULK_ARTIFACTS, OWT_BULK_BOARD and OWT_BULK_SERIAL; selected device must already be in recovery"]
    fn hardware_bulk_update() {
        let directory = std::path::PathBuf::from(std::env::var("OWT_BULK_ARTIFACTS").unwrap());
        let board = std::env::var("OWT_BULK_BOARD").unwrap();
        let serial = std::env::var("OWT_BULK_SERIAL").unwrap();
        assert!(!serial.is_empty());
        let manifest: super::super::Manifest = serde_json::from_slice(
            &std::fs::read(directory.join("firmware-manifest.json")).unwrap(),
        )
        .unwrap();
        manifest.validate().unwrap();
        let asset = &manifest.firmware.variants[&board];
        let image = std::fs::read(directory.join(&asset.name)).unwrap();
        super::super::validate_image(&image, asset, &board).unwrap();
        let mut usb = Usb::open(&serial).unwrap();
        let started = Instant::now();
        let mut last = 255;
        transfer(&mut usb, &image, manifest.firmware.protocol, &mut |p| {
            if p != last {
                println!("Bulk update: {p}%");
                last = p;
            }
        })
        .unwrap();
        println!(
            "Bulk firmware verified in {:.2} seconds",
            started.elapsed().as_secs_f64()
        );
    }
    #[derive(Default)]
    struct Fake {
        pending: VecDeque<Vec<u8>>,
        frames: Vec<Vec<u8>>,
        fail: bool,
        fail_begin_once: bool,
        max_pending: usize,
    }
    impl Io for Fake {
        fn submit(&mut self, frame: Vec<u8>) -> Result<(), String> {
            assert_eq!(frame.len() % 64, 32);
            self.pending.push_back(frame.clone());
            self.frames.push(frame);
            self.max_pending = self.max_pending.max(self.pending.len());
            Ok(())
        }
        fn receive(&mut self) -> Result<Vec<u8>, String> {
            let f = self.pending.pop_front().unwrap();
            if f[4] == 2 && self.fail_begin_once {
                self.fail_begin_once = false;
                return Err("incomplete old frame consumed BEGIN".into());
            }
            let mut r = b"OWTA".to_vec();
            r.extend_from_slice(&[f[4], u8::from(self.fail && f[4] == 4), 1, 0]);
            r.extend_from_slice(&f[12..20]);
            r.extend_from_slice(&f[8..12]);
            r.extend_from_slice(&crc32(&r).to_le_bytes());
            Ok(r)
        }
    }
    #[test]
    fn sparse_pipelined_transfer_preserves_bonds_and_checks_finish() {
        let mut image = vec![0xff; (BONDS_END - APP_BASE) as usize + 32];
        image[..4096].fill(0xa5);
        image[(BONDS_END - APP_BASE) as usize..].fill(0x5a);
        let mut io = Fake::default();
        let mut progress = Vec::new();
        transfer_io(&mut io, &image, 2, 123, |p| progress.push(p)).unwrap();
        assert_eq!(io.max_pending, 2);
        assert_eq!(io.frames.iter().filter(|f| f[4] == 4).count(), 2);
        assert!(io
            .frames
            .iter()
            .filter(|f| matches!(f[4], 4 | 8))
            .all(|f| !(BONDS_START..BONDS_END)
                .contains(&u32::from_le_bytes(f[8..12].try_into().unwrap()))));
        assert_eq!(io.frames[1][32..], image[..4096]);
        let last_data = &io.frames[io.frames.len() - 2];
        assert_eq!(last_data.len(), 96);
        assert_eq!(&last_data[32..64], &[0x5a; 32]);
        assert_eq!(progress.last(), Some(&100));
        assert!(progress[..progress.len() - 1].iter().all(|p| *p < 100));
    }
    #[test]
    fn failed_sector_never_sends_finish() {
        let mut io = Fake {
            fail: true,
            ..Default::default()
        };
        assert!(transfer_io(&mut io, &[0xa5; 8200], 2, 7, |_| {}).is_err());
        assert!(!io.frames.iter().any(|f| f[4] == 5));
    }

    #[test]
    fn retries_only_begin_after_an_old_partial_frame() {
        let mut io = Fake {
            fail_begin_once: true,
            ..Default::default()
        };
        transfer_io(&mut io, &[0xa5; 4200], 2, 7, |_| {}).unwrap();
        assert_eq!(
            io.frames.iter().map(|f| f[4]).collect::<Vec<_>>(),
            [2, 2, 4, 4, 5]
        );
    }
    #[test]
    fn rejects_stale_corrupt_and_wrong_sector_replies() {
        let f = frame(4, APP_BASE, 1, 7, &[0xa5; 32]);
        let mut io = Fake::default();
        io.submit(f.clone()).unwrap();
        let good = io.receive().unwrap();
        check_reply(&good, &f).unwrap();
        assert!(!stale_reply(&good, 7u32.to_le_bytes()));
        assert!(stale_reply(&good, 8u32.to_le_bytes()));
        for at in [4, 6, 8, 12, 16, 20] {
            let mut bad = good.clone();
            bad[at] ^= 1;
            let crc = crc32(&bad[..20]);
            bad[20..].copy_from_slice(&crc.to_le_bytes());
            if at == 20 {
                bad[20] ^= 1;
            }
            assert!(check_reply(&bad, &f).is_err());
        }
        assert!(check_reply(&good[..20], &f).is_err());
    }
    #[test]
    fn invalid_layout_never_begins_and_wait_is_bounded() {
        let mut io = Fake::default();
        assert!(transfer_io(&mut io, &[0; 1], 2, 7, |_| {}).is_err());
        assert!(io.frames.is_empty());
        assert!(wait_usb(std::future::pending::<()>(), Duration::from_millis(1)).is_err());
        assert_eq!(wait_usb(std::future::ready(42), TIMEOUT).unwrap(), 42);
    }
}
