use ctaphid_dispatch::{dispatch::Dispatch, types::HidInterchange};
use interchange::Interchange as _;
use usb_device::bus::{UsbBus, UsbBusAllocator};
use usbd_ctaphid::CtapHid;

pub fn setup<B: UsbBus>(bus_allocator: &UsbBusAllocator<B>) -> (CtapHid<'_, B>, Dispatch) {
    let (ctaphid_rq, ctaphid_rp) = HidInterchange::claim().unwrap();
    let ctaphid = CtapHid::new(bus_allocator, ctaphid_rq, 0u32)
        .implements_ctap1()
        .implements_ctap2()
        .implements_wink();
    let ctaphid_dispatch = Dispatch::new(ctaphid_rp);
    (ctaphid, ctaphid_dispatch)
}
