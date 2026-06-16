# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Rust port of the [pynput](https://github.com/moses-palmer/pynput) Python
library: control and monitor mouse + keyboard input. The public API
deliberately mirrors pynput's names and structure (`Controller`, `Listener`,
`Key`, `KeyCode`, `Button`, `HotKey`, `GlobalHotKeys`, `canonical`). When in
doubt about intended behaviour, the upstream Python semantics are the spec.

## Commands

```sh
cargo build                       # default Linux build (X11 + evdev compiled in)
cargo clippy                      # lint; keep this clean (CI-grade)
cargo test                        # integration tests (tests/logic.rs) + doctests
cargo test --test logic           # only the logic suite
cargo test --test logic dead_key_join_compose   # a single test by name
cargo test --doc                  # only doctests (lib.rs / README examples)
cargo run --example monitor       # live keyboard/mouse monitor
cargo run --example control       # move pointer + type a string
```

Runtime backend override (Linux only) — see Architecture:

```sh
PYNPUT_BACKEND=xorg   cargo run --example control   # force X11
PYNPUT_BACKEND=uinput cargo run --example monitor    # force evdev
```

## Architecture

### Backend abstraction (the core idea)

Each device has a public, platform-independent front end and a private,
per-platform backend selected at compile time via a `mod imp` alias.

- `keyboard::Controller<I: KeyboardImpl = imp::Keyboard>` and
  `mouse::Controller<I: MouseImpl = imp::Mouse>` hold all the
  platform-independent logic. `new()` is a concrete impl on the default backend;
  `with_backend()` is generic (used by tests). The default type parameter is
  what makes `Controller::new()` resolve without turbofish.
- `KeyboardImpl` / `MouseImpl` (in `keyboard/mod.rs`, `mouse/mod.rs`) are the
  backend traits: `create()`, `key_value()`, `handle()` / `position()`,
  `press()`, etc. They are `pub` only so `Controller` can name its type
  parameter — not meant to be implemented downstream.
- The keyboard `Controller` owns the stateful pynput logic that must NOT live in
  backends: modifier tracking, caps-lock toggle, dead-key composition, shift
  resolution, `type_str`. Backends only emit a single resolved key event in
  `handle()`.

### Per-OS module selection

`keyboard/mod.rs` and `mouse/mod.rs` pick `imp` by `#[cfg(target_os = ...)]`:
`linux.rs` (Linux), `win32.rs` (Windows), `darwin.rs` (macOS), `dummy.rs`
(everything else, returns `Error::Unsupported`).

### Linux is special: runtime dispatch

On Linux **both** `xorg.rs` (x11rb) and `uinput.rs` (evdev) are compiled, and
`linux.rs` is the `imp`. `linux.rs` is an enum (`Keyboard::{Xorg,Uinput}`)
whose `create()` and free functions dispatch at runtime via
`util::use_uinput(kind)`:

1. `PYNPUT_BACKEND_KEYBOARD` / `PYNPUT_BACKEND_MOUSE` (`xorg`|`uinput`),
2. else `PYNPUT_BACKEND`,
3. else evdev on a Wayland session (`WAYLAND_DISPLAY` / `XDG_SESSION_TYPE`), X11
   otherwise.

`canonical_vk()` must use the **same** selection logic as the listener, because
the X11 (keysym) and evdev (input-event code) virtual-key codes differ — a
hotkey parsed under one backend will not match events from the other.

### Shared X11 layer

`util/xorg.rs` is the only place that talks to x11rb. `Display` wraps the
connection and exposes XTEST fake input, `WarpPointer`, keyboard-mapping
normalisation (a faithful port of pynput's `keysym_normalize` / `keysym_group`),
and modifier masks. `record_listen()` is the generic global event monitor: it
creates a RECORD context on a control connection, runs the blocking
`record_enable_context` iterator on a worker thread, parses raw 32-byte X events
out of `EnableContextReply.data`, and is stopped by calling
`record_disable_context` from the control connection. Both keyboard and mouse
listeners are thin translation closures over `record_listen`.

### Listener lifecycle

`util::ListenerCore` (running flag + ready barrier) and the `ListenerHandle`
trait (`core()`, `stop()`, `join_boxed()`) are the cross-backend contract.
Public `Listener` types are builders that box a backend `ListenerHandle`.
Callbacks return `bool`; `false` stops the listener (the pynput `StopException`
/ `return False` convention).

### Generated keysym data

`keyboard/xorg_keysyms.rs` is **generated** from the upstream Python
`xorg_keysyms.py` (it no longer exists in the repo) and holds `SYMBOLS`,
`DEAD_KEYS`, `KEYPAD_KEYS`, plus a precomputed `COMPOSE` NFC table
(`compose_nfc`) used because std has no Unicode normaliser. Treat it as
generated — do not hand-edit; regenerate from a pynput checkout if it must
change.

### KeyCode equality

`KeyCode`'s `PartialEq`/`Hash` follow pynput's (slightly inconsistent) rules:
compare by `char` when both have one, else by `vk`. Hash mirrors this. The key
sets used internally are homogeneous (all-char or all-vk), so this is sound in
practice; preserve the invariant if adding new uses.

## Verification status (important when editing backends)

- **X11 and evdev** compile and are exercised on Linux. The X11 *listener* is
  verified live (RECORD); the X11 *controllers* use XTEST, which is **disabled
  on the current dev host's X server** (`WarpPointer` works, XTEST silently
  no-ops), so injection cannot be confirmed here — validate controller logic via
  unit tests, not by watching the screen.
- **win32.rs and darwin.rs are compile-gated to their OS and have never been
  built here.** They are written against the documented `windows` /
  `core-graphics` APIs and need a real build pass on those platforms.
