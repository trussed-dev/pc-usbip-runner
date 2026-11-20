// Copyright (C) Florian Franzen <Florian.Franzen@gmail.com>
// SPDX-License-Identifier: Apache-2.0 or MIT

//! Loopback tests: a small USB device runs on `UsbIpBus` in a thread, and each test talks
//! to it as a USB/IP client over TCP, the way `usbip attach` would, but with every URB
//! under the test's control.

mod bulk;
mod control;
mod harness;
mod session;
