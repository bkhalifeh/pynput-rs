// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! A fallback keyboard backend for unsupported platforms.

use std::collections::HashSet;

use super::{Key, KeyCode, KeyboardCallbacks, KeyboardImpl};
use crate::util::{Error, ListenerHandle};
use crate::Result;

pub struct Keyboard;

impl KeyboardImpl for Keyboard {
    fn create() -> Result<Self> {
        Err(Error::Unsupported("this platform is not supported".into()))
    }

    fn key_value(&self, _key: Key) -> KeyCode {
        KeyCode::from_vk(0)
    }

    fn handle(
        &self,
        _modifiers: &HashSet<Key>,
        _key: &KeyCode,
        _is_press: bool,
    ) -> Result<()> {
        Err(Error::Unsupported("this platform is not supported".into()))
    }
}

pub(crate) fn lookup(_key: Key) -> Option<KeyCode> {
    None
}

pub(crate) fn canonical_vk(_key: Key) -> Option<u32> {
    None
}

pub(crate) fn spawn_listener(
    _callbacks: KeyboardCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    Err(Error::Unsupported("this platform is not supported".into()))
}
