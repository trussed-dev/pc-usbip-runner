#[cfg(feature = "ccid")]
mod ccid;
#[cfg(feature = "ctaphid")]
mod ctaphid;

use std::{cell::RefCell, rc::Rc, thread, time::Duration};

use trussed::{
    virt::{self, Platform, StoreProvider},
    ClientImplementation, Service,
};
use usb_device::{
    bus::{UsbBus, UsbBusAllocator},
    device::{UsbDevice, UsbDeviceBuilder, UsbVidPid},
};
use usbip_device::UsbIpBus;

pub type Client<S> = ClientImplementation<Syscall<Platform<S>>>;

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

pub trait Apps<C: trussed::Client, D> {
    fn new(make_client: impl Fn(&str) -> C, data: D) -> Self;

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

pub struct Runner<S: StoreProvider> {
    store: S,
    options: Options,
    init_platform: Option<Box<dyn Fn(&mut Platform<S>)>>,
}

impl<S: StoreProvider + Clone> Runner<S> {
    pub fn new(store: S, options: Options) -> Self {
        Self {
            store,
            options,
            init_platform: Default::default(),
        }
    }

    pub fn init_platform<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(&mut Platform<S>) + 'static,
    {
        self.init_platform = Some(Box::new(f));
        self
    }

    pub fn exec<A: Apps<Client<S>, D>, D, F: Fn(&mut Platform<S>) -> D>(&self, make_data: F) {
        virt::with_platform(self.store.clone(), |mut platform| {
            if let Some(init_platform) = &self.init_platform {
                init_platform(&mut platform);
            }
            let data = make_data(&mut platform);

            // To change IP or port see usbip-device-0.1.4/src/handler.rs:26
            let bus_allocator = UsbBusAllocator::new(UsbIpBus::new());

            #[cfg(feature = "ctaphid")]
            let (mut ctaphid, mut ctaphid_dispatch) = ctaphid::setup(&bus_allocator);

            #[cfg(feature = "ccid")]
            let (mut ccid, mut apdu_dispatch) = ccid::setup(&bus_allocator);

            let mut usb_device = build_device(&bus_allocator, &self.options);
            let service = Rc::new(RefCell::new(Service::new(platform)));
            let syscall = Syscall::from(service.clone());
            let mut apps = A::new(
                |id| {
                    service
                        .borrow_mut()
                        .try_new_client(id, syscall.clone())
                        .expect("failed to create client")
                },
                data,
            );

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

pub struct Syscall<P: trussed::Platform> {
    service: Rc<RefCell<Service<P>>>,
}

impl<P: trussed::Platform> trussed::client::Syscall for Syscall<P> {
    fn syscall(&mut self) {
        log::debug!("syscall");
        self.service.borrow_mut().process();
    }
}

impl<P: trussed::Platform> Clone for Syscall<P> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
        }
    }
}

impl<P: trussed::Platform> From<Rc<RefCell<Service<P>>>> for Syscall<P> {
    fn from(service: Rc<RefCell<Service<P>>>) -> Self {
        Self { service }
    }
}
