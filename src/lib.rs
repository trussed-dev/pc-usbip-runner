#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "ccid")]
pub mod ccid;
#[cfg(feature = "ctaphid")]
pub mod ctaphid;
pub mod usb;

pub use usb::{Classes, DefaultSetup, Dispatches, Setup};

use std::{
    marker::PhantomData,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use littlefs2_core::DynFilesystem;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng as _;
use trussed::{
    backend::{CoreOnly, Dispatch},
    pipe::ServiceEndpoint,
    platform,
    service::Service,
    store,
    virt::UserInterface,
    ClientImplementation,
};
use usb_device::{
    bus::{UsbBus, UsbBusAllocator},
    device::{UsbDevice, UsbDeviceBuilder, UsbVidPid},
};
// Classes must be built against these: `usbip-device` is a pinned git dep, so a
// separately declared one yields a distinct `UsbIpBus` that will not unify.
pub use usb_device;
pub use usbip_device::UsbIpBus;

static IS_WAITING: AtomicBool = AtomicBool::new(false);

pub fn set_waiting(waiting: bool) {
    IS_WAITING.store(waiting, Ordering::Relaxed)
}

pub type Client<D = CoreOnly> = ClientImplementation<'static, Syscall, D>;

pub type InitPlatform = Box<dyn Fn(&mut Platform)>;

pub struct Options {
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub vid: u16,
    pub pid: u16,
}

impl Options {
    fn vid_pid(&self) -> UsbVidPid {
        UsbVidPid(self.vid, self.pid)
    }
}

pub trait Apps<'interrupt, D: Dispatch> {
    type Data;

    fn new(
        service: &mut Service<Platform, D>,
        endpoints: &mut Vec<ServiceEndpoint<'static, D::BackendId, D::Context>>,
        syscall: Syscall,
        data: Self::Data,
    ) -> Self;

    #[cfg(feature = "ctaphid")]
    fn with_ctaphid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn ctaphid_dispatch::app::App<'interrupt>]) -> T,
    ) -> T;

    #[cfg(feature = "ccid")]
    fn with_ccid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn apdu_dispatch::app::App]) -> T,
    ) -> T;
}

// virt::Store uses non-static references.  To be able to use the usbip runner with apps that
// require direct access to the store, e. g. provisioner-app, we use a custom store implementation
// with static lifetimes here.
#[derive(Copy, Clone)]
pub struct Store {
    pub ifs: &'static dyn DynFilesystem,
    pub efs: &'static dyn DynFilesystem,
    pub vfs: &'static dyn DynFilesystem,
}

impl store::Store for Store {
    fn ifs(&self) -> &'static dyn DynFilesystem {
        self.ifs
    }

    fn efs(&self) -> &'static dyn DynFilesystem {
        self.efs
    }

    fn vfs(&self) -> &'static dyn DynFilesystem {
        self.vfs
    }
}

unsafe impl Send for Store {}

pub struct Platform {
    rng: ChaCha8Rng,
    store: Store,
    ui: UserInterface,
}

impl Platform {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            rng: ChaCha8Rng::from_entropy(),
            ui: UserInterface::new(),
        }
    }
}

impl platform::Platform for Platform {
    type R = ChaCha8Rng;
    type S = Store;
    type UI = UserInterface;

    fn user_interface(&mut self) -> &mut Self::UI {
        &mut self.ui
    }

    fn rng(&mut self) -> &mut Self::R {
        &mut self.rng
    }

    fn store(&self) -> Self::S {
        self.store
    }
}

pub struct Runner<D, A, S = DefaultSetup> {
    options: Options,
    dispatch: D,
    setup: S,
    _marker: PhantomData<A>,
}

impl<'interrupt, D, A, S> Runner<D, A, S>
where
    D: Dispatch + Send,
    D::BackendId: Send + Sync,
    D::Context: Send + Sync,
    A: Apps<'interrupt, D>,
    S: Setup<D>,
{
    pub fn builder(options: Options) -> Builder {
        Builder::new(options)
    }

    pub fn exec(self, platform: Platform, data: A::Data)
    where
        S::Dispatches: Dispatches<A>,
    {
        // Leaked to give the classes a `'static` bus; `exec` never returns and
        // `UsbIpBus::new` binds port 3240 exclusively.
        // To change IP or port see usbip-device-0.1.4/src/handler.rs:26
        let bus_allocator: &'static UsbBusAllocator<UsbIpBus> =
            Box::leak(Box::new(UsbBusAllocator::new(UsbIpBus::new())));
        // `UsbDevice` unifies the allocator and string-descriptor borrows.
        let options: &'static Options = Box::leak(Box::new(self.options));

        let (mut classes, mut dispatches) = self.setup.setup(bus_allocator, options);

        let mut service = Service::with_dispatch(platform, self.dispatch);
        let mut endpoints = Vec::new();
        let (syscall_sender, syscall_receiver) = mpsc::channel();
        let syscall = Syscall(syscall_sender);
        let mut apps = A::new(&mut service, &mut endpoints, syscall, data);

        log::info!("Ready for work");
        thread::scope(|s| {
            // usb poll + keepalive task
            s.spawn(move || {
                let epoch = Instant::now();
                loop {
                    thread::sleep(Duration::from_millis(5));
                    classes.poll();
                    classes.keepalive(epoch);
                }
            });

            // trussed task
            s.spawn(move || {
                for _ in syscall_receiver.iter() {
                    service.process(&mut endpoints)
                }
            });

            // apps task
            loop {
                thread::sleep(Duration::from_millis(5));
                dispatches.poll(&mut apps);
            }
        });
    }
}

pub struct Builder<D = CoreOnly, S = DefaultSetup> {
    options: Options,
    dispatch: D,
    setup: S,
}

impl Builder {
    pub fn new(options: Options) -> Self {
        Self {
            options,
            dispatch: Default::default(),
            setup: DefaultSetup,
        }
    }
}

impl<D, S> Builder<D, S> {
    pub fn dispatch<E>(self, dispatch: E) -> Builder<E, S> {
        Builder {
            options: self.options,
            dispatch,
            setup: self.setup,
        }
    }

    /// Uses a custom set of USB classes instead of the feature-gated defaults.
    pub fn usb<T>(self, setup: T) -> Builder<D, T> {
        Builder {
            options: self.options,
            dispatch: self.dispatch,
            setup,
        }
    }
}

impl<D: Dispatch, S: Setup<D>> Builder<D, S> {
    pub fn build<'interrupt, A: Apps<'interrupt, D>>(self) -> Runner<D, A, S> {
        Runner {
            options: self.options,
            dispatch: self.dispatch,
            setup: self.setup,
            _marker: Default::default(),
        }
    }
}

#[derive(Clone)]
pub struct Syscall(Sender<()>);

impl trussed::platform::Syscall for Syscall {
    fn syscall(&mut self) {
        log::debug!("syscall");
        self.0.send(()).ok();
    }
}

/// Builds a device from [`Options`]. Must be called after all classes are
/// allocated: building freezes the allocator.
pub fn build_device<'a, B: UsbBus>(
    bus_allocator: &'a UsbBusAllocator<B>,
    options: &'a Options,
) -> UsbDevice<'a, B> {
    use usb_device::prelude::{LangID, StringDescriptors};

    let mut strings = StringDescriptors::new(LangID::EN);
    if let Some(manufacturer) = &options.manufacturer {
        strings = strings.manufacturer(manufacturer);
    }
    if let Some(product) = &options.product {
        strings = strings.product(product);
    }
    if let Some(serial_number) = &options.serial_number {
        strings = strings.serial_number(serial_number);
    }

    UsbDeviceBuilder::new(bus_allocator, options.vid_pid())
        .strings(&[strings])
        .expect("failed to set USB string descriptors")
        .device_class(0x03)
        .device_sub_class(0)
        .build()
}

#[derive(Default)]
pub struct Timeout(Option<Duration>);

impl Timeout {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arms the timer from `keepalive`, or fires `f` and re-arms once expired.
    pub fn update<F: FnOnce() -> Option<Duration>>(
        &mut self,
        epoch: Instant,
        keepalive: Option<Duration>,
        f: F,
    ) {
        if let Some(timeout) = self.0 {
            if epoch.elapsed() >= timeout {
                self.0 = f().map(|duration| epoch.elapsed() + duration);
            }
        } else if let Some(duration) = keepalive {
            self.0 = Some(epoch.elapsed() + duration);
        }
    }
}
