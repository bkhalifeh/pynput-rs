// pynput
// Copyright (C) 2015-2024 Moses Palmér
//
// Licensed under the GNU Lesser General Public License v3.0 or later.

//! The module containing keyboard classes.
//!
//! The platform dependent implementation is selected at compile time, mirroring
//! the backend selection performed by the original *pynput* library at import
//! time.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::util::{Error, ListenerHandle};
use crate::Result;

mod hotkey;
pub use hotkey::{GlobalHotKeys, HotKey};

// Generated keysym tables, shared by the X11 backend.
#[cfg(target_os = "linux")]
pub(crate) mod xorg_keysyms;

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
// KeyCode
// ---------------------------------------------------------------------------

/// A `KeyCode` represents the description of a key code used by the operating
/// system.
#[derive(Clone, Debug)]
pub struct KeyCode {
    /// The virtual key code, if any.
    pub vk: Option<u32>,

    /// The character represented by this key, if any.
    pub char: Option<char>,

    /// Whether this is a dead key.
    pub is_dead: bool,

    /// The combining character associated with a dead key.
    pub combining: Option<char>,

    /// The *X* symbol name (platform extension, only set by the X11 backend).
    pub(crate) symbol: Option<&'static str>,
}

impl KeyCode {
    /// Creates a key from a virtual key code.
    pub fn from_vk(vk: u32) -> Self {
        KeyCode {
            vk: Some(vk),
            char: None,
            is_dead: false,
            combining: None,
            symbol: None,
        }
    }

    /// Creates a key from a character.
    pub fn from_char(c: char) -> Self {
        KeyCode {
            vk: None,
            char: Some(c),
            is_dead: false,
            combining: None,
            symbol: None,
        }
    }

    /// Creates a dead key.
    ///
    /// `c` should be the standalone character representing the dead key, such as
    /// `'~'` for *COMBINING TILDE*.
    pub fn from_dead(c: char) -> Result<Self> {
        match combining_for(c) {
            Some(combining) => Ok(KeyCode {
                vk: None,
                char: Some(c),
                is_dead: true,
                combining: Some(combining),
                symbol: None,
            }),
            None => Err(Error::InvalidKey(format!("{:?}", c))),
        }
    }

    #[allow(dead_code)] // used by the X11 listener only
    pub(crate) fn from_vk_dead(c: char, vk: u32) -> Result<Self> {
        let mut k = Self::from_dead(c)?;
        k.vk = Some(vk);
        Ok(k)
    }

    #[allow(dead_code)] // used by the X11 listener only
    pub(crate) fn from_char_vk(c: char, vk: u32) -> Self {
        let mut k = Self::from_char(c);
        k.vk = Some(vk);
        k
    }

    /// Applies this dead key to another key and returns the result.
    ///
    /// Joining a dead key with space (`' '`) or itself yields the non-dead
    /// version of this key, if one exists.
    pub fn join(&self, key: &KeyCode) -> Result<KeyCode> {
        // A non-dead key cannot be joined.
        if !self.is_dead {
            return Err(Error::InvalidKey(format!("{:?}", self)));
        }

        // Joining two of the same key codes, or joining with space, yields the
        // non-dead version of the key.
        if key.char == Some(' ') || self == key {
            return Ok(KeyCode::from_char(self.char.unwrap()));
        }

        // Otherwise we combine the characters.
        if let (Some(kc), Some(comb)) = (key.char, self.combining) {
            if let Some(combined) = compose(kc, comb) {
                return Ok(KeyCode::from_char(combined));
            }
        }

        Err(Error::InvalidKey(format!("{:?}", key)))
    }

    fn repr(&self) -> String {
        if self.is_dead {
            format!("[{:?}]", self.char.unwrap())
        } else if let Some(c) = self.char {
            format!("{:?}", c)
        } else {
            format!("<{}>", self.vk.unwrap_or(0))
        }
    }
}

impl PartialEq for KeyCode {
    fn eq(&self, other: &Self) -> bool {
        match (self.char, other.char) {
            (Some(a), Some(b)) => a == b && self.is_dead == other.is_dead,
            _ => self.vk == other.vk && self.symbol == other.symbol,
        }
    }
}

impl Eq for KeyCode {}

impl Hash for KeyCode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash by character if present, otherwise by virtual key code. This
        // keeps the homogeneous key sets used internally (all-character or
        // all-vk) consistent with `PartialEq`.
        match self.char {
            Some(c) => {
                0u8.hash(state);
                c.hash(state);
                self.is_dead.hash(state);
            }
            None => {
                1u8.hash(state);
                self.vk.hash(state);
            }
        }
    }
}

impl std::fmt::Display for KeyCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.repr())
    }
}

/// Returns the combining character for a standalone dead-key character.
fn combining_for(dead: char) -> Option<char> {
    #[cfg(target_os = "linux")]
    {
        for &(comb, d) in xorg_keysyms::DEAD_KEYS {
            if char::from_u32(d) == Some(dead) {
                return char::from_u32(comb);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dead;
        None
    }
}

/// Composes a base character with a combining character, returning the NFC
/// form when it is a single scalar value.
fn compose(base: char, combining: char) -> Option<char> {
    let mut s = String::with_capacity(8);
    s.push(base);
    s.push(combining);
    // We do not depend on a full Unicode normaliser; instead we rely on the
    // precomposed forms being a single scalar value when they exist. Since the
    // standard library does not expose NFC, we look the pair up in the symbol
    // table by composing manually for the common Latin range.
    crate::keyboard::nfc_compose(base, combining)
}

#[cfg(target_os = "linux")]
fn nfc_compose(base: char, combining: char) -> Option<char> {
    xorg_keysyms::compose_nfc(base, combining)
}

#[cfg(not(target_os = "linux"))]
fn nfc_compose(_base: char, _combining: char) -> Option<char> {
    None
}

// ---------------------------------------------------------------------------
// Key
// ---------------------------------------------------------------------------

/// A key that may not correspond to a printable character, such as a modifier
/// or function key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Key {
    Alt,
    AltL,
    AltR,
    AltGr,
    Backspace,
    CapsLock,
    Cmd,
    CmdL,
    CmdR,
    Ctrl,
    CtrlL,
    CtrlR,
    Delete,
    Down,
    End,
    Enter,
    Esc,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    Home,
    Left,
    PageDown,
    PageUp,
    Right,
    Shift,
    ShiftL,
    ShiftR,
    Space,
    Tab,
    Up,
    MediaPlayPause,
    MediaVolumeMute,
    MediaVolumeDown,
    MediaVolumeUp,
    MediaPrevious,
    MediaNext,
    Insert,
    Menu,
    NumLock,
    Pause,
    PrintScreen,
    ScrollLock,
}

impl Key {
    /// All key variants, in declaration order.
    pub const ALL: [Key; 60] = [
        Key::Alt,
        Key::AltL,
        Key::AltR,
        Key::AltGr,
        Key::Backspace,
        Key::CapsLock,
        Key::Cmd,
        Key::CmdL,
        Key::CmdR,
        Key::Ctrl,
        Key::CtrlL,
        Key::CtrlR,
        Key::Delete,
        Key::Down,
        Key::End,
        Key::Enter,
        Key::Esc,
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
        Key::F13,
        Key::F14,
        Key::F15,
        Key::F16,
        Key::F17,
        Key::F18,
        Key::F19,
        Key::F20,
        Key::Home,
        Key::Left,
        Key::PageDown,
        Key::PageUp,
        Key::Right,
        Key::Shift,
        Key::ShiftL,
        Key::ShiftR,
        Key::Space,
        Key::Tab,
        Key::Up,
        Key::MediaPlayPause,
        Key::MediaVolumeMute,
        Key::MediaVolumeDown,
        Key::MediaVolumeUp,
        Key::MediaPrevious,
        Key::MediaNext,
        Key::Insert,
        Key::Menu,
        Key::NumLock,
        Key::Pause,
        Key::PrintScreen,
        Key::ScrollLock,
    ];

    /// The snake-case name of this key, as used by [`HotKey::parse`].
    pub fn name(self) -> &'static str {
        match self {
            Key::Alt => "alt",
            Key::AltL => "alt_l",
            Key::AltR => "alt_r",
            Key::AltGr => "alt_gr",
            Key::Backspace => "backspace",
            Key::CapsLock => "caps_lock",
            Key::Cmd => "cmd",
            Key::CmdL => "cmd_l",
            Key::CmdR => "cmd_r",
            Key::Ctrl => "ctrl",
            Key::CtrlL => "ctrl_l",
            Key::CtrlR => "ctrl_r",
            Key::Delete => "delete",
            Key::Down => "down",
            Key::End => "end",
            Key::Enter => "enter",
            Key::Esc => "esc",
            Key::F1 => "f1",
            Key::F2 => "f2",
            Key::F3 => "f3",
            Key::F4 => "f4",
            Key::F5 => "f5",
            Key::F6 => "f6",
            Key::F7 => "f7",
            Key::F8 => "f8",
            Key::F9 => "f9",
            Key::F10 => "f10",
            Key::F11 => "f11",
            Key::F12 => "f12",
            Key::F13 => "f13",
            Key::F14 => "f14",
            Key::F15 => "f15",
            Key::F16 => "f16",
            Key::F17 => "f17",
            Key::F18 => "f18",
            Key::F19 => "f19",
            Key::F20 => "f20",
            Key::Home => "home",
            Key::Left => "left",
            Key::PageDown => "page_down",
            Key::PageUp => "page_up",
            Key::Right => "right",
            Key::Shift => "shift",
            Key::ShiftL => "shift_l",
            Key::ShiftR => "shift_r",
            Key::Space => "space",
            Key::Tab => "tab",
            Key::Up => "up",
            Key::MediaPlayPause => "media_play_pause",
            Key::MediaVolumeMute => "media_volume_mute",
            Key::MediaVolumeDown => "media_volume_down",
            Key::MediaVolumeUp => "media_volume_up",
            Key::MediaPrevious => "media_previous",
            Key::MediaNext => "media_next",
            Key::Insert => "insert",
            Key::Menu => "menu",
            Key::NumLock => "num_lock",
            Key::Pause => "pause",
            Key::PrintScreen => "print_screen",
            Key::ScrollLock => "scroll_lock",
        }
    }

    /// Looks up a key by its snake-case name.
    pub fn from_name(name: &str) -> Option<Key> {
        Key::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Returns the base modifier this key normalises to, if it is a modifier.
    ///
    /// This is the Rust equivalent of `_NORMAL_MODIFIERS`: the relationship
    /// between a specific modifier and its generic form is platform
    /// independent.
    pub fn normal_modifier(self) -> Option<Key> {
        match self {
            Key::Alt | Key::AltL | Key::AltR => Some(Key::Alt),
            Key::AltGr => Some(Key::AltGr),
            Key::Cmd | Key::CmdL | Key::CmdR => Some(Key::Cmd),
            Key::Ctrl | Key::CtrlL | Key::CtrlR => Some(Key::Ctrl),
            Key::Shift | Key::ShiftL | Key::ShiftR => Some(Key::Shift),
            _ => None,
        }
    }
}

/// Groups of modifier keys; the first element of each group is the base
/// modifier, mirroring `_MODIFIER_KEYS`.
const MODIFIER_GROUPS: &[(Key, &[Key])] = &[
    (Key::AltGr, &[Key::AltGr]),
    (Key::Alt, &[Key::Alt, Key::AltL, Key::AltR]),
    (Key::Cmd, &[Key::Cmd, Key::CmdL, Key::CmdR]),
    (Key::Ctrl, &[Key::Ctrl, Key::CtrlL, Key::CtrlR]),
    (Key::Shift, &[Key::Shift, Key::ShiftL, Key::ShiftR]),
];

// ---------------------------------------------------------------------------
// Key input (the value handed to listener callbacks and hotkeys)
// ---------------------------------------------------------------------------

/// A resolved key event value: either a well-known [`Key`] or a [`KeyCode`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyInput {
    /// A well-known key.
    Key(Key),
    /// A key code.
    Code(KeyCode),
}

impl From<Key> for KeyInput {
    fn from(k: Key) -> Self {
        KeyInput::Key(k)
    }
}

impl From<KeyCode> for KeyInput {
    fn from(c: KeyCode) -> Self {
        KeyInput::Code(c)
    }
}

/// Something that can be pressed or released: a [`Key`], a [`KeyCode`] or a
/// single character.
#[derive(Clone, Debug)]
pub enum Pressable {
    /// A well-known key.
    Key(Key),
    /// A key code.
    Code(KeyCode),
    /// A single character.
    Char(char),
}

impl From<Key> for Pressable {
    fn from(k: Key) -> Self {
        Pressable::Key(k)
    }
}
impl From<KeyCode> for Pressable {
    fn from(c: KeyCode) -> Self {
        Pressable::Code(c)
    }
}
impl From<char> for Pressable {
    fn from(c: char) -> Self {
        Pressable::Char(c)
    }
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

/// The platform dependent keyboard implementation.
///
/// This trait is public only so that [`Controller`] can name its backend type
/// parameter; it is not intended to be implemented outside the crate.
pub trait KeyboardImpl: Send + Sync + 'static {
    /// Creates the backend, acquiring any platform resources.
    fn create() -> Result<Self>
    where
        Self: Sized;

    /// The platform [`KeyCode`] backing a well-known [`Key`].
    fn key_value(&self, key: Key) -> KeyCode;

    /// Emits a single key event.
    ///
    /// `modifiers` is the set of currently pressed base modifiers, as tracked
    /// by the controller.
    fn handle(
        &self,
        modifiers: &HashSet<Key>,
        key: &KeyCode,
        is_press: bool,
    ) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Controller
// ---------------------------------------------------------------------------

/// A controller for sending virtual keyboard events to the system.
pub struct Controller<I: KeyboardImpl = imp::Keyboard> {
    imp: I,
    modifiers: Mutex<HashSet<KeyCode>>,
    caps_lock: AtomicBool,
    dead_key: Mutex<Option<KeyCode>>,
    normal_modifiers: HashMap<KeyCode, Key>,
    caps_lock_value: KeyCode,
}

impl Controller<imp::Keyboard> {
    /// Creates a new controller, acquiring the platform backend.
    pub fn new() -> Result<Self> {
        Ok(Self::with_backend(imp::Keyboard::create()?))
    }
}

impl<I: KeyboardImpl> Controller<I> {
    /// Creates a controller from an explicit backend (useful for testing).
    pub fn with_backend(imp: I) -> Self {
        // Build the value -> base modifier mapping from the backend.
        let mut normal_modifiers = HashMap::new();
        for (base, members) in MODIFIER_GROUPS {
            for member in *members {
                normal_modifiers.insert(imp.key_value(*member), *base);
            }
        }
        let caps_lock_value = imp.key_value(Key::CapsLock);

        Controller {
            imp,
            modifiers: Mutex::new(HashSet::new()),
            caps_lock: AtomicBool::new(false),
            dead_key: Mutex::new(None),
            normal_modifiers,
            caps_lock_value,
        }
    }

    /// Presses a key.
    pub fn press<P: Into<Pressable>>(&self, key: P) -> Result<()> {
        let resolved = self.resolve(key.into())?;
        self.update_modifiers(&resolved, true);

        // Update caps lock state.
        if resolved == self.caps_lock_value {
            let prev = self.caps_lock.load(Ordering::SeqCst);
            self.caps_lock.store(!prev, Ordering::SeqCst);
        }

        let original = resolved.clone();
        let mut resolved = resolved;

        // If we currently have a dead key pressed, join it with this key.
        let dead = self.dead_key.lock().unwrap().clone();
        if let Some(dk) = dead {
            match dk.join(&resolved) {
                Ok(joined) => resolved = joined,
                Err(_) => {
                    self.handle(&dk, true)?;
                    self.handle(&dk, false)?;
                }
            }
        }

        // If the key is a dead key, keep it for later.
        if resolved.is_dead {
            *self.dead_key.lock().unwrap() = Some(resolved);
            return Ok(());
        }

        match self.handle(&resolved, true) {
            Ok(()) => {}
            Err(Error::InvalidKey(_)) if resolved != original => {
                if let Some(dk) = self.dead_key.lock().unwrap().clone() {
                    self.handle(&dk, true)?;
                    self.handle(&dk, false)?;
                }
                self.handle(&original, true)?;
            }
            Err(e) => return Err(e),
        }

        *self.dead_key.lock().unwrap() = None;
        Ok(())
    }

    /// Releases a key.
    pub fn release<P: Into<Pressable>>(&self, key: P) -> Result<()> {
        let resolved = self.resolve(key.into())?;
        self.update_modifiers(&resolved, false);

        // Ignore released dead keys.
        if resolved.is_dead {
            return Ok(());
        }

        self.handle(&resolved, false)
    }

    /// Presses and releases a key.
    pub fn tap<P: Into<Pressable> + Clone>(&self, key: P) -> Result<()> {
        self.press(key.clone())?;
        self.release(key)
    }

    /// Calls [`press`](Self::press) or [`release`](Self::release) depending on
    /// `is_press`.
    pub fn touch<P: Into<Pressable>>(&self, key: P, is_press: bool) -> Result<()> {
        if is_press {
            self.press(key)
        } else {
            self.release(key)
        }
    }

    /// Executes a closure with some keys held down, releasing them afterwards.
    pub fn pressed<F, R>(&self, keys: &[Pressable], f: F) -> Result<R>
    where
        F: FnOnce() -> R,
    {
        for key in keys {
            self.press(key.clone())?;
        }
        let result = f();
        for key in keys.iter().rev() {
            self.release(key.clone())?;
        }
        Ok(result)
    }

    /// Types a string, sending all key presses and releases necessary.
    pub fn type_str(&self, string: &str) -> Result<()> {
        for (i, character) in string.chars().enumerate() {
            let pressable = match character {
                '\n' | '\r' => Pressable::Key(Key::Enter),
                '\t' => Pressable::Key(Key::Tab),
                c => Pressable::Char(c),
            };
            let attempt = (|| {
                self.press(pressable.clone())?;
                self.release(pressable.clone())
            })();
            if attempt.is_err() {
                return Err(Error::InvalidCharacter(i, character));
            }
        }
        Ok(())
    }

    /// The set of currently pressed base modifier keys.
    pub fn modifiers(&self) -> HashSet<Key> {
        let modifiers = self.modifiers.lock().unwrap();
        modifiers
            .iter()
            .filter_map(|m| self.as_modifier(m))
            .collect()
    }

    /// Whether any *alt* key is pressed.
    pub fn alt_pressed(&self) -> bool {
        self.modifiers().contains(&Key::Alt)
    }

    /// Whether *altgr* is pressed.
    pub fn alt_gr_pressed(&self) -> bool {
        self.modifiers().contains(&Key::AltGr)
    }

    /// Whether any *ctrl* key is pressed.
    pub fn ctrl_pressed(&self) -> bool {
        self.modifiers().contains(&Key::Ctrl)
    }

    /// Whether any *shift* key is pressed, or *caps lock* is toggled.
    pub fn shift_pressed(&self) -> bool {
        self.caps_lock.load(Ordering::SeqCst)
            || self.modifiers().contains(&Key::Shift)
    }

    fn handle(&self, key: &KeyCode, is_press: bool) -> Result<()> {
        let modifiers = self.modifiers();
        self.imp.handle(&modifiers, key, is_press)
    }

    fn resolve(&self, key: Pressable) -> Result<KeyCode> {
        match key {
            Pressable::Key(k) => Ok(self.imp.key_value(k)),
            Pressable::Char(c) => Ok(KeyCode::from_char(c)),
            Pressable::Code(code) => match code.char {
                Some(ch) if self.shift_pressed() => {
                    let upper = ch.to_uppercase().next().unwrap_or(ch);
                    Ok(KeyCode {
                        vk: code.vk,
                        char: Some(upper),
                        is_dead: code.is_dead,
                        combining: code.combining,
                        symbol: code.symbol,
                    })
                }
                _ => Ok(code),
            },
        }
    }

    fn update_modifiers(&self, key: &KeyCode, is_press: bool) {
        if self.as_modifier(key).is_some() {
            let mut modifiers = self.modifiers.lock().unwrap();
            if is_press {
                modifiers.insert(key.clone());
            } else {
                modifiers.remove(key);
            }
        }
    }

    fn as_modifier(&self, key: &KeyCode) -> Option<Key> {
        self.normal_modifiers.get(key).copied()
    }
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// A keyboard callback: receives the key (`None` if unknown) and whether the
/// event was injected. Returning `false` stops the listener.
pub type KeyCallback = Box<dyn FnMut(Option<KeyInput>, bool) -> bool + Send>;

#[allow(dead_code)] // `suppress` is unused by some backends
pub(crate) struct KeyboardCallbacks {
    pub on_press: KeyCallback,
    pub on_release: KeyCallback,
    pub suppress: bool,
}

/// A builder for a keyboard [`Listener`].
#[derive(Default)]
pub struct ListenerBuilder {
    on_press: Option<KeyCallback>,
    on_release: Option<KeyCallback>,
    suppress: bool,
}

impl ListenerBuilder {
    /// Sets the callback invoked when a key is pressed.
    pub fn on_press<F>(mut self, f: F) -> Self
    where
        F: FnMut(Option<KeyInput>, bool) -> bool + Send + 'static,
    {
        self.on_press = Some(Box::new(f));
        self
    }

    /// Sets the callback invoked when a key is released.
    pub fn on_release<F>(mut self, f: F) -> Self
    where
        F: FnMut(Option<KeyInput>, bool) -> bool + Send + 'static,
    {
        self.on_release = Some(Box::new(f));
        self
    }

    /// Whether to suppress events system wide.
    pub fn suppress(mut self, suppress: bool) -> Self {
        self.suppress = suppress;
        self
    }

    /// Starts the listener.
    pub fn start(self) -> Result<Listener> {
        let callbacks = KeyboardCallbacks {
            on_press: self.on_press.unwrap_or_else(|| Box::new(|_, _| true)),
            on_release: self.on_release.unwrap_or_else(|| Box::new(|_, _| true)),
            suppress: self.suppress,
        };
        Ok(Listener {
            handle: imp::spawn_listener(callbacks)?,
        })
    }
}

/// A listener for keyboard events.
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

/// Performs normalisation of a key, so events compare equal regardless of
/// modifier state.
pub fn canonical(key: &KeyInput) -> KeyInput {
    match key {
        KeyInput::Code(KeyCode {
            char: Some(c), ..
        }) => {
            let lower = c.to_lowercase().next().unwrap_or(*c);
            KeyInput::Code(KeyCode::from_char(lower))
        }
        KeyInput::Key(k) => {
            if let Some(base) = k.normal_modifier() {
                KeyInput::Key(base)
            } else if let Some(vk) = imp::canonical_vk(*k) {
                KeyInput::Code(KeyCode::from_vk(vk))
            } else {
                key.clone()
            }
        }
        _ => key.clone(),
    }
}
