use std::{cell::RefCell, rc::Rc, thread, time::Duration};

use ctaphid_dispatch::{dispatch::Dispatch, types::HidInterchange};
use interchange::Interchange as _;
use trussed::{
    virt::{self, Platform, StoreProvider},
    ClientImplementation, Service,
};
use usb_device::{
    bus::UsbBusAllocator,
    device::{UsbDeviceBuilder, UsbVidPid},
};
use usbd_ctaphid::CtapHid;
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
    Ctaphid(Box<dyn ctaphid_dispatch::app::App>),
}

struct ApplicationSpec<S: StoreProvider> {
    id: String,
    constructor: Box<dyn Fn(Client<S>) -> Application>,
}

pub struct Runner<S: StoreProvider> {
    store: S,
    options: Options,
    init_platform: Option<Box<dyn Fn(&mut Platform<S>)>>,
    add_apps: Vec<ApplicationSpec<S>>,
}

impl<S: StoreProvider + Clone> Runner<S> {
    pub fn new(store: S, options: Options) -> Self {
        Self {
            store,
            options,
            init_platform: Default::default(),
            add_apps: Default::default(),
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
        self.add_apps.push(ApplicationSpec {
            id: id.into(),
            constructor: Box::new(f),
        });
        self
    }

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

            log::info!("Initializing allocator");
            // To change IP or port see usbip-device-0.1.4/src/handler.rs:26
            let bus_allocator = UsbBusAllocator::new(UsbIpBus::new());
            let (ctaphid_rq, ctaphid_rp) = HidInterchange::claim().unwrap();
            let mut ctaphid = CtapHid::new(&bus_allocator, ctaphid_rq, 0u32)
                .implements_ctap1()
                .implements_ctap2()
                .implements_wink();
            let mut ctaphid_dispatch = Dispatch::new(ctaphid_rp);
            let mut usb_builder = UsbDeviceBuilder::new(&bus_allocator, self.options.vid_pid());
            if let Some(manufacturer) = &self.options.manufacturer {
                usb_builder = usb_builder.manufacturer(manufacturer);
            }
            if let Some(product) = &self.options.product {
                usb_builder = usb_builder.product(product);
            }
            if let Some(serial_number) = &self.options.serial_number {
                usb_builder = usb_builder.serial_number(serial_number);
            }
            let mut usb_bus = usb_builder.device_class(0x03).device_sub_class(0).build();

            let service = Rc::new(RefCell::new(Service::new(platform)));
            let syscall = Syscall {
                service: service.clone(),
            };

            let mut apps = Vec::new();
            for spec in &self.add_apps {
                let client = service
                    .borrow_mut()
                    .try_new_client(&spec.id, syscall.clone())
                    .expect("failed to create client");
                apps.push((spec.constructor)(client));
            }

            log::info!("Ready for work");
            loop {
                thread::sleep(Duration::from_millis(5));
                let mut ctaphid_apps = ctaphid_apps(&mut apps);
                ctaphid_dispatch.poll(&mut ctaphid_apps);
                usb_bus.poll(&mut [&mut ctaphid]);
            }
        })
    }
}

fn ctaphid_apps(apps: &mut [Application]) -> Vec<&mut dyn ctaphid_dispatch::app::App> {
    let mut ctaphid_apps = Vec::new();
    for app in apps {
        let Application::Ctaphid(app) = app;
        ctaphid_apps.push(app.as_mut() as &mut dyn ctaphid_dispatch::app::App);
    }
    ctaphid_apps
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
