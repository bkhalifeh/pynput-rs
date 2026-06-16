// Tests for the platform-independent logic.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use pynput::keyboard::{canonical, HotKey, Key, KeyCode, KeyInput};

#[test]
fn keycode_constructors() {
    let a = KeyCode::from_char('a');
    assert_eq!(a.char, Some('a'));
    assert!(a.vk.is_none());

    let vk = KeyCode::from_vk(0xFF0D);
    assert_eq!(vk.vk, Some(0xFF0D));
    assert!(vk.char.is_none());

    assert_eq!(KeyCode::from_char('a'), KeyCode::from_char('a'));
    assert_ne!(KeyCode::from_char('a'), KeyCode::from_char('b'));
    assert_eq!(KeyCode::from_vk(5), KeyCode::from_vk(5));
}

#[test]
fn keycode_hash_set_membership() {
    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(KeyCode::from_char('a'));
    set.insert(KeyCode::from_vk(42));
    assert!(set.contains(&KeyCode::from_char('a')));
    assert!(set.contains(&KeyCode::from_vk(42)));
    assert!(!set.contains(&KeyCode::from_char('z')));
}

#[test]
fn dead_key_join_compose() {
    // '~' is the standalone form of COMBINING TILDE.
    let dead = KeyCode::from_dead('~').expect("tilde is a dead key");
    assert!(dead.is_dead);
    assert!(dead.combining.is_some());

    // Joining with 'a' yields 'ã'.
    let joined = dead.join(&KeyCode::from_char('a')).expect("compose a~");
    assert_eq!(joined.char, Some('ã'));
    assert!(!joined.is_dead);

    // Joining with space yields the non-dead standalone character.
    let space = dead.join(&KeyCode::from_char(' ')).expect("join space");
    assert_eq!(space.char, Some('~'));
    assert!(!space.is_dead);

    // Joining with itself also yields the non-dead form.
    let itself = dead.join(&dead).expect("join self");
    assert_eq!(itself.char, Some('~'));
}

#[test]
fn dead_key_join_invalid() {
    let dead = KeyCode::from_dead('~').unwrap();
    // 'q' has no precomposed tilde form.
    assert!(dead.join(&KeyCode::from_char('q')).is_err());
    // A non-dead key cannot be joined.
    assert!(KeyCode::from_char('a').join(&KeyCode::from_char('b')).is_err());
}

#[test]
fn key_name_roundtrip() {
    for key in Key::ALL {
        assert_eq!(Key::from_name(key.name()), Some(key));
    }
    assert_eq!(Key::from_name("ctrl"), Some(Key::Ctrl));
    assert_eq!(Key::from_name("nope"), None);
}

#[test]
fn key_normal_modifier() {
    assert_eq!(Key::ShiftL.normal_modifier(), Some(Key::Shift));
    assert_eq!(Key::ShiftR.normal_modifier(), Some(Key::Shift));
    assert_eq!(Key::Ctrl.normal_modifier(), Some(Key::Ctrl));
    assert_eq!(Key::AltGr.normal_modifier(), Some(Key::AltGr));
    assert_eq!(Key::F1.normal_modifier(), None);
}

#[test]
fn hotkey_parse_basic() {
    let parsed = HotKey::parse("<ctrl>+<alt>+h").unwrap();
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0], KeyInput::Key(Key::Ctrl));
    assert_eq!(parsed[1], KeyInput::Key(Key::Alt));
    assert_eq!(parsed[2], KeyInput::Code(KeyCode::from_char('h')));
}

#[test]
fn hotkey_parse_uppercase_normalised() {
    // Single characters are lower-cased.
    let parsed = HotKey::parse("<ctrl>+A").unwrap();
    assert_eq!(parsed[1], KeyInput::Code(KeyCode::from_char('a')));
}

#[test]
fn hotkey_parse_errors() {
    assert!(HotKey::parse("").is_err());
    assert!(HotKey::parse("a+").is_err());
    assert!(HotKey::parse("<nosuchkey>").is_err());
    // Duplicate parts are rejected.
    assert!(HotKey::parse("a+a").is_err());
}

#[test]
fn hotkey_activation() {
    let count = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&count);
    let mut hk = HotKey::new(
        HotKey::parse("<ctrl>+a").unwrap(),
        move || {
            c.fetch_add(1, Ordering::SeqCst);
        },
    );

    // Pressing only part of the combination does not activate.
    hk.press(KeyInput::Key(Key::Ctrl));
    assert_eq!(count.load(Ordering::SeqCst), 0);

    // Completing the combination activates exactly once.
    hk.press(KeyInput::Code(KeyCode::from_char('a')));
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // Re-pressing does not re-activate until a release happens.
    hk.press(KeyInput::Code(KeyCode::from_char('a')));
    assert_eq!(count.load(Ordering::SeqCst), 1);

    hk.release(KeyInput::Code(KeyCode::from_char('a')));
    hk.press(KeyInput::Code(KeyCode::from_char('a')));
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn canonical_normalises() {
    // Upper-case characters become lower case.
    assert_eq!(
        canonical(&KeyInput::Code(KeyCode::from_char('A'))),
        KeyInput::Code(KeyCode::from_char('a'))
    );
    // Specific modifiers become their generic form.
    assert_eq!(
        canonical(&KeyInput::Key(Key::ShiftL)),
        KeyInput::Key(Key::Shift)
    );
}
