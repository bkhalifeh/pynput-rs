# pynput (Rust)

This library lets you **control and monitor input devices** — the mouse and the
keyboard. It is a Rust port of the
[pynput](https://github.com/moses-palmer/pynput) Python library and keeps its
terminology and structure (`Controller`, `Listener`, `Key`, `KeyCode`,
`Button`, `HotKey`, `GlobalHotKeys`).

## Platforms

The operating-system family is selected at compile time:

| Target  | Backend(s)                | Controller | Listener |
| ------- | ------------------------- | ---------- | -------- |
| Linux   | `x11rb` **and** `evdev`   | ✅         | ✅       |
| Windows | `windows` crate           | ✅         | ✅       |
| macOS   | `core-graphics`           | ✅         | ✅       |

### Linux: X11 vs. evdev (chosen at runtime)

On Linux **both** the X11 and the `uinput`/`evdev` backends are compiled into a
single binary, and the one used is decided when `Controller::new()` /
`Listener::start()` runs:

1. `PYNPUT_BACKEND_KEYBOARD` / `PYNPUT_BACKEND_MOUSE` (value `xorg` or `uinput`),
2. otherwise `PYNPUT_BACKEND`,
3. otherwise: **`evdev` on a Wayland session** (detected via `WAYLAND_DISPLAY` /
   `XDG_SESSION_TYPE`), **`x11rb` otherwise**.

So a normal `cargo build` produces one binary that "does the right thing": X11
under X11, evdev under Wayland. Override per run, e.g.:

```sh
PYNPUT_BACKEND=xorg   ./my-app   # force X11
PYNPUT_BACKEND=uinput ./my-app   # force evdev
PYNPUT_BACKEND_MOUSE=xorg ./my-app  # X11 mouse, auto keyboard
```

The **evdev** backend needs access to `/dev/uinput` (to send) and
`/dev/input/event*` (to monitor) — typically `root` or membership of the
`input` group:

```sh
sudo usermod -aG input "$USER"   # then log out and back in
```

evdev limitations: the mouse backend emits *relative* motion only (no absolute
position) and has no pointer monitor. For full mouse support under Wayland,
force `PYNPUT_BACKEND_MOUSE=xorg` so mouse events flow through XWayland.

> **Note:** the Windows and macOS backends are written against the documented
> `windows` and `core-graphics` APIs but are compiled and exercised on their
> respective operating systems only. The X11 and `uinput` backends are built on
> Linux. Monitoring on macOS requires the process to be trusted for
> Accessibility.

## Controlling the mouse

```rust
use pynput::mouse::{Button, Controller};

let mouse = Controller::new()?;

// Read and set the pointer position.
let (x, y) = mouse.position()?;
mouse.set_position(x + 10, y + 10)?;
mouse.move_rel(5, -5)?;

// Click and scroll.
mouse.click(Button::Left, 2)?;
mouse.scroll(0, 2)?;
# Ok::<(), pynput::Error>(())
```

## Controlling the keyboard

```rust
use pynput::keyboard::{Key, Controller};

let kbd = Controller::new()?;

// Press and release a key.
kbd.press(Key::Cmd)?;
kbd.press('h')?;
kbd.release('h')?;
kbd.release(Key::Cmd)?;

// Hold keys for the duration of a closure.
use pynput::keyboard::Pressable;
kbd.pressed(&[Pressable::Key(Key::Shift)], || {
    // ...
})?;

// Type a unicode string.
kbd.type_str("Hello, World!")?;
# Ok::<(), pynput::Error>(())
```

## Monitoring the keyboard

```rust
use pynput::keyboard::Listener;

let listener = Listener::builder()
    .on_press(|key, injected| {
        println!("pressed {:?} (injected={})", key, injected);
        true // return false to stop the listener
    })
    .on_release(|key, _injected| {
        println!("released {:?}", key);
        true
    })
    .start()?;

listener.join()?;
# Ok::<(), pynput::Error>(())
```

## Monitoring the mouse

```rust
use pynput::mouse::Listener;

let listener = Listener::builder()
    .on_move(|x, y, _| { println!("moved to {},{}", x, y); true })
    .on_click(|x, y, button, pressed, _| {
        println!("{:?} {} at {},{}", button, if pressed {"down"} else {"up"}, x, y);
        true
    })
    .on_scroll(|_, _, dx, dy, _| { println!("scroll {},{}", dx, dy); true })
    .start()?;

listener.join()?;
# Ok::<(), pynput::Error>(())
```

## Global hotkeys

```rust
use pynput::keyboard::GlobalHotKeys;

let listener = GlobalHotKeys::start(vec![
    ("<ctrl>+<alt>+h", Box::new(|| println!("hotkey activated!"))),
])?;
listener.join()?;
# Ok::<(), pynput::Error>(())
```

## License

LGPL-3.0-or-later, the same license as upstream *pynput*.
