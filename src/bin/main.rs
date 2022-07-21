use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use clap::Parser;
use clap_num::maybe_hex;

use interchange::Interchange;
use log::info;
use usb_device::{bus::UsbBusAllocator, prelude::*};
use usbip_device::UsbIpBus;
use trussed::{Platform as _, virt};
use trussed::platform::{consent, reboot, ui};

use fido_authenticator;
use usbd_ctaphid::constants::MESSAGE_SIZE;

pub type FidoConfig = fido_authenticator::Config;

/// USP/IP based virtualization of the Nitrokey 3 / Solo2 device.
/// Supports FIDO application at the moment.
#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
    /// USB Name string
    #[clap(short, long, default_value = "FIDO authenticator")]
    name: String,

    /// USB Manufacturer string
    #[clap(short, long, default_value = "Simulation")]
    manufacturer: String,

    /// USB Serial string
    #[clap(long, default_value = "SIM SIM SIM")]
    serial: String,

    /// Trussed state file
    #[clap(long, default_value = "trussed-state.bin")]
    state_file: PathBuf,

    /// FIDO attestation key
    #[clap(long)]
    fido_key: Option<PathBuf>,

    /// FIDO attestation cert
    #[clap(long)]
    fido_cert: Option<PathBuf>,

    /// USB VID id
    #[clap(short, long, parse(try_from_str=maybe_hex), default_value_t = 0x20a0)]
    vid: u16,
    /// USB PID id
    #[clap(short, long, parse(try_from_str=maybe_hex), default_value_t = 0x42b2)]
    pid: u16,
}

struct Reboot;

impl admin_app::Reboot for Reboot {
    fn reboot() -> ! {
        unimplemented!();
    }

    fn reboot_to_firmware_update() -> ! {
        unimplemented!();
    }

    fn reboot_to_firmware_update_destructive() -> ! {
        unimplemented!();
    }

    fn locked() -> bool {
        false
    }
}

struct Syscall<P: trussed::Platform> {
    service: Rc<RefCell<trussed::service::Service<P>>>,
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

struct UserInterface {
    start_time: std::time::Instant,
}

impl UserInterface {
    fn new() -> Self {
        Self {
            start_time: std::time::Instant::now(),
        }
    }
}

impl trussed::platform::UserInterface for UserInterface {
    /// Prompt user to type a word for confirmation
    fn check_user_presence(&mut self) -> consent::Level {
        // use std::io::Read as _;
        // This is not nice - we should "peek" and return Level::None
        // if there is no key pressed yet (unbuffered read from stdin).
        // Couldn't get this to work (without pulling in ncurses or similar).
        // std::io::stdin().bytes().next();
        consent::Level::Normal
    }

    fn set_status(&mut self, status: ui::Status) {
        info!("Set status: {:?}", status);

        if status == ui::Status::WaitingForUserPresence {
            info!(">>>> Received confirmation request. Confirming automatically.");
        }
    }

    fn refresh(&mut self) {}

    fn uptime(&mut self) -> core::time::Duration {
        self.start_time.elapsed()
    }

    fn reboot(&mut self, to: reboot::To) -> ! {
        info!("Restart!  ({:?})", to);
        std::process::exit(25);
    }
}

fn main() {
    #[cfg(feature = "enable-logs")]
    pretty_env_logger::init();

    let args = Args::parse();

    log::info!("Initializing Trussed");

    virt::with_platform(virt::FilesystemStore::new(&args.state_file), |mut platform| {
        let ui: Box<dyn trussed::platform::UserInterface> = Box::new(UserInterface::new());
        platform.user_interface().set_inner(ui);

        if let Some(fido_key) = args.fido_key {
            store(&platform, &fido_key, "fido/sec/00");
        }
        if let Some(fido_cert) = args.fido_cert {
            store(&platform, &fido_cert, "fido/x5c/00");
        }

        let trussed_service = Rc::new(RefCell::new(trussed::service::Service::new(
            platform,
        )));

        log::info!("Initializing allocator");
        // To change IP or port see usbip-device-0.1.4/src/handler.rs:26
        let bus_allocator = UsbBusAllocator::new(UsbIpBus::new());
        let (ctaphid_rq, ctaphid_rp) = ctaphid_dispatch::types::HidInterchange::claim().unwrap();
        let mut ctaphid = usbd_ctaphid::CtapHid::new(&bus_allocator, ctaphid_rq, 0u32)
            .implements_ctap1()
            .implements_ctap2()
            .implements_wink();
        let mut ctaphid_dispatch = ctaphid_dispatch::dispatch::Dispatch::new(ctaphid_rp);
        let mut usb_bus = UsbDeviceBuilder::new(&bus_allocator, UsbVidPid(args.vid, args.pid))
            .manufacturer(&args.manufacturer)
            .product(&args.name)
            .serial_number(&args.serial)
            .device_class(0x03)
            .device_sub_class(0)
            .build();

        let syscall = Syscall {
            service: trussed_service.clone(),
        };

        let trussed_client = trussed_service
            .borrow_mut()
            .try_new_client("fido", syscall.clone())
            .unwrap();
        let mut fido_app = fido_authenticator::Authenticator::new(
            trussed_client,
            fido_authenticator::Conforming {},
            fido_authenticator::Config {
                max_msg_size: MESSAGE_SIZE,
            },
        );

        let trussed_client = trussed_service
            .borrow_mut()
            .try_new_client("admin", syscall.clone())
            .unwrap();
        let mut admin_app = admin_app::App::<_, Reboot>::new(trussed_client, [0; 16], 0);

        log::info!("Ready for work");
        loop {
            std::thread::sleep(std::time::Duration::from_millis(5));
            ctaphid_dispatch.poll(&mut [&mut fido_app, &mut admin_app]);
            usb_bus.poll(&mut [&mut ctaphid]);
        }
    });
}

fn store(platform: &impl trussed::Platform, host_file: &Path, device_file: &str) {
    log::info!("Writing {} to file system", device_file);
    let data = std::fs::read(host_file).expect("failed to read file");
    trussed::store::store(
        platform.store(),
        trussed::types::Location::Internal,
        &littlefs2::path::PathBuf::from(device_file),
        &data,
    )
    .expect("failed to store file");
}
