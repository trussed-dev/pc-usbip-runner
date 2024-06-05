#[cfg(feature = "ccid")]
mod ccid;
#[cfg(feature = "ctaphid")]
mod ctaphid;

use std::{
    marker::PhantomData,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use trussed::{
    backend::{CoreOnly, Dispatch},
    service::Service,
    virt::{self, Platform, StoreProvider},
    ClientImplementation,
};
use usb_device::{
    bus::{UsbBus, UsbBusAllocator},
    device::{UsbDevice, UsbDeviceBuilder, UsbVidPid},
};
use usbip_device::UsbIpBus;

static IS_WAITING: AtomicBool = AtomicBool::new(false);

pub fn set_waiting(waiting: bool) {
    IS_WAITING.store(waiting, Ordering::Relaxed)
}

pub type Client<D = CoreOnly> = ClientImplementation<Syscall, D>;

pub type InitPlatform<S> = Box<dyn Fn(&mut Platform<S>)>;

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

pub trait Apps<'interrupt, S: StoreProvider, D: Dispatch> {
    type Data;

    fn new(service: &mut Service<Platform<S>, D>, syscall: Syscall, data: Self::Data) -> Self;

    #[cfg(feature = "ctaphid")]
    fn with_ctaphid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn ctaphid_dispatch::app::App<'interrupt>]) -> T,
    ) -> T;

    #[cfg(feature = "ccid")]
    fn with_ccid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn apdu_dispatch::app::App<7609, 7609>]) -> T,
    ) -> T;
}

pub struct Runner<S: StoreProvider, D, A> {
    store: S,
    options: Options,
    dispatch: D,
    init_platform: Option<InitPlatform<S>>,
    _marker: PhantomData<A>,
}

impl<'interrupt, S: StoreProvider, D: Dispatch, A: Apps<'interrupt, S, D>> Runner<S, D, A> {
    pub fn builder(store: S, options: Options) -> Builder<S> {
        Builder::new(store, options)
    }

    pub fn exec<F: Fn(&mut Platform<S>) -> A::Data>(self, make_data: F) {
        virt::with_platform(self.store, |mut platform| {
            if let Some(init_platform) = &self.init_platform {
                init_platform(&mut platform);
            }
            let data = make_data(&mut platform);

            // To change IP or port see usbip-device-0.1.4/src/handler.rs:26
            let bus_allocator = UsbBusAllocator::new(UsbIpBus::new());

            #[cfg(feature = "ctaphid")]
            let ctap_channel = ctaphid_dispatch::types::Channel::new();
            #[cfg(feature = "ctaphid")]
            let (mut ctaphid, mut ctaphid_dispatch) = ctaphid::setup(&bus_allocator, &ctap_channel);

            #[cfg(feature = "ccid")]
            let (contact, contactless) = Default::default();
            #[cfg(feature = "ccid")]
            let (mut ccid, mut apdu_dispatch) = ccid::setup(&bus_allocator, &contact, &contactless);

            let mut usb_device = build_device(&bus_allocator, &self.options);
            let mut service = Service::with_dispatch(platform, self.dispatch);
            let (syscall_sender, syscall_receiver) = mpsc::channel();
            let syscall = Syscall(syscall_sender);
            let mut apps = A::new(&mut service, syscall, data);

            log::info!("Ready for work");
            thread::scope(|s| {
                // usb poll + keepalive task
                s.spawn(move || {
                    let _epoch = Instant::now();
                    #[cfg(feature = "ctaphid")]
                    let mut timeout_ctaphid = Timeout::new();
                    #[cfg(feature = "ccid")]
                    let mut timeout_ccid = Timeout::new();

                    loop {
                        thread::sleep(Duration::from_millis(5));
                        usb_device.poll(&mut [
                            #[cfg(feature = "ctaphid")]
                            &mut ctaphid,
                            #[cfg(feature = "ccid")]
                            &mut ccid,
                        ]);

                        #[cfg(feature = "ctaphid")]
                        ctaphid::keepalive(&mut ctaphid, &mut timeout_ctaphid, _epoch);
                        #[cfg(feature = "ccid")]
                        ccid::keepalive(&mut ccid, &mut timeout_ccid, _epoch);
                    }
                });

                // trussed task
                s.spawn(move || {
                    for _ in syscall_receiver.iter() {
                        service.process()
                    }
                });

                // apps task
                loop {
                    thread::sleep(Duration::from_millis(5));
                    #[cfg(feature = "ctaphid")]
                    apps.with_ctaphid_apps(|apps| ctaphid_dispatch.poll(apps));
                    #[cfg(feature = "ccid")]
                    apps.with_ccid_apps(|apps| apdu_dispatch.poll(apps));
                }
            });
        })
    }
}

pub struct Builder<S: StoreProvider, D = CoreOnly> {
    store: S,
    options: Options,
    dispatch: D,
    init_platform: Option<InitPlatform<S>>,
}

impl<S: StoreProvider> Builder<S> {
    pub fn new(store: S, options: Options) -> Self {
        Self {
            store,
            options,
            dispatch: Default::default(),
            init_platform: Default::default(),
        }
    }
}

impl<S: StoreProvider, D> Builder<S, D> {
    pub fn dispatch<E>(self, dispatch: E) -> Builder<S, E> {
        Builder {
            store: self.store,
            options: self.options,
            dispatch,
            init_platform: self.init_platform,
        }
    }

    pub fn init_platform<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut Platform<S>) + 'static,
    {
        self.init_platform = Some(Box::new(f));
        self
    }
}

impl<S: StoreProvider, D: Dispatch> Builder<S, D> {
    pub fn build<'interrupt, A: Apps<'interrupt, S, D>>(self) -> Runner<S, D, A> {
        Runner {
            store: self.store,
            options: self.options,
            dispatch: self.dispatch,
            init_platform: self.init_platform,
            _marker: Default::default(),
        }
    }
}

#[derive(Clone)]
pub struct Syscall(Sender<()>);

impl trussed::client::Syscall for Syscall {
    fn syscall(&mut self) {
        log::debug!("syscall");
        self.0.send(()).ok();
    }
}

fn build_device<'a, B: UsbBus>(
    bus_allocator: &'a UsbBusAllocator<B>,
    options: &'a Options,
) -> UsbDevice<'a, B> {
    let mut usb_builder = UsbDeviceBuilder::new(bus_allocator, options.vid_pid());
    if let Some(manufacturer) = &options.manufacturer {
        usb_builder = usb_builder.manufacturer(manufacturer);
    }
    if let Some(product) = &options.product {
        usb_builder = usb_builder.product(product);
    }
    if let Some(serial_number) = &options.serial_number {
        usb_builder = usb_builder.serial_number(serial_number);
    }
    usb_builder.device_class(0x03).device_sub_class(0).build()
}

#[derive(Default)]
pub struct Timeout(Option<Duration>);

impl Timeout {
    fn new() -> Self {
        Self::default()
    }

    fn update<F: FnOnce() -> Option<Duration>>(
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
