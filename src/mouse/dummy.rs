// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! A fallback mouse backend for unsupported platforms.

use super::{Button, MouseCallbacks, MouseImpl};
use crate::util::{Error, ListenerHandle};
use crate::Result;

pub struct Mouse;

fn unsupported<T>() -> Result<T> {
    Err(Error::Unsupported("this platform is not supported".into()))
}

impl MouseImpl for Mouse {
    fn create() -> Result<Self> {
        unsupported()
    }
    fn position(&self) -> Result<(i32, i32)> {
        unsupported()
    }
    fn set_position(&self, _x: i32, _y: i32) -> Result<()> {
        unsupported()
    }
    fn scroll(&self, _dx: i32, _dy: i32) -> Result<()> {
        unsupported()
    }
    fn press(&self, _button: Button) -> Result<()> {
        unsupported()
    }
    fn release(&self, _button: Button) -> Result<()> {
        unsupported()
    }
}

pub(crate) fn spawn_listener(
    _callbacks: MouseCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    unsupported()
}
