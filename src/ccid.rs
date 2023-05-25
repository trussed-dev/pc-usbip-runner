use apdu_dispatch::dispatch::ApduDispatch;
use apdu_dispatch::interchanges::Data;
use usb_device::bus::{UsbBus, UsbBusAllocator};
use usbd_ccid::Ccid;

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
