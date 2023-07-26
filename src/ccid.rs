use std::time::{Duration, Instant};

use apdu_dispatch::dispatch::ApduDispatch;
use apdu_dispatch::interchanges::Data;
use usb_device::bus::{UsbBus, UsbBusAllocator};
use usbd_ccid::{Ccid, Status};

use super::Timeout;

pub fn setup<'bus, 'pipe, B: UsbBus>(
    bus_allocator: &'bus UsbBusAllocator<B>,
    contact: &'pipe interchange::Channel<Data, Data>,
    contactless: &'pipe interchange::Channel<Data, Data>,
) -> (Ccid<'bus, 'pipe, B, 3072>, ApduDispatch<'pipe>) {
    let (ccid_rq, ccid_rp) = contact.split().unwrap();
    let ccid = Ccid::new(bus_allocator, ccid_rq, None);
    let apdu_dispatch = ApduDispatch::new(ccid_rp, contactless.split().unwrap().1);
    (ccid, apdu_dispatch)
}

pub fn keepalive<B: UsbBus, const N: usize>(
    ccid: &mut Ccid<'_, '_, B, N>,
    timeout: &mut Timeout,
    epoch: Instant,
) {
    timeout.update(epoch, map_status(ccid.did_start_processing()), || {
        map_status(ccid.send_wait_extension())
    });
}

fn map_status(status: Status) -> Option<Duration> {
    match status {
        Status::ReceivedData(ms) => Some(Duration::from_millis(ms.0.into())),
        Status::Idle => None,
    }
}
