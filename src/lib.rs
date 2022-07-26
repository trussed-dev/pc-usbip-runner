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

pub enum Application {
    #[cfg(feature = "ctaphid")]
    Ctaphid(Box<dyn ctaphid_dispatch::app::App>),
}

struct ApplicationSpec<S: StoreProvider> {
    id: String,
    constructor: Box<dyn Fn(Client<S>) -> Application>,
}

impl<S: StoreProvider> ApplicationSpec<S> {
    fn create(&self, service: &mut Rc<RefCell<Service<Platform<S>>>>) -> Application {
        let client = service
            .borrow_mut()
            .try_new_client(&self.id, Syscall::from(service.clone()))
            .expect("failed to create client");
        (self.constructor)(client)
    }
}

pub struct Runner<S: StoreProvider> {
    store: S,
    options: Options,
    init_platform: Option<Box<dyn Fn(&mut Platform<S>)>>,
    apps: Vec<ApplicationSpec<S>>,
}

impl<S: StoreProvider + Clone> Runner<S> {
    pub fn new(store: S, options: Options) -> Self {
        Self {
            store,
            options,
            init_platform: Default::default(),
            apps: Default::default(),
        }
    }

    pub fn init_platform<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(&mut Platform<S>) + 'static,
    {
        self.init_platform = Some(Box::new(f));
        self
    }

    pub fn add_app<N, F>(&mut self, id: N, f: F) -> &mut Self
    where
        N: Into<String>,
        F: Fn(Client<S>) -> Application + 'static,
    {
        self.apps.push(ApplicationSpec {
            id: id.into(),
            constructor: Box::new(f),
        });
        self
    }

    #[cfg(feature = "ctaphid")]
    pub fn add_ctaphid_app<N, F, A>(&mut self, id: N, f: F) -> &mut Self
    where
        N: Into<String>,
        F: Fn(Client<S>) -> A + 'static,
        A: ctaphid_dispatch::app::App + 'static,
    {
        self.add_app(id, move |client| Application::Ctaphid(Box::new(f(client))))
    }

    pub fn exec(&self) {
        virt::with_platform(self.store.clone(), |mut platform| {
            if let Some(init_platform) = &self.init_platform {
                init_platform(&mut platform);
            }

            // To change IP or port see usbip-device-0.1.4/src/handler.rs:26
            let bus_allocator = UsbBusAllocator::new(UsbIpBus::new());

            #[cfg(feature = "ctaphid")]
            let (mut ctaphid, mut ctaphid_dispatch) = ctaphid::setup(&bus_allocator);

            let mut usb_device = build_device(&bus_allocator, &self.options);
            let mut service = Rc::new(RefCell::new(Service::new(platform)));
            let mut apps = self.create_apps(&mut service);

            log::info!("Ready for work");
            loop {
                thread::sleep(Duration::from_millis(5));

                #[cfg(feature = "ctaphid")]
                ctaphid_dispatch.poll(&mut ctaphid::apps(&mut apps));

                usb_device.poll(&mut [
                    #[cfg(feature = "ctaphid")]
                    &mut ctaphid,
                ]);
            }
        })
    }

    fn create_apps(&self, service: &mut Rc<RefCell<Service<Platform<S>>>>) -> Vec<Application> {
        self.apps.iter().map(|app| app.create(service)).collect()
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
