// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! General utility types shared between the platform backends.

#[cfg(target_os = "linux")]
pub(crate) mod xorg;

/// Decides whether the *uinput*/*evdev* backend should be used on Linux.
///
/// `kind` is `"KEYBOARD"` or `"MOUSE"`. The decision is:
///
/// 1. `PYNPUT_BACKEND_<KIND>` if set (`xorg` or `uinput`),
/// 2. otherwise `PYNPUT_BACKEND` if set,
/// 3. otherwise `uinput` on a Wayland session, `xorg` elsewhere.
#[cfg(target_os = "linux")]
pub(crate) fn use_uinput(kind: &str) -> bool {
    let pick = std::env::var(format!("PYNPUT_BACKEND_{}", kind))
        .ok()
        .or_else(|| std::env::var("PYNPUT_BACKEND").ok());
    match pick.as_deref() {
        Some("uinput") => true,
        Some("xorg") => false,
        _ => {
            let wayland_display = std::env::var("WAYLAND_DISPLAY")
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            let wayland_session = std::env::var("XDG_SESSION_TYPE")
                .map(|v| v == "wayland")
                .unwrap_or(false);
            wayland_display || wayland_session
        }
    }
}

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// The error type used throughout the crate.
///
/// It mirrors the exceptions raised by the original *pynput* library.
#[derive(Debug)]
pub enum Error {
    /// The exception raised when an invalid `key` parameter is passed to either
    /// [`press`](crate::keyboard::Controller::press) or
    /// [`release`](crate::keyboard::Controller::release).
    InvalidKey(String),

    /// The exception raised when an invalid character is encountered in the
    /// string passed to [`type_str`](crate::keyboard::Controller::type_str).
    ///
    /// The first field is the index of the character in the string, the second
    /// the character.
    InvalidCharacter(usize, char),

    /// Raised when a string that should represent a single key does not have a
    /// length of exactly one, or when a hotkey description cannot be parsed.
    InvalidString(String),

    /// A platform backend failed to initialise or perform an operation.
    Backend(String),

    /// The current platform is not supported.
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidKey(k) => write!(f, "invalid key: {}", k),
            Error::InvalidCharacter(i, c) => {
                write!(f, "invalid character {:?} at index {}", c, i)
            }
            Error::InvalidString(s) => write!(f, "invalid string: {}", s),
            Error::Backend(s) => write!(f, "backend error: {}", s),
            Error::Unsupported(s) => write!(f, "unsupported platform: {}", s),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Backend(e.to_string())
    }
}

/// Shared synchronisation state for the platform event listeners.
///
/// This is the Rust counterpart of the original `AbstractListener`. It tracks
/// whether the listener is running and provides a "ready" barrier so callers
/// can wait until the platform has finished initialising.
pub struct ListenerCore {
    running: AtomicBool,
    ready: Mutex<bool>,
    ready_cond: Condvar,
}

impl Default for ListenerCore {
    fn default() -> Self {
        Self::new()
    }
}

impl ListenerCore {
    pub fn new() -> Self {
        ListenerCore {
            running: AtomicBool::new(false),
            ready: Mutex::new(false),
            ready_cond: Condvar::new(),
        }
    }

    /// Whether the listener is currently running.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub(crate) fn set_running(&self, value: bool) {
        self.running.store(value, Ordering::SeqCst);
    }

    /// Marks this listener as ready to receive events.
    pub(crate) fn mark_ready(&self) {
        let mut ready = self.ready.lock().unwrap();
        *ready = true;
        self.ready_cond.notify_all();
    }

    /// Blocks until [`mark_ready`](Self::mark_ready) has been called.
    pub fn wait(&self) {
        let mut ready = self.ready.lock().unwrap();
        while !*ready {
            ready = self.ready_cond.wait(ready).unwrap();
        }
    }
}

/// A handle to a running platform listener, owned by the public listener types.
pub trait ListenerHandle: Send {
    /// The shared synchronisation state.
    fn core(&self) -> &Arc<ListenerCore>;

    /// Signals the listener to stop. May be called multiple times.
    fn stop(&self);

    /// Waits for the listener thread to terminate, consuming the handle.
    fn join_boxed(self: Box<Self>) -> crate::Result<()>;
}
