// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The keyboard implementation for *macOS* (Quartz / Core Graphics).
//!
//! Sending events posts `CGEvent`s; monitoring installs a `CGEventTap` on a
//! dedicated run-loop thread. Monitoring requires the process to be trusted for
//! accessibility (System Settings → Privacy & Security → Accessibility).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventType, CGKeyCode,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

use super::{Key, KeyCode, KeyboardCallbacks, KeyboardImpl, KeyInput};
use crate::util::{Error, ListenerCore, ListenerHandle};
use crate::Result;

/// Maps a well-known [`Key`] to a macOS virtual key code.
fn keycode(key: Key) -> u16 {
    match key {
        Key::Alt | Key::AltL => 0x3A,
        Key::AltR | Key::AltGr => 0x3D,
        Key::Backspace => 0x33,
        Key::CapsLock => 0x39,
        Key::Cmd | Key::CmdL => 0x37,
        Key::CmdR => 0x36,
        Key::Ctrl | Key::CtrlL => 0x3B,
        Key::CtrlR => 0x3E,
        Key::Delete => 0x75,
        Key::Down => 0x7D,
        Key::End => 0x77,
        Key::Enter => 0x24,
        Key::Esc => 0x35,
        Key::F1 => 0x7A,
        Key::F2 => 0x78,
        Key::F3 => 0x63,
        Key::F4 => 0x76,
        Key::F5 => 0x60,
        Key::F6 => 0x61,
        Key::F7 => 0x62,
        Key::F8 => 0x64,
        Key::F9 => 0x65,
        Key::F10 => 0x6D,
        Key::F11 => 0x67,
        Key::F12 => 0x6F,
        Key::F13 => 0x69,
        Key::F14 => 0x6B,
        Key::F15 => 0x71,
        Key::F16 => 0x6A,
        Key::F17 => 0x40,
        Key::F18 => 0x4F,
        Key::F19 => 0x50,
        Key::F20 => 0x5A,
        Key::Home => 0x73,
        Key::Left => 0x7B,
        Key::PageDown => 0x79,
        Key::PageUp => 0x74,
        Key::Right => 0x7C,
        Key::Shift | Key::ShiftL => 0x38,
        Key::ShiftR => 0x3C,
        Key::Space => 0x31,
        Key::Tab => 0x30,
        Key::Up => 0x7E,
        // Media keys are delivered as NSSystemDefined events on macOS and have
        // no standard CG virtual key code; they are best-effort only.
        Key::MediaPlayPause => 0x0,
        Key::MediaVolumeMute => 0x4A,
        Key::MediaVolumeDown => 0x49,
        Key::MediaVolumeUp => 0x48,
        Key::MediaPrevious => 0x0,
        Key::MediaNext => 0x0,
        Key::Insert => 0x72, // Help
        Key::Menu => 0x6E,
        Key::NumLock => 0x47,
        Key::Pause => 0x0,
        Key::PrintScreen => 0x0,
        Key::ScrollLock => 0x0,
    }
}

fn special_from_keycode(code: u16) -> Option<Key> {
    Key::ALL.into_iter().find(|&k| keycode(k) == code)
}

fn source() -> Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| Error::Backend("failed to create CGEventSource".into()))
}

pub struct Keyboard;

impl KeyboardImpl for Keyboard {
    fn create() -> Result<Self> {
        // Verify we can create an event source up front.
        let _ = source()?;
        Ok(Keyboard)
    }

    fn key_value(&self, key: Key) -> KeyCode {
        KeyCode::from_vk(keycode(key) as u32)
    }

    fn handle(
        &self,
        _modifiers: &HashSet<Key>,
        key: &KeyCode,
        is_press: bool,
    ) -> Result<()> {
        let src = source()?;

        if let Some(vk) = key.vk {
            let event =
                CGEvent::new_keyboard_event(src, vk as CGKeyCode, is_press)
                    .map_err(|_| Error::Backend("CGEvent failed".into()))?;
            event.post(CGEventTapLocation::HID);
            return Ok(());
        }

        if let Some(c) = key.char {
            let event = CGEvent::new_keyboard_event(src, 0, is_press)
                .map_err(|_| Error::Backend("CGEvent failed".into()))?;
            let s = c.to_string();
            event.set_string(&s);
            event.post(CGEventTapLocation::HID);
            return Ok(());
        }

        Err(Error::InvalidKey(format!("{}", key)))
    }
}

pub(crate) fn lookup(key: Key) -> Option<KeyCode> {
    Some(KeyCode::from_vk(keycode(key) as u32))
}

pub(crate) fn canonical_vk(key: Key) -> Option<u32> {
    lookup(key).and_then(|c| c.vk)
}

// ---------------------------------------------------------------------------
// Listener (CGEventTap)
// ---------------------------------------------------------------------------

type Callbacks = (
    Box<dyn FnMut(Option<KeyInput>, bool) -> bool + Send>,
    Box<dyn FnMut(Option<KeyInput>, bool) -> bool + Send>,
);

struct TapState {
    callbacks: Mutex<Callbacks>,
    core: Arc<ListenerCore>,
}

static TAP_STATE: OnceLock<TapState> = OnceLock::new();

fn keycode_to_keyinput(code: u16) -> Option<KeyInput> {
    if let Some(key) = special_from_keycode(code) {
        return Some(KeyInput::Key(key));
    }
    Some(KeyInput::Code(KeyCode::from_vk(code as u32)))
}

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
    callbacks: KeyboardCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    let core = Arc::new(ListenerCore::new());
    let KeyboardCallbacks {
        on_press,
        on_release,
        ..
    } = callbacks;
    TAP_STATE
        .set(TapState {
            callbacks: Mutex::new((on_press, on_release)),
            core: Arc::clone(&core),
        })
        .map_err(|_| Error::Backend("a keyboard listener is already running".into()))?;

    let stop_flag = Arc::new(AtomicBool::new(false));
    let thread_core = Arc::clone(&core);
    let handle = std::thread::spawn(move || {
        use core_foundation::runloop::{
            kCFRunLoopCommonModes, CFRunLoop,
        };

        let current = CFRunLoop::get_current();
        let tap = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::KeyDown, CGEventType::KeyUp],
            |_proxy, event_type, event| {
                if let Some(state) = TAP_STATE.get() {
                    let code = event.get_integer_value_field(9) as u16; // kCGKeyboardEventKeycode
                    let key = keycode_to_keyinput(code);
                    let mut cb = state.callbacks.lock().unwrap();
                    let keep = match event_type {
                        CGEventType::KeyDown => (cb.0)(key, false),
                        CGEventType::KeyUp => (cb.1)(key, false),
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
