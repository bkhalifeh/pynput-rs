// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The mouse implementation for *macOS* (Quartz / Core Graphics).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventType, CGMouseButton, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use super::{Button, MouseCallbacks, MouseImpl};
use crate::util::{Error, ListenerCore, ListenerHandle};
use crate::Result;

fn source() -> Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| Error::Backend("failed to create CGEventSource".into()))
}

fn cg_button(button: Button) -> CGMouseButton {
    match button {
        Button::Left => CGMouseButton::Left,
        Button::Right => CGMouseButton::Right,
        Button::Middle | Button::Unknown => CGMouseButton::Center,
    }
}

fn down_type(button: Button) -> CGEventType {
    match button {
        Button::Left => CGEventType::LeftMouseDown,
        Button::Right => CGEventType::RightMouseDown,
        _ => CGEventType::OtherMouseDown,
    }
}

fn up_type(button: Button) -> CGEventType {
    match button {
        Button::Left => CGEventType::LeftMouseUp,
        Button::Right => CGEventType::RightMouseUp,
        _ => CGEventType::OtherMouseUp,
    }
}

pub struct Mouse;

impl Mouse {
    fn current_point(&self) -> Result<CGPoint> {
        let event = CGEvent::new(source()?)
            .map_err(|_| Error::Backend("CGEvent failed".into()))?;
        Ok(event.location())
    }
}

impl MouseImpl for Mouse {
    fn create() -> Result<Self> {
        let _ = source()?;
        Ok(Mouse)
    }

    fn position(&self) -> Result<(i32, i32)> {
        let p = self.current_point()?;
        Ok((p.x as i32, p.y as i32))
    }

    fn set_position(&self, x: i32, y: i32) -> Result<()> {
        let point = CGPoint::new(x as f64, y as f64);
        let event = CGEvent::new_mouse_event(
            source()?,
            CGEventType::MouseMoved,
            point,
            CGMouseButton::Left,
        )
        .map_err(|_| Error::Backend("CGEvent failed".into()))?;
        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn scroll(&self, dx: i32, dy: i32) -> Result<()> {
        let event = CGEvent::new_scroll_event(
            source()?,
            ScrollEventUnit::LINE,
            2,
            dy,
            dx,
            0,
        )
        .map_err(|_| Error::Backend("CGEvent scroll failed".into()))?;
        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn press(&self, button: Button) -> Result<()> {
        let point = self.current_point()?;
        let event = CGEvent::new_mouse_event(
            source()?,
            down_type(button),
            point,
            cg_button(button),
        )
        .map_err(|_| Error::Backend("CGEvent failed".into()))?;
        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn release(&self, button: Button) -> Result<()> {
        let point = self.current_point()?;
        let event = CGEvent::new_mouse_event(
            source()?,
            up_type(button),
            point,
            cg_button(button),
        )
        .map_err(|_| Error::Backend("CGEvent failed".into()))?;
        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Listener (CGEventTap)
// ---------------------------------------------------------------------------

struct TapState {
    callbacks: Mutex<MouseCallbacks>,
    core: Arc<ListenerCore>,
}

static TAP_STATE: OnceLock<TapState> = OnceLock::new();

pub(crate) struct TapListener {
    core: Arc<ListenerCore>,
    stop_flag: Arc<AtomicBool>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl ListenerHandle for TapListener {
    fn core(&self) -> &Arc<ListenerCore> {
        &self.core
    }

    fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.core.set_running(false);
        unsafe {
            core_foundation::runloop::CFRunLoopStop(
                core_foundation::runloop::CFRunLoopGetMain(),
            );
        }
    }

    fn join_boxed(self: Box<Self>) -> Result<()> {
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
        Ok(())
    }
}

pub(crate) fn spawn_listener(
    callbacks: MouseCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    let core = Arc::new(ListenerCore::new());
    TAP_STATE
        .set(TapState {
            callbacks: Mutex::new(callbacks),
            core: Arc::clone(&core),
        })
        .map_err(|_| Error::Backend("a mouse listener is already running".into()))?;

    let stop_flag = Arc::new(AtomicBool::new(false));
    let thread_core = Arc::clone(&core);
    let handle = std::thread::spawn(move || {
        use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};

        let current = CFRunLoop::get_current();
        let tap = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![
                CGEventType::MouseMoved,
                CGEventType::LeftMouseDown,
                CGEventType::LeftMouseUp,
                CGEventType::RightMouseDown,
                CGEventType::RightMouseUp,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseUp,
                CGEventType::ScrollWheel,
            ],
            |_proxy, event_type, event| {
                if let Some(state) = TAP_STATE.get() {
                    let p = event.location();
                    let (x, y) = (p.x as i32, p.y as i32);
                    let mut cb = state.callbacks.lock().unwrap();
                    let keep = match event_type {
                        CGEventType::MouseMoved => (cb.on_move)(x, y, false),
                        CGEventType::LeftMouseDown => {
                            (cb.on_click)(x, y, Button::Left, true, false)
                        }
                        CGEventType::LeftMouseUp => {
                            (cb.on_click)(x, y, Button::Left, false, false)
                        }
                        CGEventType::RightMouseDown => {
                            (cb.on_click)(x, y, Button::Right, true, false)
                        }
                        CGEventType::RightMouseUp => {
                            (cb.on_click)(x, y, Button::Right, false, false)
                        }
                        CGEventType::OtherMouseDown => {
                            (cb.on_click)(x, y, Button::Middle, true, false)
                        }
                        CGEventType::OtherMouseUp => {
                            (cb.on_click)(x, y, Button::Middle, false, false)
                        }
                        CGEventType::ScrollWheel => {
                            // Fields 11/12: vertical/horizontal scroll amounts.
                            let dy = event.get_integer_value_field(11) as i32;
                            let dx = event.get_integer_value_field(12) as i32;
                            (cb.on_scroll)(x, y, dx, dy, false)
                        }
                        _ => true,
                    };
                    if !keep {
                        state.core.set_running(false);
                    }
                }
                None
            },
        );
        let tap = match tap {
            Ok(t) => t,
            Err(_) => {
                thread_core.mark_ready();
                return;
            }
        };
        unsafe {
            let loop_source = tap
                .mach_port
                .create_runloop_source(0)
                .expect("runloop source");
            current.add_source(&loop_source, kCFRunLoopCommonModes);
            tap.enable();
            thread_core.set_running(true);
            thread_core.mark_ready();
            CFRunLoop::run_current();
        }
        thread_core.set_running(false);
    });

    Ok(Box::new(TapListener {
        core,
        stop_flag,
        thread: Mutex::new(Some(handle)),
    }))
}
