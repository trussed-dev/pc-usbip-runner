#[cfg(feature = "ccid")]
mod ccid;
#[cfg(feature = "ctaphid")]
mod ctaphid;

use std::{cell::RefCell, marker::PhantomData, rc::Rc, thread, time::Duration};

use trussed::{
    backend::{BackendId, CoreOnly, Dispatch},
    client,
    virt::{self, Platform, StoreProvider},
    ClientImplementation,
};
use usb_device::{
    bus::{UsbBus, UsbBusAllocator},
    device::{UsbDevice, UsbDeviceBuilder, UsbVidPid},
};
use usbip_device::UsbIpBus;

pub type Client<S, D = CoreOnly> = ClientImplementation<Service<S, D>, D>;

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

pub trait Apps<C: trussed::Client, D: Dispatch> {
    type Data;

    fn new<B: ClientBuilder<C, D>>(builder: &B, data: Self::Data) -> Self;

    #[cfg(feature = "ctaphid")]
    fn with_ctaphid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn ctaphid_dispatch::app::App]) -> T,
    ) -> T;

    #[cfg(feature = "ccid")]
    fn with_ccid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn apdu_dispatch::app::App<7609, 7609>]) -> T,
    ) -> T;
}

pub trait ClientBuilder<C, D: Dispatch> {
    fn build(&self, id: &str, backends: &'static [BackendId<D::BackendId>]) -> C;
}

pub struct Runner<S: StoreProvider, D, A> {
    store: S,
    options: Options,
    dispatch: D,
    init_platform: Option<InitPlatform<S>>,
    _marker: PhantomData<A>,
}

impl<S: StoreProvider, D: Dispatch, A: Apps<Client<S, D>, D>> Runner<S, D, A> {
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
            let service = Service::new(platform, self.dispatch);
            let mut apps = A::new(&service, data);

            log::info!("Ready for work");
            thread::scope(|s| {
                s.spawn(move || loop {
                    thread::sleep(Duration::from_millis(5));
                    usb_device.poll(&mut [
                        #[cfg(feature = "ctaphid")]
                        &mut ctaphid,
                        #[cfg(feature = "ccid")]
                        &mut ccid,
                    ]);
                });
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
    pub fn build<A: Apps<Client<S, D>, D>>(self) -> Runner<S, D, A> {
        Runner {
            store: self.store,
            options: self.options,
            dispatch: self.dispatch,
            init_platform: self.init_platform,
            _marker: Default::default(),
        }
    }
}

pub struct Service<S: StoreProvider, D: Dispatch>(Rc<RefCell<trussed::Service<Platform<S>, D>>>);

impl<S: StoreProvider, D: Dispatch> Service<S, D> {
    fn new(platform: Platform<S>, dispatch: D) -> Self {
        let service = trussed::Service::with_dispatch(platform, dispatch);
        Self(Rc::new(RefCell::new(service)))
    }
}

impl<S: StoreProvider, D: Dispatch> ClientBuilder<Client<S, D>, D> for Service<S, D> {
    fn build(&self, id: &str, backends: &'static [BackendId<D::BackendId>]) -> Client<S, D> {
        client::ClientBuilder::new(id)
            .backends(backends)
            .prepare(&mut *self.0.borrow_mut())
            .expect("failed to create client")
            .build(self.clone())
    }
}

impl<S: StoreProvider, D: Dispatch> Clone for Service<S, D> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S: StoreProvider, D: Dispatch> trussed::client::Syscall for Service<S, D> {
    fn syscall(&mut self) {
        log::debug!("syscall");
        self.0.borrow_mut().process();
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
