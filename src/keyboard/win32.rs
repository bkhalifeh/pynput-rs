// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The keyboard implementation for *Windows*.
//!
//! Sending events uses `SendInput`; monitoring uses a low-level
//! `WH_KEYBOARD_LL` hook running on a dedicated message-loop thread.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK,
    KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::{Key, KeyCode, KeyboardCallbacks, KeyboardImpl, KeyInput};
use crate::util::{Error, ListenerCore, ListenerHandle};
use crate::Result;

/// Maps a well-known [`Key`] to a Windows virtual key code.
fn vk(key: Key) -> u16 {
    match key {
        Key::Alt | Key::AltL => 0xA4,   // VK_LMENU
        Key::AltR | Key::AltGr => 0xA5, // VK_RMENU
        Key::Backspace => 0x08,
        Key::CapsLock => 0x14,
        Key::Cmd | Key::CmdL => 0x5B, // VK_LWIN
        Key::CmdR => 0x5C,            // VK_RWIN
        Key::Ctrl | Key::CtrlL => 0xA2, // VK_LCONTROL
        Key::CtrlR => 0xA3,           // VK_RCONTROL
        Key::Delete => 0x2E,
        Key::Down => 0x28,
        Key::End => 0x23,
        Key::Enter => 0x0D,
        Key::Esc => 0x1B,
        Key::F1 => 0x70,
        Key::F2 => 0x71,
        Key::F3 => 0x72,
        Key::F4 => 0x73,
        Key::F5 => 0x74,
        Key::F6 => 0x75,
        Key::F7 => 0x76,
        Key::F8 => 0x77,
        Key::F9 => 0x78,
        Key::F10 => 0x79,
        Key::F11 => 0x7A,
        Key::F12 => 0x7B,
        Key::F13 => 0x7C,
        Key::F14 => 0x7D,
        Key::F15 => 0x7E,
        Key::F16 => 0x7F,
        Key::F17 => 0x80,
        Key::F18 => 0x81,
        Key::F19 => 0x82,
        Key::F20 => 0x83,
        Key::Home => 0x24,
        Key::Left => 0x25,
        Key::PageDown => 0x22,
        Key::PageUp => 0x21,
        Key::Right => 0x27,
        Key::Shift | Key::ShiftL => 0xA0, // VK_LSHIFT
        Key::ShiftR => 0xA1,              // VK_RSHIFT
        Key::Space => 0x20,
        Key::Tab => 0x09,
        Key::Up => 0x26,
        Key::MediaPlayPause => 0xB3,
        Key::MediaVolumeMute => 0xAD,
        Key::MediaVolumeDown => 0xAE,
        Key::MediaVolumeUp => 0xAF,
        Key::MediaPrevious => 0xB1,
        Key::MediaNext => 0xB0,
        Key::Insert => 0x2D,
        Key::Menu => 0x5D, // VK_APPS
        Key::NumLock => 0x90,
        Key::Pause => 0x13,
        Key::PrintScreen => 0x2C,
        Key::ScrollLock => 0x91,
    }
}

fn special_from_vk(code: u32) -> Option<Key> {
    Key::ALL.into_iter().find(|&k| vk(k) as u32 == code)
}

pub struct Keyboard;

impl Keyboard {
    fn send(&self, vk_code: u16, scan: u16, unicode: bool, is_press: bool) -> Result<()> {
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if unicode {
            flags |= KEYEVENTF_UNICODE;
        }
        if !is_press {
            flags |= KEYEVENTF_KEYUP;
        }
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk_code),
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        if sent == 0 {
            Err(Error::Backend("SendInput failed".into()))
        } else {
            Ok(())
        }
    }
}

impl KeyboardImpl for Keyboard {
    fn create() -> Result<Self> {
        Ok(Keyboard)
    }

    fn key_value(&self, key: Key) -> KeyCode {
        KeyCode::from_vk(vk(key) as u32)
    }

    fn handle(
        &self,
        _modifiers: &HashSet<Key>,
        key: &KeyCode,
        is_press: bool,
    ) -> Result<()> {
        if let Some(code) = key.vk {
            return self.send(code as u16, 0, false, is_press);
        }
        if let Some(c) = key.char {
            let mut buf = [0u16; 2];
            let units = c.encode_utf16(&mut buf);
            for unit in units.iter() {
                self.send(0, *unit, true, is_press)?;
            }
            return Ok(());
        }
        Err(Error::InvalidKey(format!("{}", key)))
    }
}

pub(crate) fn lookup(key: Key) -> Option<KeyCode> {
    Some(KeyCode::from_vk(vk(key) as u32))
}

pub(crate) fn canonical_vk(key: Key) -> Option<u32> {
    lookup(key).and_then(|c| c.vk)
}

// ---------------------------------------------------------------------------
// Listener (WH_KEYBOARD_LL)
// ---------------------------------------------------------------------------

type Callbacks = (
    Box<dyn FnMut(Option<KeyInput>, bool) -> bool + Send>,
    Box<dyn FnMut(Option<KeyInput>, bool) -> bool + Send>,
);

struct HookState {
    callbacks: Mutex<Callbacks>,
    core: Arc<ListenerCore>,
}

static HOOK_STATE: OnceLock<HookState> = OnceLock::new();

fn vk_to_keyinput(code: u32) -> Option<KeyInput> {
    if let Some(key) = special_from_vk(code) {
        return Some(KeyInput::Key(key));
    }
    Some(KeyInput::Code(KeyCode::from_vk(code)))
}

unsafe extern "system" fn hook_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0 {
        if let Some(state) = HOOK_STATE.get() {
            let data = &*(l_param.0 as *const KBDLLHOOKSTRUCT);
            let injected = data.flags.0 & 0x10 != 0; // LLKHF_INJECTED
            let key = vk_to_keyinput(data.vkCode);
            let msg = w_param.0 as u32;
            let mut cb = state.callbacks.lock().unwrap();
            let keep = match msg {
                WM_KEYDOWN | WM_SYSKEYDOWN => (cb.0)(key, injected),
                WM_KEYUP | WM_SYSKEYUP => (cb.1)(key, injected),
                _ => true,
            };
            if !keep {
                state.core.set_running(false);
            }
        }
    }
    CallNextHookEx(HHOOK(0), n_code, w_param, l_param)
}

pub(crate) struct HookListener {
    core: Arc<ListenerCore>,
    thread_id: Arc<AtomicU32>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl ListenerHandle for HookListener {
    fn core(&self) -> &Arc<ListenerCore> {
        &self.core
    }

    fn stop(&self) {
        self.core.set_running(false);
        let tid = self.thread_id.load(Ordering::SeqCst);
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
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

    HOOK_STATE
        .set(HookState {
            callbacks: Mutex::new((on_press, on_release)),
            core: Arc::clone(&core),
        })
        .map_err(|_| Error::Backend("a keyboard listener is already running".into()))?;

    let thread_id = Arc::new(AtomicU32::new(0));
    let thread_core = Arc::clone(&core);
    let thread_id_set = Arc::clone(&thread_id);
    let handle = std::thread::spawn(move || {
        unsafe {
            thread_id_set.store(
                windows::Win32::System::Threading::GetCurrentThreadId(),
                Ordering::SeqCst,
            );
            let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0)
            {
                Ok(h) => h,
                Err(_) => {
                    thread_core.mark_ready();
                    return;
                }
            };
            thread_core.set_running(true);
            thread_core.mark_ready();

            let mut msg = MSG::default();
            while thread_core.running()
                && GetMessageW(&mut msg, None, 0, 0).as_bool()
            {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(hook);
            thread_core.set_running(false);
        }
    });

    Ok(Box::new(HookListener {
        core,
        thread_id,
        thread: Mutex::new(Some(handle)),
    }))
}
