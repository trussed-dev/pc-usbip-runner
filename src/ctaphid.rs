use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use ctaphid_dispatch::Dispatch;
use usb_device::bus::{UsbBus, UsbBusAllocator};
use usbd_ctaphid::{types::Status, CtapHid};

use super::{Timeout, IS_WAITING};

pub fn setup<'bus, 'pipe, 'interrupt, B: UsbBus, const N: usize>(
    bus_allocator: &'bus UsbBusAllocator<B>,
    interchange: &'pipe ctaphid_dispatch::Channel<N>,
) -> (
    CtapHid<'bus, 'pipe, 'interrupt, B, N>,
    Dispatch<'pipe, 'interrupt, N>,
) {
    let (ctaphid_rq, ctaphid_rp) = interchange.split().unwrap();
    let ctaphid = CtapHid::new(bus_allocator, ctaphid_rq, 0u32)
        .implements_ctap1()
        .implements_ctap2()
        .implements_wink();
    let ctaphid_dispatch = Dispatch::new(ctaphid_rp);
    (ctaphid, ctaphid_dispatch)
}

pub fn keepalive<B: UsbBus, const N: usize>(
    ctaphid: &mut CtapHid<'_, '_, '_, B, N>,
    timeout: &mut Timeout,
    epoch: Instant,
) {
    timeout.update(epoch, map_status(ctaphid.did_start_processing()), || {
        map_status(ctaphid.send_keepalive(IS_WAITING.load(Ordering::Relaxed)))
    });
}

fn map_status(status: Status) -> Option<Duration> {
    match status {
        Status::ReceivedData(ms) => Some(Duration::from_millis(ms.0.into())),
        Status::Idle => None,
    }
}
