// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The mouse implementation for *Windows*.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetMessageW,
    PostThreadMessageW, SetCursorPos, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, HHOOK, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

use super::{Button, MouseCallbacks, MouseImpl};
use crate::util::{Error, ListenerCore, ListenerHandle};
use crate::Result;

const WHEEL_DELTA: i32 = 120;

pub struct Mouse;

impl Mouse {
    fn send(&self, dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> Result<()> {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data as u32,
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

impl MouseImpl for Mouse {
    fn create() -> Result<Self> {
        Ok(Mouse)
    }

    fn position(&self) -> Result<(i32, i32)> {
        let mut point = POINT::default();
        unsafe {
            GetCursorPos(&mut point)
                .map_err(|e| Error::Backend(e.to_string()))?;
        }
        Ok((point.x, point.y))
    }

    fn set_position(&self, x: i32, y: i32) -> Result<()> {
        unsafe {
            SetCursorPos(x, y).map_err(|e| Error::Backend(e.to_string()))
        }
    }

    fn scroll(&self, dx: i32, dy: i32) -> Result<()> {
        if dy != 0 {
            self.send(0, 0, dy * WHEEL_DELTA, MOUSEEVENTF_WHEEL)?;
        }
        if dx != 0 {
            self.send(0, 0, dx * WHEEL_DELTA, MOUSEEVENTF_HWHEEL)?;
        }
        Ok(())
    }

    fn press(&self, button: Button) -> Result<()> {
        let flag = match button {
            Button::Left => MOUSEEVENTF_LEFTDOWN,
            Button::Middle => MOUSEEVENTF_MIDDLEDOWN,
            Button::Right => MOUSEEVENTF_RIGHTDOWN,
            Button::Unknown => return Err(Error::Backend("unknown button".into())),
        };
        self.send(0, 0, 0, flag)
    }

    fn release(&self, button: Button) -> Result<()> {
        let flag = match button {
            Button::Left => MOUSEEVENTF_LEFTUP,
            Button::Middle => MOUSEEVENTF_MIDDLEUP,
            Button::Right => MOUSEEVENTF_RIGHTUP,
            Button::Unknown => return Err(Error::Backend("unknown button".into())),
        };
        self.send(0, 0, 0, flag)
    }
}

// ---------------------------------------------------------------------------
// Listener (WH_MOUSE_LL)
// ---------------------------------------------------------------------------

struct HookState {
    callbacks: Mutex<MouseCallbacks>,
    core: Arc<ListenerCore>,
}

static HOOK_STATE: OnceLock<HookState> = OnceLock::new();

unsafe extern "system" fn hook_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0 {
        if let Some(state) = HOOK_STATE.get() {
            let data = &*(l_param.0 as *const MSLLHOOKSTRUCT);
            let x = data.pt.x;
            let y = data.pt.y;
            let injected = data.flags & 0x01 != 0; // LLMHF_INJECTED
            let high_word = ((data.mouseData >> 16) & 0xFFFF) as i16 as i32;
            let mut cb = state.callbacks.lock().unwrap();
            let keep = match w_param.0 as u32 {
                WM_MOUSEMOVE => (cb.on_move)(x, y, injected),
                WM_LBUTTONDOWN => (cb.on_click)(x, y, Button::Left, true, injected),
                WM_LBUTTONUP => (cb.on_click)(x, y, Button::Left, false, injected),
                WM_RBUTTONDOWN => (cb.on_click)(x, y, Button::Right, true, injected),
                WM_RBUTTONUP => (cb.on_click)(x, y, Button::Right, false, injected),
                WM_MBUTTONDOWN => (cb.on_click)(x, y, Button::Middle, true, injected),
                WM_MBUTTONUP => (cb.on_click)(x, y, Button::Middle, false, injected),
                WM_MOUSEWHEEL => {
                    (cb.on_scroll)(x, y, 0, high_word / WHEEL_DELTA, injected)
                }
                WM_MOUSEHWHEEL => {
                    (cb.on_scroll)(x, y, high_word / WHEEL_DELTA, 0, injected)
                }
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
    callbacks: MouseCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    let core = Arc::new(ListenerCore::new());
    HOOK_STATE
        .set(HookState {
            callbacks: Mutex::new(callbacks),
            core: Arc::clone(&core),
        })
        .map_err(|_| Error::Backend("a mouse listener is already running".into()))?;

    let thread_id = Arc::new(AtomicU32::new(0));
    let thread_core = Arc::clone(&core);
    let thread_id_set = Arc::clone(&thread_id);
    let handle = std::thread::spawn(move || unsafe {
        thread_id_set.store(
            windows::Win32::System::Threading::GetCurrentThreadId(),
            Ordering::SeqCst,
        );
        let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), None, 0) {
            Ok(h) => h,
            Err(_) => {
                thread_core.mark_ready();
                return;
            }
        };
        thread_core.set_running(true);
        thread_core.mark_ready();

        let mut msg = MSG::default();
        while thread_core.running() && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
        thread_core.set_running(false);
    });

    Ok(Box::new(HookListener {
        core,
        thread_id,
        thread: Mutex::new(Some(handle)),
    }))
}
