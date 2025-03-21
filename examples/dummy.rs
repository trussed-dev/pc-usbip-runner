//! USB/IP simulation of a Trussed device.
//!
//! This example contains a dummy app that responds with random data to the CTAPHID vendor command
//! 0x60.  It can be tested with `nitropy nk3 list` and `nitropy nk3 rng`.
//!
//! For a more complete example, see the [usbip runner][] for the Nitrokey 3.
//!
//! [usbip runner]: https://github.com/Nitrokey/nitrokey-3-firmware/tree/main/runners/usbip

use std::path::PathBuf;

#[cfg(feature = "ctaphid")]
use ctaphid_dispatch::app::{Command, Error, VendorCommand};

use clap::Parser;
use clap_num::maybe_hex;
use littlefs2::{
    const_ram_storage,
    fs::{Allocation, Filesystem},
};
use littlefs2_core::{path, DynFilesystem};
use trussed::{
    backend::{CoreOnly, NoId},
    client::Client,
    pipe::{ServiceEndpoint, TrussedChannel},
    service::Service,
    syscall,
    types::{CoreContext, NoData},
};
use trussed_usbip::{Platform, Store, Syscall};

/// USP/IP based virtualization a Trussed device.
#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
    /// USB Name string
    #[clap(short, long, default_value = "Trussed")]
    name: String,

    /// USB Manufacturer string
    #[clap(short, long, default_value = "Trussed")]
    manufacturer: String,

    /// Trussed state file
    #[clap(long, default_value = "trussed-state.bin")]
    state_file: PathBuf,

    /// USB VID id
    #[clap(short, long, parse(try_from_str=maybe_hex), default_value_t = 0x20a0)]
    vid: u16,

    /// USB PID id
    #[clap(short, long, parse(try_from_str=maybe_hex), default_value_t = 0x42b2)]
    pid: u16,
}

struct DummyApp<C: Client> {
    client: C,
}

impl<C: Client> DummyApp<C> {
    fn rng<const N: usize>(&mut self, response: &mut heapless_bytes::Bytes<N>) {
        let bytes = syscall!(self.client.random_bytes(57)).bytes;
        response.extend_from_slice(&bytes).unwrap();
    }
}

#[cfg(feature = "ctaphid")]
const CTAPHID_COMMAND_RNG: Command = Command::Vendor(VendorCommand::H60);

#[cfg(feature = "ctaphid")]
impl<C: Client, const N: usize> ctaphid_dispatch::app::App<'_, N> for DummyApp<C> {
    fn commands(&self) -> &'static [Command] {
        &[CTAPHID_COMMAND_RNG]
    }

    fn call(
        &mut self,
        command: Command,
        _request: &[u8],
        response: &mut heapless_bytes::Bytes<N>,
    ) -> Result<(), Error> {
        match command {
            CTAPHID_COMMAND_RNG => self.rng(response),
            _ => return Err(Error::InvalidCommand),
        }
        Ok(())
    }
}

struct Apps<C: Client> {
    dummy: DummyApp<C>,
}

impl<'a> trussed_usbip::Apps<'a, CoreOnly> for Apps<trussed_usbip::Client<CoreOnly>> {
    type Data = ();

    fn new(
        _service: &mut Service<Platform, CoreOnly>,
        endpoints: &mut Vec<ServiceEndpoint<'static, NoId, NoData>>,
        syscall: Syscall,
        _data: (),
    ) -> Self {
        static CHANNEL: TrussedChannel = TrussedChannel::new();
        let (requester, responder) = CHANNEL.split().unwrap();
        let context = CoreContext::new(path!("dummy").into());
        endpoints.push(ServiceEndpoint::new(responder, context, &[]));
        let client = trussed_usbip::Client::new(requester, syscall, None);
        let dummy = DummyApp { client };
        Self { dummy }
    }

    #[cfg(feature = "ctaphid")]
    fn with_ctaphid_apps<T, const N: usize>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn ctaphid_dispatch::app::App<'a, N>]) -> T,
    ) -> T {
        f(&mut [&mut self.dummy])
    }

    #[cfg(feature = "ccid")]
    fn with_ccid_apps<T, const N: usize>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn apdu_dispatch::app::App<N>]) -> T,
    ) -> T {
        f(&mut [])
    }
}

const_ram_storage!(RamStorage, 512 * 128);

fn ram_filesystem() -> &'static dyn DynFilesystem {
    let storage = Box::leak(Box::new(RamStorage::new()));
    Filesystem::format(storage).expect("failed to format RAM filesystem");
    let alloc = Box::leak(Box::new(Allocation::new()));
    let fs = Filesystem::mount(alloc, storage).expect("failed to mount RAM filesystem");
    Box::leak(Box::new(fs))
}

fn main() {
    pretty_env_logger::init();

    let args = Args::parse();

    // TODO: use filesystem storage for IFS
    let store = Store {
        ifs: ram_filesystem(),
        efs: ram_filesystem(),
        vfs: ram_filesystem(),
    };
    let options = trussed_usbip::Options {
        manufacturer: Some(args.manufacturer),
        product: Some(args.name),
        serial_number: None,
        vid: args.vid,
        pid: args.pid,
    };

    log::info!("Initializing Trussed");
    let platform = Platform::new(store);
    trussed_usbip::Builder::new(options)
        .build::<Apps<_>>()
        .exec(platform, ());
}
