//! The CoreBluetooth side of the link: a central manager on a private
//! serial dispatch queue, the delegate it calls, and the engine that turns
//! those callbacks into [`attach`] inputs and [`Shared`] updates.
//!
//! **Why a private queue.** deckd's main thread is a CFRunLoop that belongs
//! to the speech engines (`audio::mainloop`): AVSpeech and SFSpeechRecognizer
//! deliver there and nowhere else. A central manager created without a queue
//! would put its callbacks — fifty mic notifications a second while someone
//! talks — on that same loop. So the manager gets its own serial queue, and
//! every CoreBluetooth object (the manager, the delegate, the peripheral,
//! its service and characteristics) is created, called and released there.
//!
//! **How the rest of deckd reaches it.** Not by sharing objects. The handle
//! and the engine share [`Shared`] — plain Rust state behind locks and
//! crossbeam channels — and anything that has to happen on the queue is
//! posted there as a closure: a drain when a frame was queued, a tick when
//! the retry timer fires, a teardown when the last handle goes. Delegate
//! callbacks are on the queue already and run their closure at once.
//!
//! [`OnQueue`] is where that rule is spelled out to the compiler. It is
//! `Send + Sync` so it can be named from any thread, and it hands out its
//! contents only after libdispatch confirms the caller is running on its
//! queue. Inside it, `Link` keeps the engine in a `RefCell` and an inbox of
//! jobs, so a callback that arrives while the engine is busy (CoreBluetooth
//! does not call back synchronously today, but nothing promises it) waits
//! its turn rather than panicking or being lost.
use super::attach::{self, Action, Attach, Input, Radio, Reach, Report};
use super::link::Transport;
use super::cb::{
    CBAdvertisementDataLocalNameKey, CBCentralManager, CBCentralManagerDelegate,
    CBCentralManagerOptionShowPowerAlertKey, CBCharacteristic, CBManager, CBPeripheral,
    CBPeripheralDelegate, CBService, CBUUID, PERIPHERAL_DISCONNECTED,
    PROPERTY_WRITE_WITHOUT_RESPONSE, WRITE_WITHOUT_RESPONSE,
};
use super::link::{LinkState, Peer, Shared};
use super::{
    hex, Info, AUDIO_SERVICE_UUID, BATTERY_SERVICE_UUID, INFO_UUID, MIC_UUID, SPEAKER_UUID,
};
use crossbeam_channel::{Receiver, Sender};
use dispatch2::{
    DispatchAutoReleaseFrequency, DispatchQueue, DispatchQueueAttr, DispatchRetained, DispatchTime,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, Message};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSNumber, NSString};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// The queue's label, as crash reports and `sample` show it.
const QUEUE_LABEL: &str = "deckd-ble";

// ---------------------------------------------------------------------------
// OnQueue
// ---------------------------------------------------------------------------

extern "C" {
    /// libdispatch, in libSystem. dispatch2 only exposes it as a destructor
    /// hook; the tag below needs a context value of its own.
    fn dispatch_queue_set_specific(
        queue: &DispatchQueue,
        key: NonNull<c_void>,
        context: *mut c_void,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    );
}

/// The queue-specific key [`OnQueue`] tags its queue under; only its
/// address matters.
static QUEUE_TAG: u8 = 0;

fn tag_key() -> NonNull<c_void> {
    NonNull::from(&QUEUE_TAG).cast()
}

/// A value that lives on one serial dispatch queue.
///
/// `get` returns the value only when libdispatch says the caller runs on
/// that queue (the queue is tagged with this value's address, and
/// `dispatch_get_specific` reads the tag of the queue the caller is on).
/// Off the queue it panics: a bug, never undefined behaviour.
pub(super) struct OnQueue<T> {
    queue: DispatchRetained<DispatchQueue>,
    value: ManuallyDrop<T>,
}

// SAFETY: `value` is reached only through `get`, which first checks that
// the caller runs on `queue`. A serial queue runs one work item at a time
// and orders each after the last, so every access to `value` happens-after
// the previous one, whichever worker thread runs it. `Drop` releases
// `value` on the queue or not at all.
unsafe impl<T> Send for OnQueue<T> {}
// SAFETY: as above; a shared reference gives no access off the queue.
unsafe impl<T> Sync for OnQueue<T> {}

impl<T> OnQueue<T> {
    fn new(queue: DispatchRetained<DispatchQueue>, value: T) -> Arc<OnQueue<T>> {
        let this = Arc::new(OnQueue {
            queue,
            value: ManuallyDrop::new(value),
        });
        // SAFETY: the key is a static's address and the context is only
        // ever compared, never dereferenced, so neither needs a destructor.
        // Each queue made here carries exactly one `OnQueue`.
        unsafe {
            dispatch_queue_set_specific(
                &this.queue,
                tag_key(),
                Arc::as_ptr(&this).cast_mut().cast(),
                None,
            );
        }
        this
    }

    fn is_current(&self) -> bool {
        // SAFETY: reads the current queue's context for our key; any thread
        // may ask.
        let tag = unsafe { dispatch2::dispatch_get_specific(tag_key()) };
        std::ptr::eq(tag.cast_const().cast::<OnQueue<T>>(), self)
    }

    fn get(&self) -> &T {
        assert!(self.is_current(), "deckd-ble state reached off its queue");
        &self.value
    }
}

impl<T> Drop for OnQueue<T> {
    fn drop(&mut self) {
        if self.is_current() {
            // SAFETY: the one place `value` is dropped, on its queue.
            unsafe { ManuallyDrop::drop(&mut self.value) };
        } else {
            // Releasing CoreBluetooth objects off their queue is the one
            // thing this type exists to prevent. Leak instead (the handle
            // tears down on the queue, so this is a bug report, not a path).
            tracing::error!("deckd-ble state dropped off its queue; leaking it");
        }
        // SAFETY: clears the tag; no context, no destructor.
        unsafe {
            dispatch_queue_set_specific(&self.queue, tag_key(), std::ptr::null_mut(), None);
        }
    }
}

/// A serial queue whose work items each get an autorelease pool, so the
/// objects CoreBluetooth autoreleases in its callbacks go when the callback
/// does rather than piling up on a worker thread.
fn serial_queue(label: &str) -> DispatchRetained<DispatchQueue> {
    let attr = DispatchQueueAttr::with_autorelease_frequency(
        DispatchQueueAttr::SERIAL,
        DispatchAutoReleaseFrequency::WORK_ITEM,
    );
    DispatchQueue::new(label, Some(&attr))
}

/// A central manager whose callbacks all run on `queue`, and which never
/// asks the system to pop up "turn Bluetooth on" (a daemon must not).
///
/// The caller makes it on `queue`, makes every call on it there, and keeps
/// `delegate` alive for as long as the manager (which holds it weakly).
fn central_manager(
    queue: &DispatchQueue,
    delegate: &ProtocolObject<dyn CBCentralManagerDelegate>,
) -> Retained<CBCentralManager> {
    // SAFETY: reading a framework constant.
    let key = unsafe { CBCentralManagerOptionShowPowerAlertKey };
    let no = NSNumber::new_bool(false);
    let value: &AnyObject = &no;
    let options = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[value]);
    // SAFETY: the callbacks go to `queue`, where (per this function's
    // contract) the caller makes every call on the manager and on what it
    // hands out; the caller owns the delegate for the manager's life; the
    // one option is the BOOL NSNumber its key documents.
    unsafe {
        CBCentralManager::initWithDelegate_queue_options(
            CBCentralManager::alloc(),
            Some(delegate),
            Some(queue),
            Some(&options),
        )
    }
}

// ---------------------------------------------------------------------------
// The handle's side
// ---------------------------------------------------------------------------

/// What [`super::BleAudio`] holds on macOS.
pub(crate) struct Backend {
    queue: DispatchRetained<DispatchQueue>,
    /// `Some` until `Drop` moves it onto the queue.
    link: Option<Arc<OnQueue<Link>>>,
    shared: Arc<Shared>,
}

impl Backend {
    /// The link for a deck that goes by `name`, attaching as far as `reach`
    /// allows.
    pub(crate) fn spawn(shared: Arc<Shared>, name: &str, reach: Reach) -> Backend {
        let queue = serial_queue(QUEUE_LABEL);
        let link = OnQueue::new(queue.clone(), Link::default());
        let weak = Arc::downgrade(&link);
        let (engine_queue, engine_shared) = (queue.clone(), shared.clone());
        let name = name.to_string();
        // The manager and its delegate are made on the queue they live on.
        queue.exec_async(move || {
            if let Some(link) = weak.upgrade() {
                let engine = Engine::new(
                    engine_queue,
                    Arc::downgrade(&link),
                    engine_shared,
                    name,
                    reach,
                );
                link.get().install(engine);
            }
        });
        Backend {
            queue,
            link: Some(link),
            shared,
        }
    }

    /// Frames are waiting: drain on the queue (once, however many were
    /// queued before it runs).
    pub(crate) fn kick(&self) {
        let Some(link) = &self.link else { return };
        if self.shared.drain_posted.swap(true, Ordering::AcqRel) {
            return;
        }
        let link = Arc::downgrade(link);
        let shared = self.shared.clone();
        self.queue.exec_async(move || {
            shared.drain_posted.store(false, Ordering::Release);
            if let Some(link) = link.upgrade() {
                link.get().run(Engine::drain);
            }
        });
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        // The strong reference moves onto the queue, so the engine — and
        // every CoreBluetooth object in it — is released there.
        if let Some(link) = self.link.take() {
            self.queue.exec_async(move || {
                link.get().shutdown();
                drop(link);
            });
        }
    }
}

// ---------------------------------------------------------------------------
// The queue's side
// ---------------------------------------------------------------------------

/// The engine (once made) and the jobs for it: everything on the queue.
type Link = Inbox<Engine>;

/// One thing to do with the engine, on its queue.
type Job<E> = Box<dyn FnOnce(&mut E)>;

/// An engine that is made on its queue after the first callbacks may
/// already be arriving, and an inbox that keeps those (and any job posted
/// from inside another job) in order until it can run them. Generic over
/// the engine so its rules are tested without a radio.
struct Inbox<E> {
    engine: RefCell<Option<E>>,
    inbox: RefCell<VecDeque<Job<E>>>,
    /// Torn down: jobs are dropped, not queued.
    closed: Cell<bool>,
}

impl<E> Default for Inbox<E> {
    fn default() -> Self {
        Inbox {
            engine: RefCell::new(None),
            inbox: RefCell::new(VecDeque::new()),
            closed: Cell::new(false),
        }
    }
}

impl<E> Inbox<E> {
    /// The engine is made; anything that arrived while it was being made
    /// (a callback from inside the manager's initializer) runs now.
    fn install(&self, engine: E) {
        *self.engine.borrow_mut() = Some(engine);
        self.pump();
    }

    /// Run `job` on the engine: now, or — when this call came from inside
    /// another job, or before the engine exists — as soon as it can, in
    /// order.
    fn run(&self, job: impl FnOnce(&mut E) + 'static) {
        if self.closed.get() {
            return;
        }
        self.inbox.borrow_mut().push_back(Box::new(job));
        self.pump();
    }

    fn pump(&self) {
        let Ok(mut slot) = self.engine.try_borrow_mut() else {
            return; // the job running further up the stack drains the inbox
        };
        let Some(engine) = slot.as_mut() else {
            return; // not made yet: `install` drains it
        };
        loop {
            let next = self.inbox.borrow_mut().pop_front();
            let Some(job) = next else { break };
            // A panic must not unwind into CoreBluetooth or libdispatch.
            if std::panic::catch_unwind(AssertUnwindSafe(|| job(engine))).is_err() {
                tracing::error!("a CoreBluetooth callback panicked");
            }
        }
    }

    /// Stop taking jobs, drop the ones waiting, and hand the engine back
    /// (unless a job is running it right now).
    fn close(&self) -> Option<E> {
        self.closed.set(true);
        self.inbox.borrow_mut().clear();
        self.engine
            .try_borrow_mut()
            .ok()
            .and_then(|mut slot| slot.take())
    }
}

impl Inbox<Engine> {
    fn shutdown(&self) {
        if let Some(mut engine) = self.close() {
            engine.shutdown();
        }
    }
}

/// The UUIDs, made once.
struct Ids {
    service: Retained<CBUUID>,
    mic: Retained<CBUUID>,
    speaker: Retained<CBUUID>,
    info: Retained<CBUUID>,
    battery: Retained<CBUUID>,
}

impl Ids {
    fn new() -> Ids {
        let uuid = |s: &str| CBUUID::UUIDWithString(&NSString::from_str(s));
        Ids {
            service: uuid(AUDIO_SERVICE_UUID),
            mic: uuid(MIC_UUID),
            speaker: uuid(SPEAKER_UUID),
            info: uuid(INFO_UUID),
            battery: uuid(BATTERY_SERVICE_UUID),
        }
    }
}

/// The peripheral being attached, and what discovery found on it.
struct Candidate {
    peripheral: Retained<CBPeripheral>,
    service: Option<Retained<CBService>>,
    mic: Option<Retained<CBCharacteristic>>,
    speaker: Option<Retained<CBCharacteristic>>,
    info: Option<Retained<CBCharacteristic>>,
}

impl Candidate {
    fn new(peripheral: Retained<CBPeripheral>) -> Candidate {
        Candidate {
            peripheral,
            service: None,
            mic: None,
            speaker: None,
            info: None,
        }
    }
}

fn peer_of(peripheral: &CBPeripheral) -> Peer {
    Peer {
        name: peripheral.name().map(|n| n.to_string()),
        id: peripheral.identifier().to_string(),
        transport: Transport::Ble,
    }
}

/// A callback's error, or what happened when there was none.
fn describe(error: Option<&NSError>, otherwise: &str) -> String {
    error.map_or_else(|| otherwise.to_string(), |e| e.to_string())
}

struct Engine {
    queue: DispatchRetained<DispatchQueue>,
    link: Weak<OnQueue<Link>>,
    shared: Arc<Shared>,
    central: Retained<CBCentralManager>,
    /// The central and the peripheral hold it weakly; this is its owner.
    delegate: Retained<Delegate>,
    ids: Ids,
    /// The GAP name the deck goes by.
    name: String,
    attach: Attach,
    candidate: Option<Candidate>,
    radio: Option<Radio>,
    /// The last problem logged: a retry that fails the same way every two
    /// seconds says so once.
    problem: Option<String>,
}

impl Engine {
    /// On the queue.
    fn new(
        queue: DispatchRetained<DispatchQueue>,
        link: Weak<OnQueue<Link>>,
        shared: Arc<Shared>,
        name: String,
        reach: Reach,
    ) -> Engine {
        let delegate = Delegate::new(link.clone());
        let central = central_manager(&queue, ProtocolObject::from_ref(&*delegate));
        Engine {
            queue,
            link,
            shared,
            central,
            delegate,
            ids: Ids::new(),
            attach: Attach::reaching(&name, reach),
            name,
            candidate: None,
            radio: None,
            problem: None,
        }
    }

    fn feed(&mut self, input: Input) {
        for action in self.attach.step(Instant::now(), input) {
            self.act(action);
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Find => self.find(),
            Action::Scan => {
                tracing::debug!(name = %self.name, "scanning");
                self.central
                    .scanForPeripheralsWithServices_options(None, None);
            }
            Action::StopScan => self.central.stopScan(),
            Action::Connect => self.connect(),
            Action::Release => self.release(),
            Action::DiscoverServices => {
                if let Some(c) = &self.candidate {
                    let services = NSArray::from_slice(&[&*self.ids.service]);
                    c.peripheral.discoverServices(Some(&services));
                }
            }
            Action::DiscoverCharacteristics => {
                if let Some(c) = &self.candidate {
                    if let Some(service) = &c.service {
                        let wanted = NSArray::from_slice(&[
                            &*self.ids.mic,
                            &*self.ids.speaker,
                            &*self.ids.info,
                        ]);
                        c.peripheral
                            .discoverCharacteristics_forService(Some(&wanted), service);
                    }
                }
            }
            Action::ReadInfo => {
                if let Some(c) = &self.candidate {
                    if let Some(info) = &c.info {
                        c.peripheral.readValueForCharacteristic(info);
                    }
                }
            }
            Action::Subscribe => {
                if let Some(c) = &self.candidate {
                    if let Some(mic) = &c.mic {
                        c.peripheral.setNotifyValue_forCharacteristic(true, mic);
                    }
                }
            }
            Action::Report(report) => self.report(report),
            Action::Problem(why) => self.problem(why),
            Action::Arm => self.arm(),
        }
    }

    /// A deck the system already holds: by the audio service, then by the
    /// Battery service and the name.
    fn find(&mut self) {
        let by_service = self.connected_with(&self.ids.service, None);
        let found = match by_service {
            Some(p) => Some((p, "connected, with the audio service")),
            None => self
                .connected_with(&self.ids.battery, Some(&self.name))
                .map(|p| (p, "connected, by name")),
        };
        match found {
            Some((peripheral, via)) => {
                tracing::debug!(peer = %peer_of(&peripheral), via, "found the deck");
                self.candidate = Some(Candidate::new(peripheral));
                self.feed(Input::Found);
            }
            None => self.feed(Input::NotFound),
        }
    }

    fn connected_with(
        &self,
        service: &CBUUID,
        name: Option<&str>,
    ) -> Option<Retained<CBPeripheral>> {
        let services = NSArray::from_slice(&[service]);
        self.central
            .retrieveConnectedPeripheralsWithServices(&services)
            .iter()
            .find(|p| name.is_none_or(|n| p.name().is_some_and(|got| got.to_string() == n)))
    }

    fn connect(&mut self) {
        let Some(c) = &self.candidate else { return };
        c.peripheral
            .setDelegate(Some(ProtocolObject::from_ref(&*self.delegate)));
        self.central.connectPeripheral_options(&c.peripheral, None);
        let peer = peer_of(&c.peripheral);
        tracing::debug!(%peer, "connecting");
        self.shared.set_peer(Transport::Ble, Some(peer));
    }

    fn release(&mut self) {
        let Some(c) = self.candidate.take() else {
            return;
        };
        c.peripheral.setDelegate(None);
        if c.peripheral.state() != PERIPHERAL_DISCONNECTED {
            // Our claim only: a deck the system holds for HID stays up.
            self.central.cancelPeripheralConnection(&c.peripheral);
        }
        self.shared.set_peer(Transport::Ble, None);
    }

    fn report(&mut self, report: Report) {
        let state = match report {
            Report::Detached => LinkState::Detached,
            Report::Attaching => LinkState::Attaching,
            Report::Ready { info } => {
                let Some(c) = &self.candidate else { return };
                let max_write = c
                    .peripheral
                    .maximumWriteValueLengthForType(WRITE_WITHOUT_RESPONSE);
                let described = info.map_or_else(
                    || "unreadable".to_string(),
                    |bytes| format!("{} ({})", hex(&bytes), Info::from_bytes(bytes)),
                );
                tracing::info!(
                    peer = %peer_of(&c.peripheral),
                    max_write,
                    info = %described,
                    "deck audio link ready"
                );
                self.problem = None;
                LinkState::Ready { max_write, info }
            }
        };
        if state == LinkState::Detached {
            tracing::debug!("deck audio link detached");
        }
        self.shared.set_state(Transport::Ble, state);
    }

    fn problem(&mut self, why: String) {
        if self.problem.as_deref() == Some(why.as_str()) {
            tracing::debug!(%why, "deck audio link not ready (again)");
        } else {
            tracing::info!(%why, "deck audio link not ready");
        }
        self.problem = Some(why.clone());
        self.shared.set_problem(Transport::Ble, Some(why));
    }

    fn arm(&self) {
        let link = self.link.clone();
        let when = DispatchTime::try_from(attach::RETRY).unwrap_or(DispatchTime::NOW);
        let _ = self.queue.after(when, move || {
            if let Some(link) = link.upgrade() {
                link.get().run(|engine| {
                    tracing::trace!(phase = ?engine.attach.phase(), "retry tick");
                    engine.feed(Input::Tick);
                });
            }
        });
    }

    fn is_candidate(&self, peripheral: &CBPeripheral) -> bool {
        self.candidate
            .as_ref()
            .is_some_and(|c| &*c.peripheral == peripheral)
    }

    // --- callbacks -------------------------------------------------------

    fn on_radio(&mut self, radio: Radio) {
        if self.radio != Some(radio) && radio == Radio::On {
            tracing::info!("Bluetooth is on");
        }
        // The grant can only have moved with the state (an unauthorized
        // manager reports `Unauthorized`), so this is when to read it.
        self.shared
            .set_bluetooth(super::link::bluetooth_permission(CBManager::authorization()));
        self.radio = Some(radio);
        self.feed(Input::Radio(radio));
    }

    fn on_discovered(&mut self, peripheral: Retained<CBPeripheral>, name: Option<String>) {
        if name.as_deref() != Some(self.name.as_str()) || !self.attach.wants_candidate() {
            return;
        }
        tracing::debug!(peer = %peer_of(&peripheral), "found the deck advertising");
        self.candidate = Some(Candidate::new(peripheral));
        self.feed(Input::Found);
    }

    fn on_connected(&mut self, peripheral: &CBPeripheral) {
        if self.is_candidate(peripheral) {
            tracing::debug!("connected; discovering the audio service");
            self.feed(Input::Connected);
        }
    }

    fn on_connect_failed(&mut self, peripheral: &CBPeripheral, why: String) {
        if self.is_candidate(peripheral) {
            self.feed(Input::ConnectFailed(format!("could not connect: {why}")));
        }
    }

    fn on_disconnected(&mut self, peripheral: &CBPeripheral, why: String) {
        if self.is_candidate(peripheral) {
            self.feed(Input::Disconnected(format!("disconnected: {why}")));
        }
    }

    fn on_services(&mut self, peripheral: &CBPeripheral, error: Option<String>) {
        if !self.is_candidate(peripheral) {
            return;
        }
        let input = match error {
            Some(e) => Input::Services(Err(format!("service discovery failed: {e}"))),
            None => {
                let service = peripheral
                    .services()
                    .and_then(|all| all.iter().find(|s| *s.UUID() == *self.ids.service));
                match (service, self.candidate.as_mut()) {
                    (Some(service), Some(c)) => {
                        tracing::debug!("found the audio service; discovering its characteristics");
                        c.service = Some(service);
                        Input::Services(Ok(()))
                    }
                    _ => Input::Services(Err(format!(
                        "{} has no audio service (firmware without voice?)",
                        peer_of(peripheral)
                    ))),
                }
            }
        };
        self.feed(input);
    }

    fn on_characteristics(
        &mut self,
        peripheral: &CBPeripheral,
        service: &CBService,
        error: Option<String>,
    ) {
        if !self.is_candidate(peripheral) || *service.UUID() != *self.ids.service {
            return;
        }
        if let Some(e) = error {
            self.feed(Input::Characteristics(Err(format!(
                "characteristic discovery failed: {e}"
            ))));
            return;
        }
        let all = service
            .characteristics()
            .map(|a| a.to_vec())
            .unwrap_or_default();
        let find = |id: &CBUUID| all.iter().find(|c| *c.UUID() == *id).cloned();
        let (mic, speaker, info) = (
            find(&self.ids.mic),
            find(&self.ids.speaker),
            find(&self.ids.info),
        );
        let missing: Vec<&str> = [
            ("mic", mic.is_none()),
            ("speaker", speaker.is_none()),
            ("info", info.is_none()),
        ]
        .iter()
        .filter(|(_, gone)| *gone)
        .map(|(name, _)| *name)
        .collect();
        if let Some(speaker) = &speaker {
            if speaker.properties() & PROPERTY_WRITE_WITHOUT_RESPONSE == 0 {
                tracing::warn!("the speaker characteristic does not take writes without response");
            }
        }
        if let Some(c) = self.candidate.as_mut() {
            (c.mic, c.speaker, c.info) = (mic, speaker, info);
        }
        let input = if missing.is_empty() {
            tracing::debug!("found mic, speaker and info; reading info, subscribing to the mic");
            Input::Characteristics(Ok(()))
        } else {
            Input::Characteristics(Err(format!(
                "the audio service has no {} characteristic",
                missing.join(", ")
            )))
        };
        self.feed(input);
    }

    fn on_value(
        &mut self,
        peripheral: &CBPeripheral,
        characteristic: &CBCharacteristic,
        value: Option<Vec<u8>>,
        error: Option<String>,
    ) {
        let Some(c) = self
            .candidate
            .as_ref()
            .filter(|c| &*c.peripheral == peripheral)
        else {
            return;
        };
        if c.mic.as_deref() == Some(characteristic) {
            match (value, error) {
                (Some(value), None) => self.shared.notify(Transport::Ble, &value),
                (_, error) => tracing::debug!(?error, "mic notification without a value"),
            }
        } else if c.info.as_deref() == Some(characteristic) {
            let info = match (&value, &error) {
                (Some(v), None) if v.len() >= 5 => Some([v[0], v[1], v[2], v[3], v[4]]),
                _ => {
                    tracing::warn!(
                        ?error,
                        value = %value.as_deref().map(hex).unwrap_or_default(),
                        "the info characteristic did not read as five bytes"
                    );
                    None
                }
            };
            tracing::debug!(info = %info.map(|b| hex(&b)).unwrap_or_default(), "info read");
            self.feed(Input::Info(info));
        }
    }

    fn on_notification_state(
        &mut self,
        peripheral: &CBPeripheral,
        characteristic: &CBCharacteristic,
        notifying: bool,
        error: Option<String>,
    ) {
        let is_mic = self
            .candidate
            .as_ref()
            .filter(|c| &*c.peripheral == peripheral)
            .is_some_and(|c| c.mic.as_deref() == Some(characteristic));
        if !is_mic {
            return;
        }
        let input = match error {
            Some(e) => Input::Notifying(Err(format!("could not subscribe to the microphone: {e}"))),
            None if notifying => {
                tracing::debug!("mic notifications on");
                Input::Notifying(Ok(()))
            }
            None => Input::Notifying(Err("microphone notifications were turned off".into())),
        };
        self.feed(input);
    }

    fn on_services_modified(
        &mut self,
        peripheral: &CBPeripheral,
        invalidated: &[Retained<CBService>],
    ) {
        if self.is_candidate(peripheral)
            && invalidated.iter().any(|s| *s.UUID() == *self.ids.service)
        {
            tracing::info!("the deck changed its audio service; discovering it again");
            self.feed(Input::ServicesChanged);
        }
    }

    /// Write queued frames while CoreBluetooth takes them.
    fn drain(&mut self) {
        let Some(c) = &self.candidate else { return };
        let Some(speaker) = &c.speaker else { return };
        let peripheral = &c.peripheral;
        self.shared.drain_for(
            Transport::Ble,
            || peripheral.canSendWriteWithoutResponse(),
            |frame| {
                let data = NSData::with_bytes(frame);
                peripheral.writeValue_forCharacteristic_type(
                    &data,
                    speaker,
                    WRITE_WITHOUT_RESPONSE,
                );
            },
        );
    }

    fn shutdown(&mut self) {
        if self.central.isScanning() {
            self.central.stopScan();
        }
        self.release();
        self.central.setDelegate(None);
        self.shared.set_state(Transport::Ble, LinkState::Detached);
    }
}

// ---------------------------------------------------------------------------
// The delegate
// ---------------------------------------------------------------------------

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Delegate` does
    // not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[ivars = Weak<OnQueue<Link>>]
    struct Delegate;

    // SAFETY: NSObjectProtocol asks nothing NSObject does not already do.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: each signature is CBCentralManager.h's, and the protocol's one
    // required method is implemented.
    unsafe impl CBCentralManagerDelegate for Delegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn central_did_update_state(&self, central: &CBCentralManager) {
            let radio = Radio::from_raw(central.state());
            self.post(move |engine| engine.on_radio(radio));
        }

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn central_did_discover(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            advertisement: &NSDictionary<NSString, AnyObject>,
            _rssi: &NSNumber,
        ) {
            let name = advertised_name(peripheral, advertisement);
            let peripheral = peripheral.retain();
            self.post(move |engine| engine.on_discovered(peripheral, name));
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn central_did_connect(&self, _central: &CBCentralManager, peripheral: &CBPeripheral) {
            let peripheral = peripheral.retain();
            self.post(move |engine| engine.on_connected(&peripheral));
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn central_did_fail_to_connect(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            let why = describe(error, "no reason given");
            let peripheral = peripheral.retain();
            self.post(move |engine| engine.on_connect_failed(&peripheral, why));
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn central_did_disconnect(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            let why = describe(error, "connection released");
            let peripheral = peripheral.retain();
            self.post(move |engine| engine.on_disconnected(&peripheral, why));
        }
    }

    // SAFETY: each signature is CBPeripheral.h's; the protocol has no
    // required methods.
    unsafe impl CBPeripheralDelegate for Delegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn peripheral_did_discover_services(&self, peripheral: &CBPeripheral, error: Option<&NSError>) {
            let error = error.map(|e| e.to_string());
            let peripheral = peripheral.retain();
            self.post(move |engine| engine.on_services(&peripheral, error));
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn peripheral_did_discover_characteristics(
            &self,
            peripheral: &CBPeripheral,
            service: &CBService,
            error: Option<&NSError>,
        ) {
            let error = error.map(|e| e.to_string());
            let (peripheral, service) = (peripheral.retain(), service.retain());
            self.post(move |engine| engine.on_characteristics(&peripheral, &service, error));
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn peripheral_did_update_value(
            &self,
            peripheral: &CBPeripheral,
            characteristic: &CBCharacteristic,
            error: Option<&NSError>,
        ) {
            // Read now: the next notification replaces `value`.
            let value = characteristic.value().map(|data| data.to_vec());
            let error = error.map(|e| e.to_string());
            let (peripheral, characteristic) = (peripheral.retain(), characteristic.retain());
            self.post(move |engine| engine.on_value(&peripheral, &characteristic, value, error));
        }

        #[unsafe(method(peripheral:didUpdateNotificationStateForCharacteristic:error:))]
        fn peripheral_did_update_notification_state(
            &self,
            peripheral: &CBPeripheral,
            characteristic: &CBCharacteristic,
            error: Option<&NSError>,
        ) {
            let notifying = characteristic.isNotifying();
            let error = error.map(|e| e.to_string());
            let (peripheral, characteristic) = (peripheral.retain(), characteristic.retain());
            self.post(move |engine| {
                engine.on_notification_state(&peripheral, &characteristic, notifying, error);
            });
        }

        #[unsafe(method(peripheralIsReadyToSendWriteWithoutResponse:))]
        fn peripheral_is_ready_to_send(&self, peripheral: &CBPeripheral) {
            let peripheral = peripheral.retain();
            self.post(move |engine| {
                if engine.is_candidate(&peripheral) {
                    engine.drain();
                }
            });
        }

        #[unsafe(method(peripheral:didModifyServices:))]
        fn peripheral_did_modify_services(
            &self,
            peripheral: &CBPeripheral,
            invalidated: &NSArray<CBService>,
        ) {
            let invalidated = invalidated.to_vec();
            let peripheral = peripheral.retain();
            self.post(move |engine| engine.on_services_modified(&peripheral, &invalidated));
        }
    }
);

impl Delegate {
    fn new(link: Weak<OnQueue<Link>>) -> Retained<Delegate> {
        let this = Delegate::alloc().set_ivars(link);
        // SAFETY: NSObject's designated initializer, on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }

    /// Run `job` on the engine, if the link still has one. Delegate
    /// callbacks arrive on the link's queue, which is what `get` checks.
    fn post(&self, job: impl FnOnce(&mut Engine) + 'static) {
        if let Some(link) = self.ivars().upgrade() {
            link.get().run(job);
        }
    }
}

/// The advertised local name, else the cached GAP name.
fn advertised_name(
    peripheral: &CBPeripheral,
    advertisement: &NSDictionary<NSString, AnyObject>,
) -> Option<String> {
    // SAFETY: reading a framework constant.
    let key = unsafe { CBAdvertisementDataLocalNameKey };
    advertisement
        .objectForKey(key)
        .and_then(|value| value.downcast::<NSString>().ok())
        .map(|name| name.to_string())
        .or_else(|| peripheral.name().map(|name| name.to_string()))
}

// ---------------------------------------------------------------------------
// look: the read-only probe
// ---------------------------------------------------------------------------

/// One peripheral [`look`] saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sighting {
    pub name: Option<String>,
    pub id: String,
    /// How it was seen: `"connected, with the audio service"`, `"connected,
    /// with the Battery service"` or `"advertising"`.
    pub how: &'static str,
    /// For an advertisement, the last RSSI (dBm).
    pub rssi: Option<i32>,
}

/// What [`look`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub radio: Radio,
    /// This process's Bluetooth grant (`CBManager.authorization`).
    pub bluetooth: deck_model::Permission,
    /// Connected peripherals (with the audio or the Battery service), then
    /// advertisers with the deck's name.
    pub sightings: Vec<Sighting>,
    /// Advertisers seen under any other name (or none), not listed.
    pub other_advertisers: usize,
}

enum Seen {
    Radio(Radio),
    Advert(Sighting),
}

/// Look without touching: the radio's state, the peripherals the system is
/// connected to that offer the audio or Battery service, and whatever
/// advertises as `name` (normally [`super::DECK_NAME`]) during `window`. Nothing is
/// connected to, read or written, so it is safe next to a deck in any
/// state — including one whose bond this Mac no longer shares.
pub fn look(window: Duration, name: &str) -> Look {
    let (tx, rx) = crossbeam_channel::unbounded::<Seen>();
    let queue = serial_queue("deckd-ble-look");
    let cell = OnQueue::new(queue.clone(), RefCell::new(None::<Looker>));
    {
        let (cell, on_queue) = (cell.clone(), queue.clone());
        queue.exec_sync(move || {
            *cell.get().borrow_mut() = Some(Looker::new(&on_queue, tx));
        });
    }

    let radio = wait_for_radio(&rx, Duration::from_secs(3));
    let bluetooth = super::link::bluetooth_permission(CBManager::authorization());
    let mut sightings = Vec::new();
    let mut other_advertisers = 0;
    if radio == Radio::On {
        let found = &mut sightings;
        let on_queue = cell.clone();
        queue.exec_sync(move || {
            if let Some(looker) = on_queue.get().borrow().as_ref() {
                found.extend(looker.connected());
                looker
                    .central
                    .scanForPeripheralsWithServices_options(None, None);
            }
        });
        let deadline = Instant::now() + window;
        let mut adverts: Vec<Sighting> = Vec::new();
        let mut others = std::collections::BTreeSet::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(Seen::Advert(seen)) if seen.name.as_deref() == Some(name) => {
                    match adverts.iter_mut().find(|a| a.id == seen.id) {
                        Some(known) => known.rssi = seen.rssi,
                        None => adverts.push(seen),
                    }
                }
                Ok(Seen::Advert(seen)) => {
                    others.insert(seen.id);
                }
                Ok(Seen::Radio(_)) => {}
                Err(_) => break,
            }
        }
        sightings.extend(adverts);
        other_advertisers = others.len();
    }

    // Stop and release on the queue; the last strong reference goes with it.
    queue.exec_sync(move || {
        if let Some(looker) = cell.get().borrow_mut().take() {
            if looker.central.isScanning() {
                looker.central.stopScan();
            }
            looker.central.setDelegate(None);
        }
        drop(cell);
    });
    Look {
        radio,
        bluetooth,
        sightings,
        other_advertisers,
    }
}

fn wait_for_radio(rx: &Receiver<Seen>, patience: Duration) -> Radio {
    let deadline = Instant::now() + patience;
    let mut radio = Radio::Unknown;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(Seen::Radio(state)) => {
                radio = state;
                // Unknown and resetting are on their way somewhere.
                if !matches!(state, Radio::Unknown | Radio::Resetting) {
                    break;
                }
            }
            Ok(Seen::Advert(_)) => {}
            Err(_) => break,
        }
    }
    radio
}

struct Looker {
    central: Retained<CBCentralManager>,
    _delegate: Retained<LookDelegate>,
    ids: Ids,
}

impl Looker {
    /// On the queue.
    fn new(queue: &DispatchQueue, tx: Sender<Seen>) -> Looker {
        let delegate = LookDelegate::new(tx);
        let central = central_manager(queue, ProtocolObject::from_ref(&*delegate));
        Looker {
            central,
            _delegate: delegate,
            ids: Ids::new(),
        }
    }

    fn connected(&self) -> Vec<Sighting> {
        let mut out: Vec<Sighting> = Vec::new();
        for (service, how) in [
            (&self.ids.service, "connected, with the audio service"),
            (&self.ids.battery, "connected, with the Battery service"),
        ] {
            let services = NSArray::from_slice(&[&**service]);
            for p in self
                .central
                .retrieveConnectedPeripheralsWithServices(&services)
            {
                let peer = peer_of(&p);
                if out.iter().all(|s| s.id != peer.id) {
                    out.push(Sighting {
                        name: peer.name,
                        id: peer.id,
                        how,
                        rssi: None,
                    });
                }
            }
        }
        out
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `LookDelegate`
    // does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[ivars = Sender<Seen>]
    struct LookDelegate;

    // SAFETY: NSObjectProtocol asks nothing NSObject does not already do.
    unsafe impl NSObjectProtocol for LookDelegate {}

    // SAFETY: each signature is CBCentralManager.h's, and the protocol's one
    // required method is implemented.
    unsafe impl CBCentralManagerDelegate for LookDelegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn look_did_update_state(&self, central: &CBCentralManager) {
            let _ = self.ivars().send(Seen::Radio(Radio::from_raw(central.state())));
        }

        #[unsafe(method(centralManager:didDiscoverPeripheral:advertisementData:RSSI:))]
        fn look_did_discover(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            advertisement: &NSDictionary<NSString, AnyObject>,
            rssi: &NSNumber,
        ) {
            let _ = self.ivars().send(Seen::Advert(Sighting {
                name: advertised_name(peripheral, advertisement),
                id: peripheral.identifier().to_string(),
                how: "advertising",
                rssi: Some(rssi.as_i32()),
            }));
        }
    }
);

impl LookDelegate {
    fn new(tx: Sender<Seen>) -> Retained<LookDelegate> {
        let this = LookDelegate::alloc().set_ivars(tx);
        // SAFETY: NSObject's designated initializer, on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn queue_state_is_reachable_on_its_queue_and_nowhere_else() {
        let queue = serial_queue("deckd-ble-test-get");
        let state = OnQueue::new(queue.clone(), 7u32);
        let on_queue = state.clone();
        queue.exec_sync(move || assert_eq!(*on_queue.get(), 7));
        let off_queue = std::panic::catch_unwind(AssertUnwindSafe(|| *state.get()));
        assert!(off_queue.is_err(), "reached off its queue");
        // Hand the last reference to the queue, as the handle does.
        queue.exec_sync(move || drop(state));
    }

    struct Flag(Arc<AtomicBool>);

    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn queue_state_is_released_on_its_queue_and_leaked_off_it() {
        let queue = serial_queue("deckd-ble-test-drop");
        let dropped = Arc::new(AtomicBool::new(false));
        let state = OnQueue::new(queue.clone(), Flag(dropped.clone()));
        queue.exec_sync(move || drop(state));
        assert!(dropped.load(Ordering::SeqCst), "released on its queue");

        let queue = serial_queue("deckd-ble-test-leak");
        let leaked = Arc::new(AtomicBool::new(false));
        drop(OnQueue::new(queue, Flag(leaked.clone())));
        assert!(
            !leaked.load(Ordering::SeqCst),
            "never released off its queue"
        );
    }

    #[test]
    fn a_job_posted_from_inside_a_job_runs_after_it_in_order() {
        let inbox: Rc<Inbox<Vec<u32>>> = Rc::new(Inbox::default());
        inbox.install(Vec::new());
        let again = inbox.clone();
        inbox.run(move |log| {
            log.push(1);
            again.run(|log| log.push(3));
            log.push(2);
        });
        inbox.run(|log| log.push(4));
        assert_eq!(inbox.close(), Some(vec![1, 2, 3, 4]));
    }

    #[test]
    fn jobs_from_before_the_engine_exists_run_when_it_is_installed() {
        let inbox: Inbox<Vec<u32>> = Inbox::default();
        inbox.run(|log| log.push(1));
        inbox.run(|log| log.push(2));
        inbox.install(vec![0]);
        assert_eq!(inbox.close(), Some(vec![0, 1, 2]));
    }

    #[test]
    fn a_panicking_job_is_contained_and_the_next_one_runs() {
        let inbox: Inbox<Vec<u32>> = Inbox::default();
        inbox.install(Vec::new());
        inbox.run(|_| panic!("a callback went wrong"));
        inbox.run(|log| log.push(9));
        assert_eq!(inbox.close(), Some(vec![9]));
    }

    #[test]
    fn a_closed_inbox_takes_no_more_jobs() {
        let inbox: Inbox<Vec<u32>> = Inbox::default();
        inbox.install(vec![1]);
        assert_eq!(inbox.close(), Some(vec![1]));
        inbox.run(|log| log.push(2));
        assert_eq!(inbox.close(), None);
    }
}
