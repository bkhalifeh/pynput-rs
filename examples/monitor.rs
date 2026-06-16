//! Monitors keyboard and mouse events until <esc> is pressed.
//!
//! Run with: `cargo run --example monitor`

use pynput::keyboard::{Key, KeyInput, Listener as KbdListener};
use pynput::mouse::Listener as MouseListener;

fn main() -> pynput::Result<()> {
    // The mouse listener is unavailable on some backends (e.g. uinput); start
    // it if we can, otherwise carry on with the keyboard only.
    let mouse = MouseListener::builder()
        .on_click(|x, y, button, pressed, injected| {
            println!(
                "click {:?} {} at ({}, {}) injected={}",
                button,
                if pressed { "down" } else { "up" },
                x,
                y,
                injected
            );
            true
        })
        .on_scroll(|_, _, dx, dy, _| {
            println!("scroll ({}, {})", dx, dy);
            true
        })
        .start();
    match &mouse {
        Ok(_) => println!("Listening for keyboard and mouse. Press <esc> to quit."),
        Err(e) => {
            println!("Mouse monitoring unavailable ({e}); keyboard only. Press <esc> to quit.")
        }
    }
    let keyboard = KbdListener::builder()
        .on_press(|key, injected| {
            println!("press {:?} injected={}", key, injected);
            // Stop when escape is pressed.
            !matches!(key, Some(KeyInput::Key(Key::Esc)))
        })
        .start()?;

    keyboard.join()?;
    if let Ok(mouse) = mouse {
        mouse.stop();
        mouse.join()?;
    }
    Ok(())
}
