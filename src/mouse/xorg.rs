// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The mouse implementation for *Xorg*.

use super::{Button, MouseCallbacks, MouseImpl};
use crate::util::xorg::{
    record_listen, Display, Grab, RecordedEvent, BUTTON_PRESS, BUTTON_RELEASE,
    MOTION_NOTIFY,
};
use crate::util::{Error, ListenerHandle};
use crate::Result;

const SCROLL_UP: u8 = 4;
const SCROLL_DOWN: u8 = 5;
const SCROLL_LEFT: u8 = 6;
const SCROLL_RIGHT: u8 = 7;

fn button_value(button: Button) -> u8 {
    match button {
        Button::Left => 1,
        Button::Middle => 2,
        Button::Right => 3,
        Button::Unknown => 0,
    }
}

/// Maps an *X* button detail back to a [`Button`].
pub(crate) fn button_from_detail(detail: u8) -> Button {
    match detail {
        1 => Button::Left,
        2 => Button::Middle,
        3 => Button::Right,
        _ => Button::Unknown,
    }
}

fn check_bounds(value: i32) -> Result<i16> {
    if (-0x8000..=0x7FFF).contains(&value) {
        Ok(value as i16)
    } else {
        Err(Error::Backend(format!("value out of bounds: {}", value)))
    }
}

pub struct Mouse {
    display: Display,
}

impl Mouse {
    fn click_button(&self, detail: u8, count: u32) -> Result<()> {
        for _ in 0..count {
            self.display.fake_button(true, detail)?;
            self.display.fake_button(false, detail)?;
        }
        Ok(())
    }
}

impl MouseImpl for Mouse {
    fn create() -> Result<Self> {
        Ok(Mouse {
            display: Display::open()?,
        })
    }

    fn position(&self) -> Result<(i32, i32)> {
        let (x, y) = self.display.query_pointer()?;
        Ok((x as i32, y as i32))
    }

    fn set_position(&self, x: i32, y: i32) -> Result<()> {
        let px = check_bounds(x)?;
        let py = check_bounds(y)?;
        self.display.warp_pointer(px, py)
    }

    fn scroll(&self, dx: i32, dy: i32) -> Result<()> {
        if dy != 0 {
            let button = if dy > 0 { SCROLL_UP } else { SCROLL_DOWN };
            self.click_button(button, dy.unsigned_abs())?;
        }
        if dx != 0 {
            let button = if dx > 0 { SCROLL_RIGHT } else { SCROLL_LEFT };
            self.click_button(button, dx.unsigned_abs())?;
        }
        Ok(())
    }

    fn press(&self, button: Button) -> Result<()> {
        self.display.fake_button(true, button_value(button))
    }

    fn release(&self, button: Button) -> Result<()> {
        self.display.fake_button(false, button_value(button))
    }
}

/// Scroll directions for the scroll button detail codes.
fn scroll_vector(detail: u8) -> Option<(i32, i32)> {
    match detail {
        SCROLL_UP => Some((0, 1)),
        SCROLL_DOWN => Some((0, -1)),
        SCROLL_RIGHT => Some((1, 0)),
        SCROLL_LEFT => Some((-1, 0)),
        _ => None,
    }
}

pub(crate) fn spawn_listener(
    callbacks: MouseCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    let grab = if callbacks.suppress {
        Grab::Pointer
    } else {
        Grab::None
    };
    let MouseCallbacks {
        mut on_move,
        mut on_click,
        mut on_scroll,
        ..
    } = callbacks;

    record_listen(grab, (BUTTON_PRESS, MOTION_NOTIFY), move || {
        Ok(move |event: &RecordedEvent| -> bool {
            let px = event.root_x as i32;
            let py = event.root_y as i32;
            match event.type_ {
                BUTTON_PRESS => {
                    if let Some((dx, dy)) = scroll_vector(event.detail) {
                        on_scroll(px, py, dx, dy, event.injected)
                    } else {
                        on_click(
                            px,
                            py,
                            button_from_detail(event.detail),
                            true,
                            event.injected,
                        )
                    }
                }
                BUTTON_RELEASE => {
                    if scroll_vector(event.detail).is_some() {
                        true
                    } else {
                        on_click(
                            px,
                            py,
                            button_from_detail(event.detail),
                            false,
                            event.injected,
                        )
                    }
                }
                _ => on_move(px, py, event.injected),
            }
        })
    })
}
