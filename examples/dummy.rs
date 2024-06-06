//! USB/IP simulation of a Trussed device.
//!
//! This example contains a dummy app that responds with random data to the CTAPHID vendor command
//! 0x60.  It can be tested with `nitropy nk3 list` and `nitropy nk3 rng`.
//!
//! For a more complete example, see the [usbip runner][] for the Nitrokey 3.
//!
//! [usbip runner]: https://github.com/Nitrokey/nitrokey-3-firmware/tree/main/runners/usbip

use std::path::PathBuf;

#[cfg(feature = "ccid")]
use apdu_dispatch::command::SIZE as ApduCommandSize;
#[cfg(feature = "ctaphid")]
use ctaphid_dispatch::{
    command::{Command, VendorCommand},
    types::{AppResult, Error, Message},
};

use clap::Parser;
use clap_num::maybe_hex;
use trussed::{
    backend::CoreOnly,
    client::{Client, ClientBuilder},
    service::Service,
    syscall,
    types::Vec,
    virt::{self, Platform, StoreProvider},
};
use trussed_usbip::Syscall;

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
    fn rng<const N: usize>(&mut self, response: &mut Vec<u8, N>) {
        let bytes = syscall!(self.client.random_bytes(57)).bytes;
        response.extend_from_slice(&bytes).unwrap();
    }
}

#[cfg(feature = "ctaphid")]
const CTAPHID_COMMAND_RNG: Command = Command::Vendor(VendorCommand::H60);

#[cfg(feature = "ctaphid")]
impl<'a, C: Client> ctaphid_dispatch::app::App<'a> for DummyApp<C> {
    fn commands(&self) -> &'static [Command] {
        &[CTAPHID_COMMAND_RNG]
    }

    fn call(&mut self, command: Command, _request: &Message, response: &mut Message) -> AppResult {
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

impl<'a, S: StoreProvider> trussed_usbip::Apps<'a, S, CoreOnly>
    for Apps<trussed_usbip::Client<CoreOnly>>
{
    type Data = ();

    fn new(service: &mut Service<Platform<S>, CoreOnly>, syscall: Syscall, _data: ()) -> Self {
        let client = ClientBuilder::new("dummy")
            .prepare(service)
            .unwrap()
            .build(syscall);
        let dummy = DummyApp { client };
        Self { dummy }
    }

    #[cfg(feature = "ctaphid")]
    fn with_ctaphid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn ctaphid_dispatch::app::App<'a>]) -> T,
    ) -> T {
        f(&mut [&mut self.dummy])
    }

    #[cfg(feature = "ccid")]
    fn with_ccid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn apdu_dispatch::app::App<ApduCommandSize, ApduCommandSize>]) -> T,
    ) -> T {
        f(&mut [])
    }
}

fn main() {
    pretty_env_logger::init();

    let args = Args::parse();

    let store = virt::Filesystem::new(args.state_file);
    let options = trussed_usbip::Options {
        manufacturer: Some(args.manufacturer),
        product: Some(args.name),
        serial_number: None,
        vid: args.vid,
        pid: args.pid,
    };

    log::info!("Initializing Trussed");
    trussed_usbip::Builder::new(store, options)
        .build::<Apps<_>>()
        .exec(|_| ());
}
