# trussed-usbip

This crate facilitates simulation of Trussed devices using USB/IP.
It should only be used for development and testing.

Remarks:
- Requires multiple `usbip attach` calls to make it work [1].
- Works best with CTAPHID.  CCID is supported but often unstable.

[1] https://github.com/Sawchord/usbip-device#known-bugs

## Examples

[`examples/dummy.rs`](`examples/dummy.rs`) contains a very simple example that
shows how to run a simulated Trussed device.
For a more complex example, see the [usbip runner][] of the Nitrokey 3 that
provides all features of the Nitrokey 3.

[usbip runner]: https://github.com/Nitrokey/nitrokey-3-firmware/tree/main/runners/usbip

## Setup

USB/IP tools are required to work, as well as kernel supporting it.

On Fedora these could be installed with:
```
make setup-fedora
```

## Run 

Simulation starts USB/IP server, which can be connected to with the USB/IP tools. 
1. Make sure `vhci-hcd` module is loaded
2. Run simulation app
3. Attach to the simulated device (2 times if needed) 

This series of steps is scripted in the Makefile, thus it is sufficient to call:
```
make 
```

Stop execution with:
``` 
make stop
```

Warning: in some cases simulation can sometimes cause kernel faults, which makes the system it is running unstable.
