// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The mouse implementation for *uinput*/*evdev*.
//!
//! `uinput` can only synthesise *relative* pointer motion, so absolute
//! positioning ([`position`](super::Controller::position) and
//! [`set_position`](super::Controller::set_position)) is unsupported; use
//! [`move_rel`](super::Controller::move_rel)-style relative motion via the
//! buttons and scroll instead. There is no global pointer monitor over
//! `uinput`; use the X11 backend for that.

use std::sync::Mutex;

use evdev::{
    AttributeSet, EventType, InputEvent, Key as EKey, RelativeAxisType,
};

use super::{Button, MouseCallbacks, MouseImpl};
use crate::util::{Error, ListenerHandle};
use crate::Result;

fn button_key(button: Button) -> Option<EKey> {
    match button {
        Button::Left => Some(EKey::BTN_LEFT),
        Button::Middle => Some(EKey::BTN_MIDDLE),
        Button::Right => Some(EKey::BTN_RIGHT),
        Button::Unknown => None,
    }
}

pub struct Mouse {
    device: Mutex<evdev::uinput::VirtualDevice>,
}

impl Mouse {
    fn emit(&self, type_: EventType, code: u16, value: i32) -> Result<()> {
        let mut device = self.device.lock().unwrap();
        device
            .emit(&[InputEvent::new(type_, code, value)])
            .map_err(|e| Error::Backend(e.to_string()))
    }
}

impl MouseImpl for Mouse {
    fn create() -> Result<Self> {
        let mut buttons = AttributeSet::<EKey>::new();
        buttons.insert(EKey::BTN_LEFT);
        buttons.insert(EKey::BTN_MIDDLE);
        buttons.insert(EKey::BTN_RIGHT);

        let mut axes = AttributeSet::<RelativeAxisType>::new();
        axes.insert(RelativeAxisType::REL_X);
        axes.insert(RelativeAxisType::REL_Y);
        axes.insert(RelativeAxisType::REL_WHEEL);
        axes.insert(RelativeAxisType::REL_HWHEEL);

        let device = evdev::uinput::VirtualDeviceBuilder::new()
            .map_err(|e| Error::Backend(e.to_string()))?
            .name("pynput-rs-mouse")
            .with_keys(&buttons)
            .map_err(|e| Error::Backend(e.to_string()))?
            .with_relative_axes(&axes)
            .map_err(|e| Error::Backend(e.to_string()))?
            .build()
            .map_err(|e| Error::Backend(e.to_string()))?;
        Ok(Mouse {
            device: Mutex::new(device),
        })
    }

    fn position(&self) -> Result<(i32, i32)> {
        Err(Error::Unsupported(
            "uinput cannot read the pointer position".into(),
        ))
    }

    fn set_position(&self, _x: i32, _y: i32) -> Result<()> {
        Err(Error::Unsupported(
            "uinput cannot set an absolute pointer position".into(),
        ))
    }

    fn scroll(&self, dx: i32, dy: i32) -> Result<()> {
        if dy != 0 {
            self.emit(EventType::RELATIVE, RelativeAxisType::REL_WHEEL.0, dy)?;
        }
        if dx != 0 {
            self.emit(EventType::RELATIVE, RelativeAxisType::REL_HWHEEL.0, dx)?;
        }
        Ok(())
    }

    fn press(&self, button: Button) -> Result<()> {
        let key = button_key(button)
            .ok_or_else(|| Error::Backend("unknown button".into()))?;
        self.emit(EventType::KEY, key.0, 1)
    }

    fn release(&self, button: Button) -> Result<()> {
        let key = button_key(button)
            .ok_or_else(|| Error::Backend("unknown button".into()))?;
        self.emit(EventType::KEY, key.0, 0)
    }
}

pub(crate) fn spawn_listener(
    _callbacks: MouseCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    Err(Error::Unsupported(
        "global mouse monitoring is not available over uinput; use the X11 \
         backend"
            .into(),
    ))
}
