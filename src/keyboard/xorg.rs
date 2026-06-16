// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The keyboard implementation for *Xorg*.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use x11rb::protocol::xproto::{ConnectionExt as _, Keycode, Keysym};

use super::xorg_keysyms::{DEAD_KEYS, KEYPAD_KEYS, SYMBOLS};
use super::{Key, KeyCode, KeyInput, KeyboardCallbacks, KeyboardImpl};
use crate::util::xorg::{
    char_to_keysym, record_listen, Display, Grab, RecordedEvent, KEY_PRESS,
    KEY_RELEASE,
};
use crate::util::{Error, ListenerHandle};
use crate::Result;

/// Table of well-known keys and the *X* symbol they map to.
///
/// `(Key, symbol name, keysym, optional character)`
const KEY_SYMBOLS: &[(Key, &str, u32, Option<char>)] = &[
    (Key::Alt, "Alt_L", 0xFFE9, None),
    (Key::AltL, "Alt_L", 0xFFE9, None),
    (Key::AltR, "Alt_R", 0xFFEA, None),
    (Key::AltGr, "Mode_switch", 0xFF7E, None),
    (Key::Backspace, "BackSpace", 0xFF08, None),
    (Key::CapsLock, "Caps_Lock", 0xFFE5, None),
    (Key::Cmd, "Super_L", 0xFFEB, None),
    (Key::CmdL, "Super_L", 0xFFEB, None),
    (Key::CmdR, "Super_R", 0xFFEC, None),
    (Key::Ctrl, "Control_L", 0xFFE3, None),
    (Key::CtrlL, "Control_L", 0xFFE3, None),
    (Key::CtrlR, "Control_R", 0xFFE4, None),
    (Key::Delete, "Delete", 0xFFFF, None),
    (Key::Down, "Down", 0xFF54, None),
    (Key::End, "End", 0xFF57, None),
    (Key::Enter, "Return", 0xFF0D, None),
    (Key::Esc, "Escape", 0xFF1B, None),
    (Key::F1, "F1", 0xFFBE, None),
    (Key::F2, "F2", 0xFFBF, None),
    (Key::F3, "F3", 0xFFC0, None),
    (Key::F4, "F4", 0xFFC1, None),
    (Key::F5, "F5", 0xFFC2, None),
    (Key::F6, "F6", 0xFFC3, None),
    (Key::F7, "F7", 0xFFC4, None),
    (Key::F8, "F8", 0xFFC5, None),
    (Key::F9, "F9", 0xFFC6, None),
    (Key::F10, "F10", 0xFFC7, None),
    (Key::F11, "F11", 0xFFC8, None),
    (Key::F12, "F12", 0xFFC9, None),
    (Key::F13, "F13", 0xFFCA, None),
    (Key::F14, "F14", 0xFFCB, None),
    (Key::F15, "F15", 0xFFCC, None),
    (Key::F16, "F16", 0xFFCD, None),
    (Key::F17, "F17", 0xFFCE, None),
    (Key::F18, "F18", 0xFFCF, None),
    (Key::F19, "F19", 0xFFD0, None),
    (Key::F20, "F20", 0xFFD1, None),
    (Key::Home, "Home", 0xFF50, None),
    (Key::Left, "Left", 0xFF51, None),
    (Key::PageDown, "Page_Down", 0xFF56, None),
    (Key::PageUp, "Page_Up", 0xFF55, None),
    (Key::Right, "Right", 0xFF53, None),
    (Key::Shift, "Shift_L", 0xFFE1, None),
    (Key::ShiftL, "Shift_L", 0xFFE1, None),
    (Key::ShiftR, "Shift_R", 0xFFE2, None),
    (Key::Space, "space", 0x0020, Some(' ')),
    (Key::Tab, "Tab", 0xFF09, None),
    (Key::Up, "Up", 0xFF52, None),
    (Key::MediaPlayPause, "XF86AudioPlay", 0x1008FF14, None),
    (Key::MediaVolumeMute, "XF86AudioMute", 0x1008FF12, None),
    (Key::MediaVolumeDown, "XF86AudioLowerVolume", 0x1008FF11, None),
    (Key::MediaVolumeUp, "XF86AudioRaiseVolume", 0x1008FF13, None),
    (Key::MediaPrevious, "XF86AudioPrev", 0x1008FF16, None),
    (Key::MediaNext, "XF86AudioNext", 0x1008FF17, None),
    (Key::Insert, "Insert", 0xFF63, None),
    (Key::Menu, "Menu", 0xFF67, None),
    (Key::NumLock, "Num_Lock", 0xFF7F, None),
    (Key::Pause, "Pause", 0xFF13, None),
    (Key::PrintScreen, "Print", 0xFF61, None),
    (Key::ScrollLock, "Scroll_Lock", 0xFF14, None),
];

fn key_code_for(key: Key) -> KeyCode {
    for &(k, sym, keysym, ch) in KEY_SYMBOLS {
        if k == key {
            return KeyCode {
                vk: Some(keysym),
                char: ch,
                is_dead: false,
                combining: None,
                symbol: Some(sym),
            };
        }
    }
    // Unreachable: the table is exhaustive.
    KeyCode::from_vk(0)
}

/// Resolves a key code to a *keysym* for output.
fn key_to_keysym(key: &KeyCode) -> Option<Keysym> {
    if let Some(vk) = key.vk {
        return Some(vk);
    }
    if let Some(c) = key.char {
        // Try the symbol table first for a canonical keysym, then fall back on
        // the unicode mapping.
        for &(_name, keysym, codepoint) in SYMBOLS {
            if codepoint != 0 && char::from_u32(codepoint) == Some(c) {
                return Some(keysym);
            }
        }
        return Some(char_to_keysym(c));
    }
    None
}

pub struct Keyboard {
    display: Display,
    borrows: Mutex<HashMap<Keysym, Keycode>>,
}

impl KeyboardImpl for Keyboard {
    fn create() -> Result<Self> {
        Ok(Keyboard {
            display: Display::open()?,
            borrows: Mutex::new(HashMap::new()),
        })
    }

    fn key_value(&self, key: Key) -> KeyCode {
        key_code_for(key)
    }

    fn handle(
        &self,
        _modifiers: &HashSet<Key>,
        key: &KeyCode,
        is_press: bool,
    ) -> Result<()> {
        let keysym = key_to_keysym(key)
            .ok_or_else(|| Error::InvalidKey(format!("{}", key)))?;

        // Special keys with a virtual key code use XTEST directly.
        if key.vk.is_some() {
            let keycode = self
                .display
                .keysym_to_keycode(keysym)?
                .ok_or_else(|| Error::InvalidKey(format!("{}", key)))?;
            return self.display.fake_key(is_press, keycode);
        }

        // Character keys: look up the keysym in the keyboard mapping.
        let mapping = self.display.keyboard_mapping()?;
        if let Some(&(keycode, shift_state)) = mapping.get(&keysym) {
            return self.display.send_mapped_key(is_press, keycode, shift_state);
        }

        // The keysym is not on the current layout; borrow an unused keycode.
        self.handle_borrowed(keysym, is_press)
    }
}

impl Keyboard {
    fn handle_borrowed(&self, keysym: Keysym, is_press: bool) -> Result<()> {
        let mut borrows = self.borrows.lock().unwrap();
        let keycode = match borrows.get(&keysym) {
            Some(&kc) => kc,
            None => {
                let kc = self.allocate_keycode()?;
                self.display
                    .conn
                    .change_keyboard_mapping(1, kc, 1, &[keysym, keysym, keysym, keysym])
                    .map_err(|e| Error::Backend(e.to_string()))?;
                self.display.sync()?;
                borrows.insert(keysym, kc);
                kc
            }
        };
        self.display.fake_key(is_press, keycode)
    }

    /// Finds a keycode whose mapping is entirely empty.
    fn allocate_keycode(&self) -> Result<Keycode> {
        let mapping = self.display.raw_mapping()?;
        for (i, syms) in mapping.iter().enumerate() {
            if syms.iter().all(|&s| s == 0) {
                return Ok(self.display.min_keycode + i as u8);
            }
        }
        Err(Error::Backend("no free keycode to borrow".into()))
    }
}

/// Looks up the platform key code for a [`Key`] without a live connection.
pub(crate) fn lookup(key: Key) -> Option<KeyCode> {
    Some(key_code_for(key))
}

/// The virtual key code used when canonicalising a non-modifier special key.
pub(crate) fn canonical_vk(key: Key) -> Option<u32> {
    lookup(key).and_then(|c| c.vk)
}

// ---------------------------------------------------------------------------
// Listener event translation
// ---------------------------------------------------------------------------

fn keysym_to_special(keysym: Keysym) -> Option<Key> {
    KEY_SYMBOLS
        .iter()
        .find(|&&(_, _, ks, _)| ks == keysym)
        .map(|&(k, _, _, _)| k)
}

fn keypad_keyinput(keysym: Keysym) -> Option<KeyInput> {
    let name = KEYPAD_KEYS
        .iter()
        .find(|&&(_, ks)| ks == keysym)
        .map(|&(n, _)| n)?;
    let mapped = match name {
        "KP_0" => KeyInput::Code(KeyCode::from_char('0')),
        "KP_1" => KeyInput::Code(KeyCode::from_char('1')),
        "KP_2" => KeyInput::Code(KeyCode::from_char('2')),
        "KP_3" => KeyInput::Code(KeyCode::from_char('3')),
        "KP_4" => KeyInput::Code(KeyCode::from_char('4')),
        "KP_5" => KeyInput::Code(KeyCode::from_char('5')),
        "KP_6" => KeyInput::Code(KeyCode::from_char('6')),
        "KP_7" => KeyInput::Code(KeyCode::from_char('7')),
        "KP_8" => KeyInput::Code(KeyCode::from_char('8')),
        "KP_9" => KeyInput::Code(KeyCode::from_char('9')),
        "KP_Add" => KeyInput::Code(KeyCode::from_char('+')),
        "KP_Decimal" => KeyInput::Code(KeyCode::from_char(',')),
        "KP_Delete" => KeyInput::Key(Key::Delete),
        "KP_Divide" => KeyInput::Code(KeyCode::from_char('/')),
        "KP_Down" => KeyInput::Key(Key::Down),
        "KP_End" => KeyInput::Key(Key::End),
        "KP_Enter" => KeyInput::Key(Key::Enter),
        "KP_Equal" => KeyInput::Code(KeyCode::from_char('=')),
        "KP_F1" => KeyInput::Key(Key::F1),
        "KP_F2" => KeyInput::Key(Key::F2),
        "KP_F3" => KeyInput::Key(Key::F3),
        "KP_F4" => KeyInput::Key(Key::F4),
        "KP_Home" => KeyInput::Key(Key::Home),
        "KP_Insert" => KeyInput::Key(Key::Insert),
        "KP_Left" => KeyInput::Key(Key::Left),
        "KP_Multiply" => KeyInput::Code(KeyCode::from_char('*')),
        "KP_Page_Down" => KeyInput::Key(Key::PageDown),
        "KP_Page_Up" => KeyInput::Key(Key::PageUp),
        "KP_Right" => KeyInput::Key(Key::Right),
        "KP_Space" => KeyInput::Key(Key::Space),
        "KP_Subtract" => KeyInput::Code(KeyCode::from_char('-')),
        "KP_Tab" => KeyInput::Key(Key::Tab),
        "KP_Up" => KeyInput::Key(Key::Up),
        _ => return None,
    };
    Some(mapped)
}

/// The printable character associated with a keysym, if any.
fn char_for_keysym(keysym: Keysym) -> Option<char> {
    for &(_name, ks, codepoint) in SYMBOLS {
        if ks == keysym && codepoint != 0 {
            return char::from_u32(codepoint);
        }
    }
    None
}

/// The standalone dead-key character for a combining character, if any.
fn dead_standalone(combining: char) -> Option<char> {
    for &(comb, dead) in DEAD_KEYS {
        if char::from_u32(comb) == Some(combining) {
            return char::from_u32(dead);
        }
    }
    None
}

struct Translator {
    mapping: Vec<Vec<Keysym>>,
    min_keycode: Keycode,
    alt_gr_mask: u16,
    numlock_mask: u16,
}

impl Translator {
    fn keycode_to_keysym(&self, keycode: Keycode, index: u8) -> Keysym {
        if keycode < self.min_keycode {
            return 0;
        }
        let i = (keycode - self.min_keycode) as usize;
        let syms = match self.mapping.get(i) {
            Some(s) => s,
            None => return 0,
        };
        let keysym = syms.get(index as usize).copied().unwrap_or(0);
        if keysym != 0 {
            keysym
        } else if index & 0x2 != 0 {
            self.keycode_to_keysym(keycode, index & !0x2)
        } else if index & 0x1 != 0 {
            self.keycode_to_keysym(keycode, index & !0x1)
        } else {
            0
        }
    }

    /// Converts a keycode and modifier state into a [`KeyInput`].
    fn event_to_key(&self, keycode: Keycode, state: u16) -> Option<KeyInput> {
        // Out-of-range keycodes have no mapping row.
        if keycode < self.min_keycode
            || (keycode - self.min_keycode) as usize >= self.mapping.len()
        {
            return None;
        }

        let index = (if state & 1 != 0 { 1 } else { 0 })
            + (if state & self.alt_gr_mask != 0 { 2 } else { 0 });

        let keysym = self.keycode_to_keysym(keycode, index);

        // First try special keys...
        if let Some(key) = keysym_to_special(keysym) {
            return Some(KeyInput::Key(key));
        }
        // ...then keypad keys, recalculating with the numlock state...
        if keypad_keyinput(keysym).is_some() {
            let numlock_index = if state & self.numlock_mask != 0 { 1 } else { 0 };
            let recalculated = self.keycode_to_keysym(keycode, numlock_index);
            if let Some(k) = keypad_keyinput(recalculated) {
                return Some(k);
            }
        }

        // ...then characters...
        if let Some(mut ch) = char_for_keysym(keysym) {
            if index & 1 != 0 {
                ch = ch.to_uppercase().next().unwrap_or(ch);
            }
            if let Some(dead) = dead_standalone(ch) {
                return KeyCode::from_vk_dead(dead, keysym)
                    .ok()
                    .map(KeyInput::Code);
            }
            return Some(KeyInput::Code(KeyCode::from_char_vk(ch, keysym)));
        }

        // ...and fall back on a virtual key code.
        Some(KeyInput::Code(KeyCode::from_vk(keysym)))
    }
}

pub(crate) fn spawn_listener(
    callbacks: KeyboardCallbacks,
) -> Result<Box<dyn ListenerHandle>> {
    let grab = if callbacks.suppress {
        Grab::Keyboard
    } else {
        Grab::None
    };
    let KeyboardCallbacks {
        mut on_press,
        mut on_release,
        ..
    } = callbacks;

    record_listen(grab, (KEY_PRESS, KEY_RELEASE), move || {
        let display = Display::open()?;
        let translator = Translator {
            mapping: display.raw_mapping()?,
            min_keycode: display.min_keycode,
            alt_gr_mask: display.alt_gr_mask()?,
            numlock_mask: display.numlock_mask()?,
        };
        Ok(move |event: &RecordedEvent| -> bool {
            let key = translator.event_to_key(event.detail, event.state);
            match event.type_ {
                KEY_PRESS => on_press(key, event.injected),
                KEY_RELEASE => on_release(key, event.injected),
                _ => true,
            }
        })
    })
}
