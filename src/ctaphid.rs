use ctaphid_dispatch::dispatch::Dispatch;
use usb_device::bus::{UsbBus, UsbBusAllocator};
use usbd_ctaphid::CtapHid;

pub fn setup<'bus, 'pipe, 'interrupt, B: UsbBus>(
    bus_allocator: &'bus UsbBusAllocator<B>,
    interchange: &'pipe ctaphid_dispatch::types::Channel,
) -> (
    CtapHid<'bus, 'pipe, 'interrupt, B>,
    Dispatch<'pipe, 'interrupt>,
) {
    let (ctaphid_rq, ctaphid_rp) = interchange.split().unwrap();
    let ctaphid = CtapHid::new(bus_allocator, ctaphid_rq, 0u32)
        .implements_ctap1()
        .implements_ctap2()
        .implements_wink();
    let ctaphid_dispatch = Dispatch::new(ctaphid_rp);
    (ctaphid, ctaphid_dispatch)
}
