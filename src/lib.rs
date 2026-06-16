// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Lesser General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
// details.
//
// You should have received a copy of the GNU Lesser General Public License
// along with this program. If not, see <http://www.gnu.org/licenses/>.

//! # pynput
//!
//! This library allows you to control and monitor input devices. It is a Rust
//! port of the [pynput](https://github.com/moses-palmer/pynput) Python library
//! and keeps its terminology and structure.
//!
//! Currently, mouse and keyboard input and monitoring are supported.
//!
//! ## Controlling the mouse
//!
//! ```no_run
//! use pynput::mouse::{Button, Controller};
//!
//! let mouse = Controller::new().unwrap();
//! let (x, y) = mouse.position().unwrap();
//! mouse.set_position(x + 10, y + 10).unwrap();
//! mouse.click(Button::Left, 2).unwrap();
//! ```
//!
//! ## Controlling the keyboard
//!
//! ```no_run
//! use pynput::keyboard::{Key, Controller};
//!
//! let kbd = Controller::new().unwrap();
//! kbd.press(Key::Cmd).unwrap();
//! kbd.release(Key::Cmd).unwrap();
//! kbd.type_str("Hello World").unwrap();
//! ```
//!
//! ## Monitoring input
//!
//! ```no_run
//! use pynput::keyboard::Listener;
//!
//! let listener = Listener::builder()
//!     .on_press(|key, _injected| {
//!         println!("pressed {:?}", key);
//!         true
//!     })
//!     .start()
//!     .unwrap();
//! listener.join().unwrap();
//! ```

pub mod keyboard;
pub mod mouse;

mod util;

pub use util::Error;

/// The result type used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;
