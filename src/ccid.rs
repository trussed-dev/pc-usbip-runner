use apdu_dispatch::{
    dispatch::ApduDispatch,
    interchanges::{Contact, Contactless},
};
use interchange::Interchange as _;
use usb_device::bus::{UsbBus, UsbBusAllocator};
use usbd_ccid::Ccid;

pub fn setup<B: UsbBus>(
    bus_allocator: &UsbBusAllocator<B>,
) -> (Ccid<'_, B, Contact, 3072>, ApduDispatch) {
    let (ccid_rq, ccid_rp) = Contact::claim().unwrap();
    let ccid = Ccid::new(bus_allocator, ccid_rq, None);
    let apdu_dispatch = ApduDispatch::new(ccid_rp, Contactless::claim().unwrap().1);
    (ccid, apdu_dispatch)
}
