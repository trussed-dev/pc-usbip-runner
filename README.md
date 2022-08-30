# USB/IP Simulation

This runner allows using USB/IP as a means to simulate device connection
to the OS, and should allow faster development of the embedded applications.

Remarks:
- Extensible with CTAP apps: currently FIDO and Admin are active;
- Does not work with Firefox at the moment;
- Allows to inject own FIDO certificates, and device properties;
- Requires multiple `usbip attach` calls to make it work [1].

[1] https://github.com/Sawchord/usbip-device#known-bugs

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
