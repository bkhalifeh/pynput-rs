// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! Runtime backend dispatch for Linux.
//!
//! Both the X11 and the *uinput*/*evdev* mouse backends are compiled in; the one
//! used is chosen at runtime (see [`crate::util::use_uinput`]): X11 by default,
//! evdev on a Wayland session, overridable with `PYNPUT_BACKEND` /
//! `PYNPUT_BACKEND_MOUSE`.
//!
//! Note: the evdev backend can only emit *relative* motion and cannot read or
//! set an absolute pointer position, and it offers no pointer monitor. For full
//! mouse support under Wayland, force the X11 backend with
//! `PYNPUT_BACKEND_MOUSE=xorg` (events flow through XWayland), or use a
//! compositor-native protocol.

use super::{uinput, xorg, Button, MouseCallbacks, MouseImpl};
use crate::util::{use_uinput, ListenerHandle};
use crate::Result;

fn uinput_selected() -> bool {
    use_uinput("MOUSE")
}

pub enum Mouse {
    Xorg(xorg::Mouse),
    Uinput(uinput::Mouse),
}

impl MouseImpl for Mouse {
    fn create() -> Result<Self> {
        if uinput_selected() {
            Ok(Mouse::Uinput(uinput::Mouse::create()?))
        } else {
            Ok(Mouse::Xorg(xorg::Mouse::create()?))
        }
    }

    fn position(&self) -> Result<(i32, i32)> {
        match self {
            Mouse::Xorg(m) => m.position(),
            Mouse::Uinput(m) => m.position(),
        }
    }

    fn set_position(&self, x: i32, y: i32) -> Result<()> {
        match self {
            Mouse::Xorg(m) => m.set_position(x, y),
            Mouse::Uinput(m) => m.set_position(x, y),
        }
    }

    fn scroll(&self, dx: i32, dy: i32) -> Result<()> {
        match self {
            Mouse::Xorg(m) => m.scroll(dx, dy),
            Mouse::Uinput(m) => m.scroll(dx, dy),
        }
    }

    fn press(&self, button: Button) -> Result<()> {
        match self {
            Mouse::Xorg(m) => m.press(button),
            Mouse::Uinput(m) => m.press(button),
        }
    }

    fn release(&self, button: Button) -> Result<()> {
        match self {
            Mouse::Xorg(m) => m.release(button),
            Mouse::Uinput(m) => m.release(button),
        }
    }
}

pub(crate) fn spawn_listener(
    callbacks: MouseCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    if uinput_selected() {
        uinput::spawn_listener(callbacks)
    } else {
        xorg::spawn_listener(callbacks)
    }
}
