//! Demonstrates controlling the mouse and keyboard.
//!
//! Run with: `cargo run --example control`

use pynput::keyboard::Controller as Keyboard;
use pynput::mouse::{Button, Controller as Mouse};

fn main() -> pynput::Result<()> {
    let mouse = Mouse::new()?;
    let (x, y) = mouse.position()?;
    println!("pointer at ({}, {})", x, y);
    mouse.set_position(x + 20, y + 20)?;
    mouse.set_position(x, y)?;
    println!("nudged the pointer and moved it back");

    let kbd = Keyboard::new()?;
    println!("typing in 2 seconds; focus a text field...");
    std::thread::sleep(std::time::Duration::from_secs(2));
    kbd.type_str("Hello from pynput-rs!")?;
    let _ = (Button::Left, &mouse);
    Ok(())
}
