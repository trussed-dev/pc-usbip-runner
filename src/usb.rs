//! Caller-supplied USB classes.
//!
//! [`Runner::exec`][crate::Runner::exec] owns the bus and the poll loop, but not
//! the class list. Implement [`Setup`] to provide your own classes and select it
//! with [`Builder::usb`][crate::Builder::usb].

use std::marker::PhantomData;
use std::time::Instant;

use usb_device::bus::UsbBusAllocator;
use usb_device::device::UsbDevice;
use usbip_device::UsbIpBus;

use crate::{Apps, Options};

/// The USB device and its classes, driven by the USB thread.
pub trait Classes: Send {
    /// Runs one iteration of the USB poll loop.
    ///
    /// [`UsbDevice::poll`] only calls [`UsbClass::poll`] on bus activity, so
    /// classes that queue application responses must be drained here too.
    ///
    /// [`UsbClass::poll`]: usb_device::class::UsbClass::poll
    fn poll(&mut self);

    /// Drives timers that are not part of [`UsbClass`][usb_device::class::UsbClass],
    /// such as the CCID wait extension. Called after every [`Classes::poll`].
    fn keepalive(&mut self, epoch: Instant) {
        let _ = epoch;
    }
}

/// The dispatchers routing class traffic to the applications, driven by the
/// thread that owns the apps.
pub trait Dispatches<A> {
    /// Runs one iteration of the dispatch loop.
    fn poll(&mut self, apps: &mut A);
}

/// Creates the USB device, its classes and the matching dispatchers.
///
/// The allocator is leaked by [`Runner::exec`][crate::Runner::exec], so
/// everything built here may be `'static`.
///
/// `D` is the trussed backend dispatch the apps are built against.
pub trait Setup<D> {
    /// Moved to the USB thread; must not hold the allocator, which is `!Send`.
    type Classes: Classes;
    /// Kept on the thread that owns the apps.
    type Dispatches;

    /// Allocates the classes, then builds the device.
    ///
    /// The device must be built last: building freezes the allocator and any
    /// later endpoint, interface or string allocation panics.
    fn setup(
        self,
        allocator: &'static UsbBusAllocator<UsbIpBus>,
        options: &'static Options,
    ) -> (Self::Classes, Self::Dispatches);
}

/// The classes selected by the `ccid` and `ctaphid` features.
#[derive(Default)]
pub struct DefaultSetup;

#[cfg(feature = "ctaphid")]
const CTAP_MESSAGE_SIZE: usize = ctaphid_dispatch::DEFAULT_MESSAGE_SIZE;

/// [`DefaultSetup`]'s device and classes.
pub struct DefaultClasses {
    usb_device: UsbDevice<'static, UsbIpBus>,
    #[cfg(feature = "ctaphid")]
    ctaphid: usbd_ctaphid::CtapHid<'static, 'static, 'static, UsbIpBus, CTAP_MESSAGE_SIZE>,
    #[cfg(feature = "ctaphid")]
    timeout_ctaphid: crate::Timeout,
    #[cfg(feature = "ccid")]
    ccid: usbd_ccid::Ccid<'static, 'static, UsbIpBus, 3072>,
    #[cfg(feature = "ccid")]
    timeout_ccid: crate::Timeout,
}

/// [`DefaultSetup`]'s dispatchers.
pub struct DefaultDispatches<D = trussed::backend::CoreOnly> {
    #[cfg(feature = "ctaphid")]
    ctaphid: ctaphid_dispatch::Dispatch<'static, 'static, CTAP_MESSAGE_SIZE>,
    #[cfg(feature = "ccid")]
    apdu: apdu_dispatch::dispatch::ApduDispatch<'static>,
    _marker: PhantomData<D>,
}

impl<D: trussed::backend::Dispatch> Setup<D> for DefaultSetup {
    type Classes = DefaultClasses;
    type Dispatches = DefaultDispatches<D>;

    fn setup(
        self,
        allocator: &'static UsbBusAllocator<UsbIpBus>,
        options: &'static Options,
    ) -> (DefaultClasses, DefaultDispatches<D>) {
        #[cfg(feature = "ctaphid")]
        static CTAP_CHANNEL: ctaphid_dispatch::Channel<CTAP_MESSAGE_SIZE> =
            ctaphid_dispatch::Channel::new();
        #[cfg(feature = "ccid")]
        static CONTACT: interchange::Channel<
            apdu_dispatch::interchanges::Data,
            apdu_dispatch::interchanges::Data,
        > = interchange::Channel::new();
        #[cfg(feature = "ccid")]
        static CONTACTLESS: interchange::Channel<
            apdu_dispatch::interchanges::Data,
            apdu_dispatch::interchanges::Data,
        > = interchange::Channel::new();

        #[cfg(feature = "ctaphid")]
        let (ctaphid, ctaphid_dispatch) = crate::ctaphid::setup(allocator, &CTAP_CHANNEL);
        #[cfg(feature = "ccid")]
        let (ccid, apdu_dispatch) = crate::ccid::setup(allocator, &CONTACT, &CONTACTLESS);

        let usb_device = crate::build_device(allocator, options);

        (
            DefaultClasses {
                usb_device,
                #[cfg(feature = "ctaphid")]
                ctaphid,
                #[cfg(feature = "ctaphid")]
                timeout_ctaphid: crate::Timeout::new(),
                #[cfg(feature = "ccid")]
                ccid,
                #[cfg(feature = "ccid")]
                timeout_ccid: crate::Timeout::new(),
            },
            DefaultDispatches {
                #[cfg(feature = "ctaphid")]
                ctaphid: ctaphid_dispatch,
                #[cfg(feature = "ccid")]
                apdu: apdu_dispatch,
                _marker: PhantomData,
            },
        )
    }
}

impl Classes for DefaultClasses {
    fn poll(&mut self) {
        // `UsbDevice::poll` only polls classes on bus activity, so queued
        // application responses have to be picked up here.
        #[cfg(feature = "ctaphid")]
        self.ctaphid.check_for_app_response();
        #[cfg(feature = "ccid")]
        self.ccid.check_for_app_response();

        self.usb_device.poll(&mut [
            #[cfg(feature = "ctaphid")]
            &mut self.ctaphid,
            #[cfg(feature = "ccid")]
            &mut self.ccid,
        ]);
    }

    fn keepalive(&mut self, epoch: Instant) {
        #[cfg(feature = "ctaphid")]
        crate::ctaphid::keepalive(&mut self.ctaphid, &mut self.timeout_ctaphid, epoch);
        #[cfg(feature = "ccid")]
        crate::ccid::keepalive(&mut self.ccid, &mut self.timeout_ccid, epoch);
        let _ = epoch;
    }
}

impl<D: trussed::backend::Dispatch, A: Apps<'static, D>> Dispatches<A> for DefaultDispatches<D> {
    fn poll(&mut self, apps: &mut A) {
        #[cfg(feature = "ctaphid")]
        apps.with_ctaphid_apps(|apps| self.ctaphid.poll(apps));
        #[cfg(feature = "ccid")]
        apps.with_ccid_apps(|apps| self.apdu.poll(apps));
        let _ = apps;
    }
}
