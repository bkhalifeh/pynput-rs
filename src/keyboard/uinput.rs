// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The keyboard implementation for *uinput*/*evdev* (Linux, Wayland-friendly).
//!
//! Sending events requires write access to `/dev/uinput`; monitoring events
//! requires read access to the `/dev/input/event*` nodes. Both typically
//! require running as `root` or membership of the `input` group.

use std::collections::HashSet;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use evdev::{AttributeSet, EventType, InputEvent, Key as EKey};

use super::{Key, KeyCode, KeyboardCallbacks, KeyboardImpl, KeyInput};
use crate::util::{Error, ListenerCore, ListenerHandle};
use crate::Result;

/// Maps a well-known [`Key`] to a Linux input key code.
fn evdev_key(key: Key) -> EKey {
    match key {
        Key::Alt | Key::AltL => EKey::KEY_LEFTALT,
        Key::AltR | Key::AltGr => EKey::KEY_RIGHTALT,
        Key::Backspace => EKey::KEY_BACKSPACE,
        Key::CapsLock => EKey::KEY_CAPSLOCK,
        Key::Cmd | Key::CmdL => EKey::KEY_LEFTMETA,
        Key::CmdR => EKey::KEY_RIGHTMETA,
        Key::Ctrl | Key::CtrlL => EKey::KEY_LEFTCTRL,
        Key::CtrlR => EKey::KEY_RIGHTCTRL,
        Key::Delete => EKey::KEY_DELETE,
        Key::Down => EKey::KEY_DOWN,
        Key::End => EKey::KEY_END,
        Key::Enter => EKey::KEY_ENTER,
        Key::Esc => EKey::KEY_ESC,
        Key::F1 => EKey::KEY_F1,
        Key::F2 => EKey::KEY_F2,
        Key::F3 => EKey::KEY_F3,
        Key::F4 => EKey::KEY_F4,
        Key::F5 => EKey::KEY_F5,
        Key::F6 => EKey::KEY_F6,
        Key::F7 => EKey::KEY_F7,
        Key::F8 => EKey::KEY_F8,
        Key::F9 => EKey::KEY_F9,
        Key::F10 => EKey::KEY_F10,
        Key::F11 => EKey::KEY_F11,
        Key::F12 => EKey::KEY_F12,
        Key::F13 => EKey::KEY_F13,
        Key::F14 => EKey::KEY_F14,
        Key::F15 => EKey::KEY_F15,
        Key::F16 => EKey::KEY_F16,
        Key::F17 => EKey::KEY_F17,
        Key::F18 => EKey::KEY_F18,
        Key::F19 => EKey::KEY_F19,
        Key::F20 => EKey::KEY_F20,
        Key::Home => EKey::KEY_HOME,
        Key::Left => EKey::KEY_LEFT,
        Key::PageDown => EKey::KEY_PAGEDOWN,
        Key::PageUp => EKey::KEY_PAGEUP,
        Key::Right => EKey::KEY_RIGHT,
        Key::Shift | Key::ShiftL => EKey::KEY_LEFTSHIFT,
        Key::ShiftR => EKey::KEY_RIGHTSHIFT,
        Key::Space => EKey::KEY_SPACE,
        Key::Tab => EKey::KEY_TAB,
        Key::Up => EKey::KEY_UP,
        Key::MediaPlayPause => EKey::KEY_PLAYPAUSE,
        Key::MediaVolumeMute => EKey::KEY_MUTE,
        Key::MediaVolumeDown => EKey::KEY_VOLUMEDOWN,
        Key::MediaVolumeUp => EKey::KEY_VOLUMEUP,
        Key::MediaPrevious => EKey::KEY_PREVIOUSSONG,
        Key::MediaNext => EKey::KEY_NEXTSONG,
        Key::Insert => EKey::KEY_INSERT,
        Key::Menu => EKey::KEY_MENU,
        Key::NumLock => EKey::KEY_NUMLOCK,
        Key::Pause => EKey::KEY_PAUSE,
        Key::PrintScreen => EKey::KEY_SYSRQ,
        Key::ScrollLock => EKey::KEY_SCROLLLOCK,
    }
}

/// Maps a character to a US-layout key code and whether shift is required.
fn char_to_evdev(c: char) -> Option<(EKey, bool)> {
    let lower = |k: EKey| Some((k, false));
    let shift = |k: EKey| Some((k, true));
    match c {
        'a'..='z' => {
            let off = c as u8 - b'a';
            lower(EKey(EKey::KEY_A.0 + letter_offset(off)))
        }
        'A'..='Z' => {
            let off = c as u8 - b'A';
            shift(EKey(EKey::KEY_A.0 + letter_offset(off)))
        }
        '1' => lower(EKey::KEY_1),
        '2' => lower(EKey::KEY_2),
        '3' => lower(EKey::KEY_3),
        '4' => lower(EKey::KEY_4),
        '5' => lower(EKey::KEY_5),
        '6' => lower(EKey::KEY_6),
        '7' => lower(EKey::KEY_7),
        '8' => lower(EKey::KEY_8),
        '9' => lower(EKey::KEY_9),
        '0' => lower(EKey::KEY_0),
        '!' => shift(EKey::KEY_1),
        '@' => shift(EKey::KEY_2),
        '#' => shift(EKey::KEY_3),
        '$' => shift(EKey::KEY_4),
        '%' => shift(EKey::KEY_5),
        '^' => shift(EKey::KEY_6),
        '&' => shift(EKey::KEY_7),
        '*' => shift(EKey::KEY_8),
        '(' => shift(EKey::KEY_9),
        ')' => shift(EKey::KEY_0),
        ' ' => lower(EKey::KEY_SPACE),
        '-' => lower(EKey::KEY_MINUS),
        '_' => shift(EKey::KEY_MINUS),
        '=' => lower(EKey::KEY_EQUAL),
        '+' => shift(EKey::KEY_EQUAL),
        '[' => lower(EKey::KEY_LEFTBRACE),
        '{' => shift(EKey::KEY_LEFTBRACE),
        ']' => lower(EKey::KEY_RIGHTBRACE),
        '}' => shift(EKey::KEY_RIGHTBRACE),
        '\\' => lower(EKey::KEY_BACKSLASH),
        '|' => shift(EKey::KEY_BACKSLASH),
        ';' => lower(EKey::KEY_SEMICOLON),
        ':' => shift(EKey::KEY_SEMICOLON),
        '\'' => lower(EKey::KEY_APOSTROPHE),
        '"' => shift(EKey::KEY_APOSTROPHE),
        '`' => lower(EKey::KEY_GRAVE),
        '~' => shift(EKey::KEY_GRAVE),
        ',' => lower(EKey::KEY_COMMA),
        '<' => shift(EKey::KEY_COMMA),
        '.' => lower(EKey::KEY_DOT),
        '>' => shift(EKey::KEY_DOT),
        '/' => lower(EKey::KEY_SLASH),
        '?' => shift(EKey::KEY_SLASH),
        '\t' => lower(EKey::KEY_TAB),
        '\n' | '\r' => lower(EKey::KEY_ENTER),
        _ => None,
    }
}

/// The keyboard layout lays the letters out as a, b, c, ... so the evdev codes
/// are *not* contiguous; this maps an alphabetical offset to the code delta.
fn letter_offset(off: u8) -> u16 {
    // KEY_A=30 KEY_B=48 ... the scancodes are not alphabetical, so map
    // explicitly.
    const ORDER: [u16; 26] = [
        EKey::KEY_A.0,
        EKey::KEY_B.0,
        EKey::KEY_C.0,
        EKey::KEY_D.0,
        EKey::KEY_E.0,
        EKey::KEY_F.0,
        EKey::KEY_G.0,
        EKey::KEY_H.0,
        EKey::KEY_I.0,
        EKey::KEY_J.0,
        EKey::KEY_K.0,
        EKey::KEY_L.0,
        EKey::KEY_M.0,
        EKey::KEY_N.0,
        EKey::KEY_O.0,
        EKey::KEY_P.0,
        EKey::KEY_Q.0,
        EKey::KEY_R.0,
        EKey::KEY_S.0,
        EKey::KEY_T.0,
        EKey::KEY_U.0,
        EKey::KEY_V.0,
        EKey::KEY_W.0,
        EKey::KEY_X.0,
        EKey::KEY_Y.0,
        EKey::KEY_Z.0,
    ];
    ORDER[off as usize] - EKey::KEY_A.0
}

pub struct Keyboard {
    device: Mutex<evdev::uinput::VirtualDevice>,
}

impl Keyboard {
    fn emit(&self, code: u16, value: i32) -> Result<()> {
        let mut device = self.device.lock().unwrap();
        device
            .emit(&[InputEvent::new(EventType::KEY, code, value)])
            .map_err(|e| Error::Backend(e.to_string()))
    }
}

impl KeyboardImpl for Keyboard {
    fn create() -> Result<Self> {
        // Register the full key range so any key can be emitted.
        let mut keys = AttributeSet::<EKey>::new();
        for code in 1u16..=255 {
            keys.insert(EKey(code));
        }
        let device = evdev::uinput::VirtualDeviceBuilder::new()
            .map_err(|e| Error::Backend(e.to_string()))?
            .name("pynput-rs")
            .with_keys(&keys)
            .map_err(|e| Error::Backend(e.to_string()))?
            .build()
            .map_err(|e| Error::Backend(e.to_string()))?;
        Ok(Keyboard {
            device: Mutex::new(device),
        })
    }

    fn key_value(&self, key: Key) -> KeyCode {
        KeyCode::from_vk(evdev_key(key).0 as u32)
    }

    fn handle(
        &self,
        _modifiers: &HashSet<Key>,
        key: &KeyCode,
        is_press: bool,
    ) -> Result<()> {
        let value = if is_press { 1 } else { 0 };

        if let Some(vk) = key.vk {
            return self.emit(vk as u16, value);
        }

        if let Some(c) = key.char {
            let (ekey, needs_shift) = char_to_evdev(c)
                .ok_or_else(|| Error::InvalidKey(format!("{:?}", c)))?;
            if needs_shift {
                if is_press {
                    self.emit(EKey::KEY_LEFTSHIFT.0, 1)?;
                    self.emit(ekey.0, 1)?;
                } else {
                    self.emit(ekey.0, 0)?;
                    self.emit(EKey::KEY_LEFTSHIFT.0, 0)?;
                }
            } else {
                self.emit(ekey.0, value)?;
            }
            return Ok(());
        }

        Err(Error::InvalidKey(format!("{}", key)))
    }
}

pub(crate) fn lookup(key: Key) -> Option<KeyCode> {
    Some(KeyCode::from_vk(evdev_key(key).0 as u32))
}

pub(crate) fn canonical_vk(key: Key) -> Option<u32> {
    lookup(key).and_then(|c| c.vk)
}

/// Reverse maps an evdev key code to a [`KeyInput`] for the listener.
fn code_to_keyinput(code: u16) -> Option<KeyInput> {
    let ekey = EKey(code);
    for key in Key::ALL {
        if evdev_key(key) == ekey {
            return Some(KeyInput::Key(key));
        }
    }
    // Fall back on a virtual key code.
    Some(KeyInput::Code(KeyCode::from_vk(code as u32)))
}

pub(crate) fn spawn_listener(
    callbacks: KeyboardCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    let core = Arc::new(ListenerCore::new());
    let running = Arc::new(AtomicBool::new(true));

    // Collect keyboard-capable devices.
    let mut devices = Vec::new();
    for (_path, device) in evdev::enumerate() {
        if device
            .supported_keys()
            .map(|keys| keys.contains(EKey::KEY_ENTER))
            .unwrap_or(false)
        {
            devices.push(device);
        }
    }
    if devices.is_empty() {
        return Err(Error::Backend(
            "no readable keyboard devices found (need root or 'input' group)"
                .into(),
        ));
    }

    let KeyboardCallbacks {
        on_press,
        on_release,
        ..
    } = callbacks;
    let callbacks = Arc::new(Mutex::new((on_press, on_release)));

    core.set_running(true);
    let mut handles = Vec::new();
    for mut device in devices {
        // Switch to non-blocking reads so the loop can observe `running`.
        let fd = device.as_raw_fd();
        unsafe {
            let flags = libc_fcntl_getfl(fd);
            libc_fcntl_setfl(fd, flags | O_NONBLOCK);
        }
        let running = Arc::clone(&running);
        let core_run = Arc::clone(&core);
        let callbacks = Arc::clone(&callbacks);
        let handle = std::thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                match device.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            if event.event_type() != EventType::KEY {
                                continue;
                            }
                            let key = code_to_keyinput(event.code());
                            let mut cb = callbacks.lock().unwrap();
                            let keep = match event.value() {
                                1 => (cb.0)(key, false),
                                0 => (cb.1)(key, false),
                                _ => true, // autorepeat (value 2): ignore
                            };
                            if !keep {
                                running.store(false, Ordering::SeqCst);
                                core_run.set_running(false);
                            }
                        }
                    }
                    Err(e) if e.raw_os_error() == Some(EAGAIN) => {
                        std::thread::sleep(std::time::Duration::from_millis(8));
                    }
                    Err(_) => break,
                }
            }
        });
        handles.push(handle);
    }
    core.mark_ready();

    Ok(Box::new(EvdevListener {
        core,
        running,
        handles: Mutex::new(handles),
    }))
}

const O_NONBLOCK: i32 = 0o4000;
const EAGAIN: i32 = 11;

unsafe fn libc_fcntl_getfl(fd: i32) -> i32 {
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    const F_GETFL: i32 = 3;
    fcntl(fd, F_GETFL)
}

unsafe fn libc_fcntl_setfl(fd: i32, flags: i32) {
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    const F_SETFL: i32 = 4;
    fcntl(fd, F_SETFL, flags);
}

pub(crate) struct EvdevListener {
    core: Arc<ListenerCore>,
    running: Arc<AtomicBool>,
    handles: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl ListenerHandle for EvdevListener {
    fn core(&self) -> &Arc<ListenerCore> {
        &self.core
    }

    fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.core.set_running(false);
    }

    fn join_boxed(self: Box<Self>) -> Result<()> {
        let handles = std::mem::take(&mut *self.handles.lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
        Ok(())
    }
}
