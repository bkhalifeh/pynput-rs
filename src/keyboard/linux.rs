// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! Runtime backend dispatch for Linux.
//!
//! Both the X11 and the *uinput*/*evdev* keyboard backends are compiled in; the
//! one used is chosen at runtime (see [`crate::util::use_uinput`]): X11 by
//! default, evdev on a Wayland session, overridable with `PYNPUT_BACKEND` /
//! `PYNPUT_BACKEND_KEYBOARD`.

use std::collections::HashSet;

use super::{xorg, uinput, Key, KeyCode, KeyboardCallbacks, KeyboardImpl};
use crate::util::{use_uinput, ListenerHandle};
use crate::Result;

fn uinput_selected() -> bool {
    use_uinput("KEYBOARD")
}

pub enum Keyboard {
    Xorg(xorg::Keyboard),
    Uinput(uinput::Keyboard),
}

impl KeyboardImpl for Keyboard {
    fn create() -> Result<Self> {
        if uinput_selected() {
            Ok(Keyboard::Uinput(uinput::Keyboard::create()?))
        } else {
            Ok(Keyboard::Xorg(xorg::Keyboard::create()?))
        }
    }

    fn key_value(&self, key: Key) -> KeyCode {
        match self {
            Keyboard::Xorg(k) => k.key_value(key),
            Keyboard::Uinput(k) => k.key_value(key),
        }
    }

    fn handle(
        &self,
        modifiers: &HashSet<Key>,
        key: &KeyCode,
        is_press: bool,
    ) -> Result<()> {
        match self {
            Keyboard::Xorg(k) => k.handle(modifiers, key, is_press),
            Keyboard::Uinput(k) => k.handle(modifiers, key, is_press),
        }
    }
}

pub(crate) fn canonical_vk(key: Key) -> Option<u32> {
    if uinput_selected() {
        uinput::canonical_vk(key)
    } else {
        xorg::canonical_vk(key)
    }
}

pub(crate) fn spawn_listener(
    callbacks: KeyboardCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    if uinput_selected() {
        uinput::spawn_listener(callbacks)
    } else {
        xorg::spawn_listener(callbacks)
    }
}
