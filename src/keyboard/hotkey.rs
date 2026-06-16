// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! Hotkey support, mirroring `pynput.keyboard.HotKey` and `GlobalHotKeys`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::{canonical, imp, Key, KeyCode, KeyInput, Listener};
use crate::util::Error;
use crate::Result;

/// A combination of keys acting as a hotkey.
///
/// This is a container of hotkey state for a keyboard listener.
pub struct HotKey {
    state: HashSet<KeyInput>,
    keys: HashSet<KeyInput>,
    on_activate: Box<dyn FnMut() + Send>,
}

impl HotKey {
    /// Creates a hotkey from a collection of keys and an activation callback.
    pub fn new<F>(keys: Vec<KeyInput>, on_activate: F) -> Self
    where
        F: FnMut() + Send + 'static,
    {
        HotKey {
            state: HashSet::new(),
            keys: keys.into_iter().collect(),
            on_activate: Box::new(on_activate),
        }
    }

    /// Parses a key combination string.
    ///
    /// Key combination strings are sequences of key identifiers separated by
    /// `'+'`. Key identifiers are either single characters representing a
    /// keyboard key, such as `'a'`, or special key names enclosed in brackets,
    /// such as `"<ctrl>"`.
    pub fn parse(keys: &str) -> Result<Vec<KeyInput>> {
        let raw_parts = split_parts(keys)?;
        let mut parsed = Vec::with_capacity(raw_parts.len());
        for part in &raw_parts {
            parsed.push(parse_part(part)?);
        }

        // Ensure no duplicate parts.
        let unique: HashSet<&KeyInput> = parsed.iter().collect();
        if unique.len() != parsed.len() {
            return Err(Error::InvalidString(keys.to_string()));
        }
        Ok(parsed)
    }

    /// Updates the hotkey state for a pressed key, invoking the activation
    /// callback if the full combination becomes active. The callback fires only
    /// once until at least one key is released.
    pub fn press(&mut self, key: KeyInput) {
        if self.keys.contains(&key) && !self.state.contains(&key) {
            self.state.insert(key);
            if self.state == self.keys {
                (self.on_activate)();
            }
        }
    }

    /// Updates the hotkey state for a released key.
    pub fn release(&mut self, key: KeyInput) {
        self.state.remove(&key);
    }
}

fn split_parts(keys: &str) -> Result<Vec<String>> {
    let chars: Vec<char> = keys.chars().collect();
    let mut parts = Vec::new();
    let mut start = 0usize;
    for (i, &c) in chars.iter().enumerate() {
        if c == '+' && i != start {
            parts.push(chars[start..i].iter().collect());
            start = i + 1;
        }
    }
    if start == chars.len() {
        return Err(Error::InvalidString(keys.to_string()));
    }
    parts.push(chars[start..].iter().collect());
    Ok(parts)
}

fn parse_part(s: &str) -> Result<KeyInput> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() == 1 {
        let lower = chars[0].to_lowercase().next().unwrap_or(chars[0]);
        return Ok(KeyInput::Code(KeyCode::from_char(lower)));
    }
    if chars.len() > 2 && chars[0] == '<' && chars[chars.len() - 1] == '>' {
        let inner: String = chars[1..chars.len() - 1].iter().collect();
        let lower = inner.to_lowercase();
        if let Some(key) = Key::from_name(&lower) {
            // Represent base modifiers as `Key`, everything else as a key code.
            if key.normal_modifier() == Some(key) {
                return Ok(KeyInput::Key(key));
            }
            let vk = imp::canonical_vk(key).unwrap_or(0);
            return Ok(KeyInput::Code(KeyCode::from_vk(vk)));
        }
        if let Ok(vk) = inner.parse::<u32>() {
            return Ok(KeyInput::Code(KeyCode::from_vk(vk)));
        }
        return Err(Error::InvalidString(s.to_string()));
    }
    Err(Error::InvalidString(s.to_string()))
}

/// A boxed hotkey activation callback.
pub type HotKeyAction = Box<dyn FnMut() + Send>;

/// A keyboard listener supporting a number of global hotkeys.
///
/// This is a convenience wrapper that registers a number of global hotkeys.
pub struct GlobalHotKeys;

impl GlobalHotKeys {
    /// Starts a listener for the given hotkeys.
    ///
    /// Each entry maps a hotkey description (passed to [`HotKey::parse`]) to an
    /// action callback.
    pub fn start(hotkeys: Vec<(&str, HotKeyAction)>) -> Result<Listener> {
        let mut parsed = Vec::with_capacity(hotkeys.len());
        for (desc, action) in hotkeys {
            parsed.push(HotKey::new(HotKey::parse(desc)?, action));
        }
        let hotkeys = Arc::new(Mutex::new(parsed));

        let press = Arc::clone(&hotkeys);
        let release = Arc::clone(&hotkeys);
        Listener::builder()
            .on_press(move |key, injected| {
                if !injected {
                    if let Some(key) = key {
                        let canonical = canonical(&key);
                        for hk in press.lock().unwrap().iter_mut() {
                            hk.press(canonical.clone());
                        }
                    }
                }
                true
            })
            .on_release(move |key, injected| {
                if !injected {
                    if let Some(key) = key {
                        let canonical = canonical(&key);
                        for hk in release.lock().unwrap().iter_mut() {
                            hk.release(canonical.clone());
                        }
                    }
                }
                true
            })
            .start()
    }
}
