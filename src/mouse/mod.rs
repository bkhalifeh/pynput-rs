// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The module containing mouse classes.

use crate::util::ListenerHandle;
use crate::Result;

// ---------------------------------------------------------------------------
// Backend selection
// ---------------------------------------------------------------------------

// On Linux both backends are compiled and selected at runtime (see
// `linux.rs`): X11 by default, evdev/uinput on Wayland.
#[cfg(target_os = "linux")]
mod uinput;
#[cfg(target_os = "linux")]
mod xorg;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "win32.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "darwin.rs"]
mod imp;
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
#[path = "dummy.rs"]
mod imp;

// ---------------------------------------------------------------------------
// Button
// ---------------------------------------------------------------------------

/// The various mouse buttons.
///
/// The actual values for these items differ between platforms; these are
/// guaranteed to be present everywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    /// An unknown button was pressed.
    Unknown,
    /// The left button.
    Left,
    /// The middle button.
    Middle,
    /// The right button.
    Right,
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

/// The platform dependent mouse implementation.
///
/// This trait is public only so that [`Controller`] can name its backend type
/// parameter; it is not intended to be implemented outside the crate.
pub trait MouseImpl: Send + Sync + 'static {
    fn create() -> Result<Self>
    where
        Self: Sized;
    fn position(&self) -> Result<(i32, i32)>;
    fn set_position(&self, x: i32, y: i32) -> Result<()>;
    fn scroll(&self, dx: i32, dy: i32) -> Result<()>;
    fn press(&self, button: Button) -> Result<()>;
    fn release(&self, button: Button) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Controller
// ---------------------------------------------------------------------------

/// A controller for sending virtual mouse events to the system.
pub struct Controller<I: MouseImpl = imp::Mouse> {
    imp: I,
}

impl Controller<imp::Mouse> {
    /// Creates a new controller, acquiring the platform backend.
    pub fn new() -> Result<Self> {
        Ok(Controller {
            imp: imp::Mouse::create()?,
        })
    }
}

impl<I: MouseImpl> Controller<I> {
    /// Creates a controller from an explicit backend (useful for testing).
    pub fn with_backend(imp: I) -> Self {
        Controller { imp }
    }

    /// The current position of the mouse pointer as `(x, y)`.
    pub fn position(&self) -> Result<(i32, i32)> {
        self.imp.position()
    }

    /// Moves the pointer to an absolute position.
    pub fn set_position(&self, x: i32, y: i32) -> Result<()> {
        self.imp.set_position(x, y)
    }

    /// Moves the pointer a number of pixels from its current position.
    pub fn move_rel(&self, dx: i32, dy: i32) -> Result<()> {
        let (x, y) = self.position()?;
        self.set_position(x + dx, y + dy)
    }

    /// Sends scroll events. The units of scrolling are undefined.
    pub fn scroll(&self, dx: i32, dy: i32) -> Result<()> {
        self.imp.scroll(dx, dy)
    }

    /// Emits a button press event at the current position.
    pub fn press(&self, button: Button) -> Result<()> {
        self.imp.press(button)
    }

    /// Emits a button release event at the current position.
    pub fn release(&self, button: Button) -> Result<()> {
        self.imp.release(button)
    }

    /// Emits a button click (press then release) event `count` times.
    pub fn click(&self, button: Button, count: u32) -> Result<()> {
        for _ in 0..count {
            self.press(button)?;
            self.release(button)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// Callback invoked on pointer movement: `(x, y, injected)`. Returning `false`
/// stops the listener.
pub type MoveCallback = Box<dyn FnMut(i32, i32, bool) -> bool + Send>;
/// Callback invoked on a button click: `(x, y, button, pressed, injected)`.
pub type ClickCallback = Box<dyn FnMut(i32, i32, Button, bool, bool) -> bool + Send>;
/// Callback invoked on scrolling: `(x, y, dx, dy, injected)`.
pub type ScrollCallback = Box<dyn FnMut(i32, i32, i32, i32, bool) -> bool + Send>;

#[allow(dead_code)] // some fields are unused by backends without a listener
pub(crate) struct MouseCallbacks {
    pub on_move: MoveCallback,
    pub on_click: ClickCallback,
    pub on_scroll: ScrollCallback,
    pub suppress: bool,
}

/// A builder for a mouse [`Listener`].
#[derive(Default)]
pub struct ListenerBuilder {
    on_move: Option<MoveCallback>,
    on_click: Option<ClickCallback>,
    on_scroll: Option<ScrollCallback>,
    suppress: bool,
}

impl ListenerBuilder {
    /// Sets the callback invoked when the pointer moves.
    pub fn on_move<F>(mut self, f: F) -> Self
    where
        F: FnMut(i32, i32, bool) -> bool + Send + 'static,
    {
        self.on_move = Some(Box::new(f));
        self
    }

    /// Sets the callback invoked when a button is clicked.
    pub fn on_click<F>(mut self, f: F) -> Self
    where
        F: FnMut(i32, i32, Button, bool, bool) -> bool + Send + 'static,
    {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Sets the callback invoked when the device is scrolled.
    pub fn on_scroll<F>(mut self, f: F) -> Self
    where
        F: FnMut(i32, i32, i32, i32, bool) -> bool + Send + 'static,
    {
        self.on_scroll = Some(Box::new(f));
        self
    }

    /// Whether to suppress events system wide.
    pub fn suppress(mut self, suppress: bool) -> Self {
        self.suppress = suppress;
        self
    }

    /// Starts the listener.
    pub fn start(self) -> Result<Listener> {
        let callbacks = MouseCallbacks {
            on_move: self.on_move.unwrap_or_else(|| Box::new(|_, _, _| true)),
            on_click: self
                .on_click
                .unwrap_or_else(|| Box::new(|_, _, _, _, _| true)),
            on_scroll: self
                .on_scroll
                .unwrap_or_else(|| Box::new(|_, _, _, _, _| true)),
            suppress: self.suppress,
        };
        Ok(Listener {
            handle: imp::spawn_listener(callbacks)?,
        })
    }
}

/// A listener for mouse events.
pub struct Listener {
    handle: Box<dyn ListenerHandle>,
}

impl Listener {
    /// Returns a new [`ListenerBuilder`].
    pub fn builder() -> ListenerBuilder {
        ListenerBuilder::default()
    }

    /// Blocks until the listener has finished initialising.
    pub fn wait(&self) {
        self.handle.core().wait();
    }

    /// Whether the listener is currently running.
    pub fn running(&self) -> bool {
        self.handle.core().running()
    }

    /// Stops listening for events.
    pub fn stop(&self) {
        self.handle.stop();
    }

    /// Waits for the listener thread to terminate.
    pub fn join(self) -> Result<()> {
        self.handle.join_boxed()
    }
}
