//! Hand-written CoreBluetooth bindings: the classes, protocols and
//! constants the audio link uses, and nothing else.
//!
//! There is no `objc2-core-bluetooth` in this tree (the build is offline and
//! pinned) and the link needs about twenty selectors, so they are declared
//! here with objc2's own macros. Every signature was copied from the macOS
//! SDK's `CoreBluetooth.framework/Headers`. Debug builds check each message
//! send against the runtime's own method encoding, so a wrong type panics
//! in a test or on the bench instead of corrupting a register.
//!
//! **Thread confinement is why the methods are safe.** None of these types
//! is `Send` or `Sync`, so safe code cannot move one off the thread it was
//! made on; the only way across is `central::OnQueue`, which hands its
//! contents out only on the link's serial queue — the queue the central
//! manager was given, where CoreBluetooth wants every call made. Being on
//! the right queue is guaranteed by construction rather than by each call
//! site, so the calls themselves carry no extra contract. The one method
//! that does (`initWithDelegate:queue:options:`) is `unsafe`.
#![allow(non_snake_case)]

use dispatch2::DispatchQueue;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{extern_class, extern_methods, extern_protocol};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString, NSUUID};

#[link(name = "CoreBluetooth", kind = "framework")]
extern "C" {
    /// `CBCentralManagerOptionShowPowerAlertKey` (NSNumber BOOL): whether
    /// the system may pop up "turn Bluetooth on" for this process. A daemon
    /// must not.
    pub static CBCentralManagerOptionShowPowerAlertKey: &'static NSString;
    /// `CBAdvertisementDataLocalNameKey` (NSString) in a scan result.
    pub static CBAdvertisementDataLocalNameKey: &'static NSString;
}

/// `CBPeripheralState`: disconnected.
pub const PERIPHERAL_DISCONNECTED: isize = 0;
/// `CBCharacteristicWriteType`: without response.
pub const WRITE_WITHOUT_RESPONSE: isize = 1;
/// `CBCharacteristicProperties`: write without response.
pub const PROPERTY_WRITE_WITHOUT_RESPONSE: usize = 0x04;

extern_class!(
    /// `CBManager` (macOS 10.13): what the central and peripheral managers
    /// share.
    #[unsafe(super(NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct CBManager;
);

impl CBManager {
    extern_methods!(
        /// `CBManagerState`; see `attach::Radio::from_raw`.
        #[unsafe(method(state))]
        pub fn state(&self) -> isize;

        /// `CBManagerAuthorization` (macOS 10.15, a class property): this
        /// process's Bluetooth grant; see `link::bluetooth_permission`.
        #[unsafe(method(authorization))]
        pub fn authorization() -> isize;
    );
}

extern_class!(
    /// `CBCentralManager`.
    #[unsafe(super(CBManager, NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct CBCentralManager;
);

impl CBCentralManager {
    extern_methods!(
        /// # Safety
        ///
        /// Every delegate callback runs on `queue`, which must be the queue
        /// all calls on the manager (and the objects it hands out) are made
        /// on. The delegate is held weakly: the caller keeps it alive.
        /// `options` holds the documented keys with their documented types.
        #[unsafe(method(initWithDelegate:queue:options:))]
        pub unsafe fn initWithDelegate_queue_options(
            this: Allocated<Self>,
            delegate: Option<&ProtocolObject<dyn CBCentralManagerDelegate>>,
            queue: Option<&DispatchQueue>,
            options: Option<&NSDictionary<NSString, AnyObject>>,
        ) -> Retained<Self>;

        #[unsafe(method(setDelegate:))]
        pub fn setDelegate(&self, delegate: Option<&ProtocolObject<dyn CBCentralManagerDelegate>>);

        /// Peripherals the *system* is connected to that it knows offer one
        /// of `services`. Synchronous.
        #[unsafe(method(retrieveConnectedPeripheralsWithServices:))]
        pub fn retrieveConnectedPeripheralsWithServices(
            &self,
            services: &NSArray<CBUUID>,
        ) -> Retained<NSArray<CBPeripheral>>;

        #[unsafe(method(scanForPeripheralsWithServices:options:))]
        pub fn scanForPeripheralsWithServices_options(
            &self,
            services: Option<&NSArray<CBUUID>>,
            options: Option<&NSDictionary<NSString, AnyObject>>,
        );

        #[unsafe(method(stopScan))]
        pub fn stopScan(&self);

        #[unsafe(method(isScanning))]
        pub fn isScanning(&self) -> bool;

        #[unsafe(method(connectPeripheral:options:))]
        pub fn connectPeripheral_options(
            &self,
            peripheral: &CBPeripheral,
            options: Option<&NSDictionary<NSString, AnyObject>>,
        );

        /// Ends *this process's* claim on the peripheral. A deck the system
        /// holds for HID stays connected.
        #[unsafe(method(cancelPeripheralConnection:))]
        pub fn cancelPeripheralConnection(&self, peripheral: &CBPeripheral);
    );
}

extern_class!(
    /// `CBPeer` (macOS 10.13).
    #[unsafe(super(NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct CBPeer;
);

impl CBPeer {
    extern_methods!(
        #[unsafe(method(identifier))]
        pub fn identifier(&self) -> Retained<NSUUID>;
    );
}

extern_class!(
    /// `CBPeripheral`.
    #[unsafe(super(CBPeer, NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct CBPeripheral;
);

impl CBPeripheral {
    extern_methods!(
        #[unsafe(method(name))]
        pub fn name(&self) -> Option<Retained<NSString>>;

        /// `CBPeripheralState`.
        #[unsafe(method(state))]
        pub fn state(&self) -> isize;

        /// Held weakly, like the central's.
        #[unsafe(method(setDelegate:))]
        pub fn setDelegate(&self, delegate: Option<&ProtocolObject<dyn CBPeripheralDelegate>>);

        #[unsafe(method(services))]
        pub fn services(&self) -> Option<Retained<NSArray<CBService>>>;

        #[unsafe(method(discoverServices:))]
        pub fn discoverServices(&self, services: Option<&NSArray<CBUUID>>);

        #[unsafe(method(discoverCharacteristics:forService:))]
        pub fn discoverCharacteristics_forService(
            &self,
            characteristics: Option<&NSArray<CBUUID>>,
            service: &CBService,
        );

        #[unsafe(method(readValueForCharacteristic:))]
        pub fn readValueForCharacteristic(&self, characteristic: &CBCharacteristic);

        #[unsafe(method(setNotifyValue:forCharacteristic:))]
        pub fn setNotifyValue_forCharacteristic(
            &self,
            enabled: bool,
            characteristic: &CBCharacteristic,
        );

        /// `type` is a `CBCharacteristicWriteType`.
        #[unsafe(method(maximumWriteValueLengthForType:))]
        pub fn maximumWriteValueLengthForType(&self, kind: isize) -> usize;

        /// Whether a write without response issued now would be taken
        /// (macOS 10.13); `peripheralIsReadyToSendWriteWithoutResponse:`
        /// says when it turns true again.
        #[unsafe(method(canSendWriteWithoutResponse))]
        pub fn canSendWriteWithoutResponse(&self) -> bool;

        #[unsafe(method(writeValue:forCharacteristic:type:))]
        pub fn writeValue_forCharacteristic_type(
            &self,
            data: &NSData,
            characteristic: &CBCharacteristic,
            kind: isize,
        );
    );
}

extern_class!(
    /// `CBAttribute` (macOS 10.13): services and characteristics.
    #[unsafe(super(NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct CBAttribute;
);

impl CBAttribute {
    extern_methods!(
        #[unsafe(method(UUID))]
        pub fn UUID(&self) -> Retained<CBUUID>;
    );
}

extern_class!(
    /// `CBService`.
    #[unsafe(super(CBAttribute, NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct CBService;
);

impl CBService {
    extern_methods!(
        #[unsafe(method(characteristics))]
        pub fn characteristics(&self) -> Option<Retained<NSArray<CBCharacteristic>>>;
    );
}

extern_class!(
    /// `CBCharacteristic`.
    #[unsafe(super(CBAttribute, NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct CBCharacteristic;
);

impl CBCharacteristic {
    extern_methods!(
        /// The latest value read or notified.
        #[unsafe(method(value))]
        pub fn value(&self) -> Option<Retained<NSData>>;

        #[unsafe(method(isNotifying))]
        pub fn isNotifying(&self) -> bool;

        /// `CBCharacteristicProperties`.
        #[unsafe(method(properties))]
        pub fn properties(&self) -> usize;
    );
}

extern_class!(
    /// `CBUUID`. Equality is `isEqual:`, which compares the UUIDs.
    #[unsafe(super(NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct CBUUID;
);

impl CBUUID {
    extern_methods!(
        /// A 16-bit (`"180F"`) or 128-bit UUID string.
        #[unsafe(method(UUIDWithString:))]
        pub fn UUIDWithString(string: &NSString) -> Retained<CBUUID>;

        #[unsafe(method(UUIDString))]
        pub fn UUIDString(&self) -> Retained<NSString>;
    );
}

extern_protocol!(
    /// `CBCentralManagerDelegate`. Only the protocol's identity is declared:
    /// `define_class!` registers the delegate's methods by selector.
    ///
    /// # Safety
    ///
    /// An implementer must answer the protocol's selectors with the
    /// signatures in CBCentralManager.h, including the one required method,
    /// `centralManagerDidUpdateState:`. (The declaration itself is sound:
    /// the name is the protocol's, and it inherits `NSObject`.)
    // The section above is the contract; the lint cannot see doc comments
    // through objc2's attribute routing (objc2's own tests allow it too).
    #[allow(clippy::missing_safety_doc)]
    pub unsafe trait CBCentralManagerDelegate: NSObjectProtocol {}
);

extern_protocol!(
    /// `CBPeripheralDelegate`, declared the same way.
    ///
    /// # Safety
    ///
    /// An implementer must answer the protocol's selectors with the
    /// signatures in CBPeripheral.h (none is required). (The declaration
    /// itself is sound: the name is the protocol's, and it inherits
    /// `NSObject`.)
    // The section above is the contract; the lint cannot see doc comments
    // through objc2's attribute routing (objc2's own tests allow it too).
    #[allow(clippy::missing_safety_doc)]
    pub unsafe trait CBPeripheralDelegate: NSObjectProtocol {}
);
