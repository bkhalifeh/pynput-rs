// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! Shared helpers for the *Xorg* backend, built on top of `x11rb`.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread::JoinHandle;

use x11rb::connection::Connection;
use x11rb::protocol::record::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    ConnectionExt as _, EventMask, GrabMode, Keycode, Keysym, Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::util::{Error, ListenerCore, ListenerHandle};
use crate::Result;

// XTEST fake-event types.
pub const KEY_PRESS: u8 = 2;
pub const KEY_RELEASE: u8 = 3;
pub const BUTTON_PRESS: u8 = 4;
pub const BUTTON_RELEASE: u8 = 5;
pub const MOTION_NOTIFY: u8 = 6;

const SHIFT_MASK: u16 = 1 << 0;
const NO_SYMBOL: Keysym = 0;

fn backend<E: std::fmt::Display>(e: E) -> Error {
    Error::Backend(e.to_string())
}

/// A managed connection to the *X* display.
pub struct Display {
    pub conn: RustConnection,
    pub root: Window,
    pub min_keycode: Keycode,
    pub max_keycode: Keycode,
}

impl Display {
    /// Opens a connection to the display named by the `DISPLAY` environment
    /// variable.
    pub fn open() -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|e| {
            Error::Backend(format!(
                "failed to acquire X connection: {}. Please make sure that \
                 you have an X server running, and that the DISPLAY \
                 environment variable is set correctly",
                e
            ))
        })?;
        let setup = conn.setup();
        let root = setup.roots[screen_num].root;
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;
        Ok(Display {
            conn,
            root,
            min_keycode,
            max_keycode,
        })
    }

    pub fn sync(&self) -> Result<()> {
        self.conn.sync().map_err(backend)
    }

    /// The current pointer position as `(x, y)`.
    pub fn query_pointer(&self) -> Result<(i16, i16)> {
        let reply = self
            .conn
            .query_pointer(self.root)
            .map_err(backend)?
            .reply()
            .map_err(backend)?;
        Ok((reply.root_x, reply.root_y))
    }

    /// Moves the pointer to an absolute position.
    pub fn warp_pointer(&self, x: i16, y: i16) -> Result<()> {
        self.conn
            .warp_pointer(
                x11rb::NONE,
                self.root,
                0,
                0,
                0,
                0,
                x,
                y,
            )
            .map_err(backend)?;
        self.sync()
    }

    pub fn fake_button(&self, press: bool, button: u8) -> Result<()> {
        let type_ = if press { BUTTON_PRESS } else { BUTTON_RELEASE };
        self.conn
            .xtest_fake_input(type_, button, 0, self.root, 0, 0, 0)
            .map_err(backend)?;
        self.sync()
    }

    pub fn fake_key(&self, press: bool, keycode: Keycode) -> Result<()> {
        let type_ = if press { KEY_PRESS } else { KEY_RELEASE };
        self.conn
            .xtest_fake_input(type_, keycode, 0, self.root, 0, 0, 0)
            .map_err(backend)?;
        self.sync()
    }

    /// Returns the keycode currently mapped to a keysym, if any.
    pub fn keysym_to_keycode(&self, keysym: Keysym) -> Result<Option<Keycode>> {
        let mapping = self.raw_mapping()?;
        for (i, syms) in mapping.iter().enumerate() {
            if syms.contains(&keysym) {
                return Ok(Some(self.min_keycode + i as u8));
            }
        }
        Ok(None)
    }

    /// Returns the raw keyboard mapping as a list of keysym lists indexed by
    /// `keycode - min_keycode`.
    pub fn raw_mapping(&self) -> Result<Vec<Vec<Keysym>>> {
        let count = self.max_keycode - self.min_keycode + 1;
        let reply = self
            .conn
            .get_keyboard_mapping(self.min_keycode, count)
            .map_err(backend)?
            .reply()
            .map_err(backend)?;
        let per = reply.keysyms_per_keycode as usize;
        Ok(reply
            .keysyms
            .chunks(per)
            .map(|c| c.to_vec())
            .collect())
    }

    /// Generates a mapping from *keysyms* to *(keycode, shift_state)*.
    pub fn keyboard_mapping(&self) -> Result<HashMap<Keysym, (Keycode, u16)>> {
        let group_mask = self.alt_gr_mask()?;
        let mut mapping: HashMap<Keysym, (Keycode, u16)> = HashMap::new();

        for (index, keysyms) in self.raw_mapping()?.into_iter().enumerate() {
            let key_code = self.min_keycode + index as u8;
            let normalized = match keysym_normalize(&keysyms) {
                Some(n) => n,
                None => continue,
            };
            for (group, groups) in [false, true].into_iter().zip(normalized) {
                let members = [groups.0, groups.1];
                for (shift, &keysym) in [false, true].into_iter().zip(members.iter()) {
                    if keysym == NO_SYMBOL {
                        continue;
                    }
                    let shift_state = (if shift { SHIFT_MASK } else { 0 })
                        | (if group { group_mask } else { 0 });
                    if let Some(existing) = mapping.get(&keysym) {
                        if existing.1 < shift_state {
                            continue;
                        }
                    }
                    mapping.insert(keysym, (key_code, shift_state));
                }
            }
        }
        Ok(mapping)
    }

    fn find_mask(&self, keysym: Keysym) -> Result<u16> {
        let target = match self.keysym_to_keycode(keysym)? {
            Some(kc) => kc,
            None => return Ok(0),
        };
        let reply = self
            .conn
            .get_modifier_mapping()
            .map_err(backend)?
            .reply()
            .map_err(backend)?;
        let per = reply.keycodes_per_modifier() as usize;
        for index in 0..8 {
            for j in 0..per {
                if reply.keycodes[index * per + j] == target {
                    return Ok(1u16 << index);
                }
            }
        }
        Ok(0)
    }

    pub fn alt_gr_mask(&self) -> Result<u16> {
        self.find_mask(0xFF7E) // Mode_switch
    }

    pub fn numlock_mask(&self) -> Result<u16> {
        self.find_mask(0xFF7F) // Num_Lock
    }

    /// Sets a key state by holding the required shift modifiers and faking the
    /// keycode.
    pub fn send_mapped_key(
        &self,
        press: bool,
        keycode: Keycode,
        shift_state: u16,
    ) -> Result<()> {
        let shift_kc = self.keysym_to_keycode(0xFFE1)?; // Shift_L
        let group_kc = self.keysym_to_keycode(0xFF7E)?; // Mode_switch
        let want_shift = shift_state & SHIFT_MASK != 0;
        let want_group = shift_state & self.alt_gr_mask()? != 0;

        if press {
            if want_shift {
                if let Some(kc) = shift_kc {
                    self.fake_key(true, kc)?;
                }
            }
            if want_group {
                if let Some(kc) = group_kc {
                    self.fake_key(true, kc)?;
                }
            }
        }

        self.fake_key(press, keycode)?;

        if press {
            if want_group {
                if let Some(kc) = group_kc {
                    self.fake_key(false, kc)?;
                }
            }
            if want_shift {
                if let Some(kc) = shift_kc {
                    self.fake_key(false, kc)?;
                }
            }
        }
        Ok(())
    }
}

/// Converts a unicode character to a *keysym*.
pub fn char_to_keysym(c: char) -> Keysym {
    let ordinal = c as u32;
    if ordinal < 0x100 {
        ordinal
    } else {
        ordinal | 0x0100_0000
    }
}

fn keysym_is_latin_upper(k: Keysym) -> bool {
    (0x41..=0x5A).contains(&k)
}
fn keysym_is_latin_lower(k: Keysym) -> bool {
    (0x61..=0x7A).contains(&k)
}

fn keysym_group(ks1: Keysym, ks2: Keysym) -> (Keysym, Keysym) {
    if ks2 == NO_SYMBOL {
        if keysym_is_latin_upper(ks1) {
            (0x61 + ks1 - 0x41, ks1)
        } else if keysym_is_latin_lower(ks1) {
            (ks1, 0x41 + ks1 - 0x61)
        } else {
            (ks1, ks1)
        }
    } else {
        (ks1, ks2)
    }
}

/// Normalises a list of *keysyms* into two groups, mirroring the *X*
/// convention.
fn keysym_normalize(keysyms: &[Keysym]) -> Option<[(Keysym, Keysym); 2]> {
    // Remove trailing NoSymbol entries.
    let mut end = keysyms.len();
    while end > 0 && keysyms[end - 1] == NO_SYMBOL {
        end -= 1;
    }
    let stripped = &keysyms[..end];

    match stripped.len() {
        0 => None,
        1 => {
            let g = keysym_group(stripped[0], NO_SYMBOL);
            Some([g, g])
        }
        2 => {
            let g = keysym_group(stripped[0], stripped[1]);
            Some([g, g])
        }
        3 => Some([
            keysym_group(stripped[0], stripped[1]),
            keysym_group(stripped[2], NO_SYMBOL),
        ]),
        n if n >= 6 => Some([
            keysym_group(stripped[0], stripped[1]),
            keysym_group(stripped[4], stripped[5]),
        ]),
        _ => Some([
            keysym_group(stripped[0], stripped[1]),
            keysym_group(stripped[2], stripped[3]),
        ]),
    }
}

// ---------------------------------------------------------------------------
// Event monitoring via the RECORD extension
// ---------------------------------------------------------------------------

/// What to grab while suppressing events.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Grab {
    None,
    Keyboard,
    Pointer,
}

/// A parsed device event extracted from the RECORD data stream.
pub struct RecordedEvent {
    /// The event type (`KEY_PRESS`, `BUTTON_PRESS`, `MOTION_NOTIFY`, ...).
    pub type_: u8,
    /// The keycode or button.
    pub detail: u8,
    pub root_x: i16,
    pub root_y: i16,
    /// The modifier state.
    pub state: u16,
    /// Whether the event was injected (came from a `SendEvent`).
    pub injected: bool,
}

fn parse_events(data: &[u8], out: &mut Vec<RecordedEvent>) {
    let mut offset = 0;
    while offset + 32 <= data.len() {
        let e = &data[offset..offset + 32];
        let raw_type = e[0];
        let type_ = raw_type & 0x7F;
        // We only care about input device events.
        if (KEY_PRESS..=MOTION_NOTIFY).contains(&type_) {
            out.push(RecordedEvent {
                type_,
                detail: e[1],
                root_x: i16::from_ne_bytes([e[20], e[21]]),
                root_y: i16::from_ne_bytes([e[22], e[23]]),
                state: u16::from_ne_bytes([e[28], e[29]]),
                injected: raw_type & 0x80 != 0,
            });
        }
        offset += 32;
    }
}

/// A running RECORD-based listener.
pub struct RecordListener {
    core: Arc<ListenerCore>,
    ctrl: RustConnection,
    context: record::Context,
    root: Window,
    grab: Grab,
    thread: Option<JoinHandle<()>>,
}

impl ListenerHandle for RecordListener {
    fn core(&self) -> &Arc<ListenerCore> {
        &self.core
    }

    fn stop(&self) {
        if self.core.running() {
            self.core.set_running(false);
            match self.grab {
                Grab::Keyboard => {
                    let _ = self.ctrl.ungrab_keyboard(x11rb::CURRENT_TIME);
                }
                Grab::Pointer => {
                    let _ = self.ctrl.ungrab_pointer(x11rb::CURRENT_TIME);
                }
                Grab::None => {}
            }
            let _ = self.ctrl.record_disable_context(self.context);
            let _ = self.ctrl.flush();
        }
        let _ = self.root; // retained for completeness
    }

    fn join_boxed(mut self: Box<Self>) -> Result<()> {
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        Ok(())
    }
}

/// Starts a RECORD context capturing the given device event range and invokes a
/// handler for every captured event. The handler returns `false` to stop.
pub fn record_listen<S, H>(
    grab: Grab,
    device_events: (u8, u8),
    setup: S,
) -> Result<Box<dyn ListenerHandle>>
where
    S: FnOnce() -> Result<H> + Send + 'static,
    H: FnMut(&RecordedEvent) -> bool + Send + 'static,
{
    let core = Arc::new(ListenerCore::new());

    // The control connection is used to create and later disable the context.
    let (ctrl, screen_num) = x11rb::connect(None).map_err(backend)?;
    let root = ctrl.setup().roots[screen_num].root;
    let context = ctrl.generate_id().map_err(backend)?;

    let range = record::Range {
        core_requests: record::Range8 { first: 0, last: 0 },
        core_replies: record::Range8 { first: 0, last: 0 },
        ext_requests: record::ExtRange {
            major: record::Range8 { first: 0, last: 0 },
            minor: record::Range16 { first: 0, last: 0 },
        },
        ext_replies: record::ExtRange {
            major: record::Range8 { first: 0, last: 0 },
            minor: record::Range16 { first: 0, last: 0 },
        },
        delivered_events: record::Range8 { first: 0, last: 0 },
        device_events: record::Range8 {
            first: device_events.0,
            last: device_events.1,
        },
        errors: record::Range8 { first: 0, last: 0 },
        client_started: false,
        client_died: false,
    };

    ctrl.record_create_context(context, 0, &[record::CS::ALL_CLIENTS.into()], &[range])
        .map_err(backend)?
        .check()
        .map_err(backend)?;

    // Apply suppression grabs, if requested.
    match grab {
        Grab::Keyboard => {
            ctrl.grab_keyboard(
                false,
                root,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .map_err(backend)?;
        }
        Grab::Pointer => {
            ctrl.grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                x11rb::CURRENT_TIME,
            )
            .map_err(backend)?;
        }
        Grab::None => {}
    }
    ctrl.flush().map_err(backend)?;

    let thread_core = Arc::clone(&core);
    let thread = std::thread::Builder::new()
        .name("pynput-record".into())
        .spawn(move || {
            run_record(thread_core, context, setup);
        })
        .map_err(|e| Error::Backend(e.to_string()))?;

    Ok(Box::new(RecordListener {
        core,
        ctrl,
        context,
        root,
        grab,
        thread: Some(thread),
    }))
}

fn run_record<S, H>(core: Arc<ListenerCore>, context: record::Context, setup: S)
where
    S: FnOnce() -> Result<H>,
    H: FnMut(&RecordedEvent) -> bool,
{
    let rec = match x11rb::connect(None) {
        Ok((conn, _)) => conn,
        Err(_) => {
            core.mark_ready();
            return;
        }
    };

    let mut handler = match setup() {
        Ok(h) => h,
        Err(_) => {
            core.mark_ready();
            return;
        }
    };

    core.set_running(true);
    core.mark_ready();

    let cookie = match rec.record_enable_context(context) {
        Ok(c) => c,
        Err(_) => {
            core.set_running(false);
            return;
        }
    };

    let mut events = Vec::new();
    for reply in cookie {
        if !core.running() {
            break;
        }
        let reply = match reply {
            Ok(r) => r,
            Err(_) => break,
        };
        // Category 0 carries recorded protocol data.
        if reply.category != 0 {
            continue;
        }
        events.clear();
        parse_events(&reply.data, &mut events);
        let mut stop = false;
        for event in &events {
            if !handler(event) {
                stop = true;
                break;
            }
        }
        if stop {
            break;
        }
    }
    core.set_running(false);
}
