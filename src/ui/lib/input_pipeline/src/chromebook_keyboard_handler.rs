// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! # Fixups for the Chromebook keyboard
//!
//! Chromebook keyboards have a top row of "action" keys, which are usually
//! reported as function keys.  The correct functionality would be to allow
//! them to be used as either function or action keys, depending on whether
//! the "search" key is being actuated alongside one of the keys.
//!
//! The "Search" key in Chromebooks is used to substitute for a range of keys
//! that are not usually present on a Chromebook keyboard.  This Handler
//! implements some of those.

use crate::input_device::{
    Handled, InputDeviceDescriptor, InputDeviceEvent, InputEvent, UnhandledInputEvent,
};
use crate::input_handler::UnhandledInputHandler;
use crate::keyboard_binding::{KeyboardDeviceDescriptor, KeyboardEvent};
use async_trait::async_trait;
use fidl_fuchsia_input::Key;
use fidl_fuchsia_ui_input3::KeyEventType;
use fuchsia_trace as ftrace;
use fuchsia_zircon as zx;
use keymaps::KeyState;
use lazy_static::lazy_static;
use maplit::hashmap;
use std::{cell::RefCell, rc::Rc};

/// The vendor ID denoting the internal Chromebook keyboard.
const VENDOR_ID: u32 = 0x18d1; // Google

/// The product ID denoting the internal Chromebook keyboard.
const PRODUCT_ID: u32 = 0x10003;

//// The Chromebook "Search" key is reported as left meta.
const SEARCH_KEY: Key = Key::LeftMeta;

lazy_static! {
    // Map key is the original key code produced by the keyboard. Map value is
    // the remapped key when the "SEARCH" key is active pressed.
    static ref REMAPPED_KEYS: std::collections::HashMap<Key, Key> = hashmap! {
        Key::F1 => Key::AcBack,
        Key::F2 => Key::AcRefresh,
        Key::F3 => Key::AcFullScreenView,
        Key::F4 => Key::AcSelectTaskApplication,
        Key::F5 => Key::BrightnessDown,
        Key::F6 => Key::BrightnessUp,
        Key::F7 => Key::PlayPause,
        Key::F8 => Key::Mute,
        Key::F9 => Key::VolumeDown,
        Key::F10 => Key::VolumeUp,
        // PgUp, PgDn, Insert, Delete, End, Home.
    };
}

/// A Chromebook dedicated keyboard handler.
///
/// Create a new instance with [ChromebookKeyboardHandler::new].
#[derive(Debug, Default)]
pub struct ChromebookKeyboardHandler {
    // Handler's mutable state must be accessed via RefCell.
    state: RefCell<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// The list of keys (using original key codes, not remapped ones) that are
    /// currently actuated.
    key_state: KeyState,
    /// Is the search key currently activated.
    is_search_key_actuated: bool,
    /// Were there any new keyboard events generated by this handler, or
    /// received by this handler, since the Search key was last pressed?
    other_key_events: bool,
    /// The minimum timestamp that is still larger than the last observed
    /// timestamp (i.e. last + 1ns). Used to generate monotonically increasing
    /// timestamps for generated events.
    next_event_time: zx::Time,
    /// Have any regular (non-remapped) keys been pressed since the actuation
    /// of the Search key?
    regular_keys_pressed: bool,
}

/// Returns true if the provided device info matches the Chromebook keyboard.
fn is_chromebook_keyboard(device_info: &fidl_fuchsia_input_report::DeviceInfo) -> bool {
    device_info.product_id == PRODUCT_ID && device_info.vendor_id == VENDOR_ID
}

#[async_trait(?Send)]
impl UnhandledInputHandler for ChromebookKeyboardHandler {
    async fn handle_unhandled_input_event(
        self: Rc<Self>,
        input_event: UnhandledInputEvent,
    ) -> Vec<InputEvent> {
        match input_event {
            // Decorate a keyboard event with key meaning.
            UnhandledInputEvent {
                device_event: InputDeviceEvent::Keyboard(event),
                device_descriptor: InputDeviceDescriptor::Keyboard(ref keyboard_descriptor),
                event_time,
                trace_id,
            } if is_chromebook_keyboard(&keyboard_descriptor.device_info) => self
                .process_keyboard_event(event, keyboard_descriptor.clone(), event_time, trace_id),
            // Pass other events unchanged.
            _ => vec![InputEvent::from(input_event)],
        }
    }
}

impl ChromebookKeyboardHandler {
    /// Creates a new instance of the handler.
    pub fn new() -> Rc<Self> {
        Rc::new(Default::default())
    }

    /// Gets the next event time that is at least as large as event_time, and
    /// larger than last seen time.
    fn next_event_time(self: &Rc<Self>, event_time: zx::Time) -> zx::Time {
        let proposed = self.state.borrow().next_event_time;
        let returned = if event_time < proposed { proposed } else { event_time };
        self.state.borrow_mut().next_event_time = returned + zx::Duration::from_nanos(1);
        returned
    }

    /// Updates the internal key state, but only for remappable keys.
    fn update_key_state(self: &Rc<Self>, event_type: KeyEventType, key: Key) {
        if REMAPPED_KEYS.contains_key(&key) {
            self.state.borrow_mut().key_state.update(event_type, key)
        }
    }

    /// Gets the list of keys in the key state, in order they were pressed.
    fn get_ordered_keys(self: &Rc<Self>) -> Vec<Key> {
        self.state.borrow().key_state.get_ordered_keys()
    }

    fn is_search_key_actuated(self: &Rc<Self>) -> bool {
        self.state.borrow().is_search_key_actuated
    }

    fn set_search_key_actuated(self: &Rc<Self>, value: bool) {
        self.state.borrow_mut().is_search_key_actuated = value;
    }

    fn has_other_key_events(self: &Rc<Self>) -> bool {
        self.state.borrow().other_key_events
    }

    fn set_other_key_events(self: &Rc<Self>, value: bool) {
        self.state.borrow_mut().other_key_events = value;
    }

    fn is_regular_keys_pressed(self: &Rc<Self>) -> bool {
        self.state.borrow().regular_keys_pressed
    }

    fn set_regular_keys_pressed(self: &Rc<Self>, value: bool) {
        self.state.borrow_mut().regular_keys_pressed = value;
    }

    /// Remaps hardware events.
    fn process_keyboard_event(
        self: &Rc<Self>,
        event: KeyboardEvent,
        device_descriptor: KeyboardDeviceDescriptor,
        event_time: zx::Time,
        trace_id: Option<ftrace::Id>,
    ) -> Vec<InputEvent> {
        // Remapping happens when search key is *not* actuated. The keyboard
        // sends the F1 key code, but we must convert it by default to AcBack,
        // for example.
        let pass_keys_unchanged = self.is_search_key_actuated();

        let key = event.get_key();
        let event_type_folded = event.get_event_type_folded();
        let event_type = event.get_event_type();

        match pass_keys_unchanged {
            true => {
                // If the meta key is released, turn the remapping off, but also flip remapping of
                // any currently active keys.
                if key == SEARCH_KEY && event_type_folded == KeyEventType::Released {
                    // Used to synthesize new events.
                    let keys_to_release = self.get_ordered_keys();

                    // No more remapping: flip any active keys to non-remapped, and continue.
                    let mut new_events = vec![];
                    for released_key in keys_to_release.iter().rev() {
                        new_events.push(into_unhandled_input_event(
                            event
                                // Cloned to ensure unrelated fields are propagated. Most of
                                // the other fields should anyways be set to None this early
                                // in the input pipeline.
                                .clone()
                                .into_with_key(*released_key)
                                .into_with_event_type(KeyEventType::Released),
                            device_descriptor.clone(),
                            self.next_event_time(event_time),
                            None,
                        ));
                    }
                    for released_key in keys_to_release.iter() {
                        let remapped_key = REMAPPED_KEYS
                            .get(released_key)
                            .expect("released_key must be in REMAPPED_KEYS");
                        new_events.push(into_unhandled_input_event(
                            event
                                .clone()
                                .into_with_key(*remapped_key)
                                .into_with_event_type(KeyEventType::Pressed),
                            device_descriptor.clone(),
                            self.next_event_time(event_time),
                            None,
                        ));
                    }

                    // The Search key serves a dual purpose: it is a "silent" modifier for
                    // action keys, and it also can serve as a left Meta key if pressed on
                    // its own, or in combination with a key that does not normally get
                    // remapped. Such would be the case of Meta+A, where we must synthesize
                    // a Meta press, then press of A, then the respective releases. Contrast
                    // to Search+AcBack, which only results in F1, without Meta.
                    let search_key_only =
                        !self.has_other_key_events() && event_type == KeyEventType::Released;
                    // If there were no intervening events between a press and a release of the
                    // Search key, then emulate a press.
                    if search_key_only {
                        new_events.push(into_unhandled_input_event(
                            event.clone().into_with_event_type(KeyEventType::Pressed),
                            device_descriptor.clone(),
                            self.next_event_time(event_time),
                            None,
                        ));
                    }
                    // Similarly, emulate a release too, in two cases:
                    //
                    // 1) No intervening presses (like above); and
                    // 2) There was a non-remapped key used with Search.
                    if search_key_only || self.is_regular_keys_pressed() {
                        new_events.push(into_unhandled_input_event(
                            event.clone().into_with_event_type(KeyEventType::Released),
                            device_descriptor.clone(),
                            self.next_event_time(event_time),
                            None,
                        ));
                    }

                    // Reset search key state tracking to initial values.
                    self.set_search_key_actuated(false);
                    self.set_other_key_events(false);
                    self.set_regular_keys_pressed(false);

                    return new_events;
                } else {
                    // Any other key press or release that is not the Search key.
                }
            }
            false => {
                if key == SEARCH_KEY && event_type == KeyEventType::Pressed {
                    // Used to synthesize new events.
                    let keys_to_release = self.get_ordered_keys();

                    let mut new_events = vec![];
                    for released_key in keys_to_release.iter().rev() {
                        let remapped_key = REMAPPED_KEYS
                            .get(released_key)
                            .expect("released_key must be in REMAPPED_KEYS");
                        new_events.push(into_unhandled_input_event(
                            event
                                .clone()
                                .into_with_key(*remapped_key)
                                .into_with_event_type(KeyEventType::Released),
                            device_descriptor.clone(),
                            self.next_event_time(event_time),
                            None,
                        ));
                    }
                    for released_key in keys_to_release.iter() {
                        new_events.push(into_unhandled_input_event(
                            event
                                .clone()
                                .into_with_key(*released_key)
                                .into_with_event_type(KeyEventType::Pressed),
                            device_descriptor.clone(),
                            self.next_event_time(event_time),
                            None,
                        ));
                    }

                    self.set_search_key_actuated(true);
                    if !keys_to_release.is_empty() {
                        self.set_other_key_events(true);
                    }
                    return new_events;
                }
            }
        }

        self.update_key_state(event_type, key);
        let maybe_remapped_key = REMAPPED_KEYS.get(&key);
        let return_events = if let Some(remapped_key) = maybe_remapped_key && !pass_keys_unchanged {
            vec![into_unhandled_input_event(
                event.into_with_key(*remapped_key),
                device_descriptor,
                self.next_event_time(event_time),
                trace_id,
            )]
        } else {
            let mut events = vec![];
            // If this is the first non-remapped keypress after SEARCH_KEY actuation, we must emit
            // the modifier before the key itself, because now we know that the user's intent was
            // to use a modifier, not to remap action keys into function keys.
            if maybe_remapped_key.is_none() && self.is_search_key_actuated() &&
                !self.has_other_key_events() && event_type == KeyEventType::Pressed {
                let new_event = event.clone().into_with_key(SEARCH_KEY).into_with_event_type(KeyEventType::Pressed);
                events.push(into_unhandled_input_event(
                        new_event, device_descriptor.clone(), self.next_event_time(event_time), None));
                self.set_regular_keys_pressed(true);
            }
            events.push(into_unhandled_input_event(
                    event, device_descriptor, self.next_event_time(event_time), trace_id));
            //
            // Set "non-remapped-key".
            events
        };

        // Remember that there were keypresses other than SEARCH_KEY after
        // SEARCH_KEY was actuated.
        if event_type == KeyEventType::Pressed && key != SEARCH_KEY && pass_keys_unchanged {
            self.set_other_key_events(true);
        }

        return_events
    }
}

fn into_unhandled_input_event(
    event: KeyboardEvent,
    device_descriptor: KeyboardDeviceDescriptor,
    event_time: zx::Time,
    trace_id: Option<ftrace::Id>,
) -> InputEvent {
    InputEvent {
        device_event: InputDeviceEvent::Keyboard(event),
        device_descriptor: device_descriptor.into(),
        event_time,
        handled: Handled::No,
        trace_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing_utilities::create_input_event;
    use fidl_fuchsia_ui_input3::KeyEventType;
    use std::convert::TryInto;
    use test_case::test_case;

    lazy_static! {
        static ref MATCHING_KEYBOARD_DESCRIPTOR: InputDeviceDescriptor =
            InputDeviceDescriptor::Keyboard(KeyboardDeviceDescriptor {
                keys: vec![],
                device_info: fidl_fuchsia_input_report::DeviceInfo {
                    vendor_id: VENDOR_ID,
                    product_id: PRODUCT_ID,
                    version: 42,
                },
                device_id: 43,
            });
        static ref MISMATCHING_KEYBOARD_DESCRIPTOR: InputDeviceDescriptor =
            InputDeviceDescriptor::Keyboard(KeyboardDeviceDescriptor {
                keys: vec![],
                device_info: fidl_fuchsia_input_report::DeviceInfo {
                    vendor_id: VENDOR_ID + 10,
                    product_id: PRODUCT_ID,
                    version: 42,
                },
                device_id: 43,
            });
    }

    async fn run_all_events<T: UnhandledInputHandler>(
        handler: &Rc<T>,
        events: Vec<InputEvent>,
    ) -> Vec<InputEvent> {
        let handler_clone = || handler.clone();
        let events_futs = events
            .into_iter()
            .map(|e| e.try_into().expect("events are always convertible in tests"))
            .map(|e| handler_clone().handle_unhandled_input_event(e));
        // Is there a good streaming way to achieve this?
        let mut events_set = vec![];
        for events_fut in events_futs.into_iter() {
            events_set.push(events_fut.await);
        }
        events_set.into_iter().flatten().collect()
    }

    // A shorthand to create a sequence of keyboard events for testing.  All events
    // created share a descriptor and a handled marker.  The event time is incremented
    // for each event in turn.
    fn new_key_sequence(
        mut event_time: zx::Time,
        descriptor: &InputDeviceDescriptor,
        handled: Handled,
        keys: Vec<(Key, KeyEventType)>,
    ) -> Vec<InputEvent> {
        let mut ret = vec![];
        for (k, t) in keys {
            ret.push(create_input_event(KeyboardEvent::new(k, t), descriptor, event_time, handled));
            event_time = event_time + zx::Duration::from_nanos(1);
        }
        ret
    }

    #[test]
    fn next_event_time() {
        let handler = ChromebookKeyboardHandler::new();
        assert_eq!(zx::Time::from_nanos(10), handler.next_event_time(zx::Time::from_nanos(10)));
        assert_eq!(zx::Time::from_nanos(11), handler.next_event_time(zx::Time::from_nanos(10)));
        assert_eq!(zx::Time::from_nanos(12), handler.next_event_time(zx::Time::from_nanos(10)));
        assert_eq!(zx::Time::from_nanos(13), handler.next_event_time(zx::Time::from_nanos(13)));
        assert_eq!(zx::Time::from_nanos(14), handler.next_event_time(zx::Time::from_nanos(13)));
    }

    // Basic tests: ensure that function key codes are normally converted into
    // action key codes on the built-in keyboard.
    #[test_case(Key::F1, Key::AcBack; "convert F1")]
    #[test_case(Key::F2, Key::AcRefresh; "convert F2")]
    #[test_case(Key::F3, Key::AcFullScreenView; "convert F3")]
    #[test_case(Key::F4, Key::AcSelectTaskApplication; "convert F4")]
    #[test_case(Key::F5, Key::BrightnessDown; "convert F5")]
    #[test_case(Key::F6, Key::BrightnessUp; "convert F6")]
    #[test_case(Key::F7, Key::PlayPause; "convert F7")]
    #[test_case(Key::F8, Key::Mute; "convert F8")]
    #[test_case(Key::F9, Key::VolumeDown; "convert F9")]
    #[test_case(Key::F10, Key::VolumeUp; "convert F10")]
    #[test_case(Key::A, Key::A; "do not convert A")]
    #[fuchsia::test]
    async fn conversion_matching_keyboard(input_key: Key, output_key: Key) {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![(input_key, KeyEventType::Pressed), (input_key, KeyEventType::Released)],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![(output_key, KeyEventType::Pressed), (output_key, KeyEventType::Released)],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }

    // Basic tests: ensure that function key codes are NOT converted into
    // action key codes on any other keyboard (e.g. external).
    #[test_case(Key::F1, Key::F1; "do not convert F1")]
    #[test_case(Key::F2, Key::F2; "do not convert F2")]
    #[test_case(Key::F3, Key::F3; "do not convert F3")]
    #[test_case(Key::F4, Key::F4; "do not convert F4")]
    #[test_case(Key::F5, Key::F5; "do not convert F5")]
    #[test_case(Key::F6, Key::F6; "do not convert F6")]
    #[test_case(Key::F7, Key::F7; "do not convert F7")]
    #[test_case(Key::F8, Key::F8; "do not convert F8")]
    #[test_case(Key::F9, Key::F9; "do not convert F9")]
    #[test_case(Key::F10, Key::F10; "do not convert F10")]
    #[test_case(Key::A, Key::A; "do not convert A")]
    #[fuchsia::test]
    async fn conversion_mismatching_keyboard(input_key: Key, output_key: Key) {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MISMATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![(input_key, KeyEventType::Pressed), (input_key, KeyEventType::Released)],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(42),
            &MISMATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![(output_key, KeyEventType::Pressed), (output_key, KeyEventType::Released)],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }

    // If a Search key is pressed without intervening keypresses, it results in
    // a delayed press and release.
    //
    // SEARCH_KEY[in]  ___/""""""""\___
    //
    // SEARCH_KEY[out] ____________/""\___
    #[fuchsia::test]
    async fn search_key_only() {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![(SEARCH_KEY, KeyEventType::Pressed), (SEARCH_KEY, KeyEventType::Released)],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(43),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![(SEARCH_KEY, KeyEventType::Pressed), (SEARCH_KEY, KeyEventType::Released)],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }

    // When a remappable key (e.g. F1) is pressed with the Search key, the effect
    // is as if the action key only is pressed.
    //
    // SEARCH_KEY[in]  ___/"""""""""""\___
    // F1[in]          ______/""""\_______
    //
    // SEARCH_KEY[out] ___________________
    // F1[out]         ______/""""\_______
    #[fuchsia::test]
    async fn f1_conversion() {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (SEARCH_KEY, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
                (SEARCH_KEY, KeyEventType::Released),
            ],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(43),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![(Key::F1, KeyEventType::Pressed), (Key::F1, KeyEventType::Released)],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }

    // SEARCH_KEY[in]  __/""""""\________
    // F1[in]          _____/""""""""\___
    //
    // SEARCH_KEY[out] __________________
    // F1[out]         _____/"""\________
    // AcBack[out]     _________/""""\___
    #[fuchsia::test]
    async fn search_released_before_f1() {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (SEARCH_KEY, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Pressed),
                (SEARCH_KEY, KeyEventType::Released),
                (Key::F1, KeyEventType::Released),
            ],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(43),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (Key::F1, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
                (Key::AcBack, KeyEventType::Pressed),
                (Key::AcBack, KeyEventType::Released),
            ],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }

    // When a "regular" key (e.g. "A") is pressed in chord with the Search key,
    // the effect is as if LeftMeta+A was pressed.
    //
    // SEARCH_KEY[in]  ___/"""""""""""\__
    // A[in]           _____/""""\_______
    //
    // SEARCH_KEY[out] _____/"""""""""\__
    // A[out]          ______/""""\______
    #[fuchsia::test]
    async fn search_key_a_is_delayed_leftmeta_a() {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (SEARCH_KEY, KeyEventType::Pressed),
                (Key::A, KeyEventType::Pressed),
                (Key::A, KeyEventType::Released),
                (SEARCH_KEY, KeyEventType::Released),
            ],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(43),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (Key::LeftMeta, KeyEventType::Pressed),
                (Key::A, KeyEventType::Pressed),
                (Key::A, KeyEventType::Released),
                (Key::LeftMeta, KeyEventType::Released),
            ],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }

    // SEARCH_KEY[in]  ___/"""""""""""\___
    // F1[in]          ______/""""\_______
    // F2[in]          _________/""""\____
    //
    // SEARCH_KEY[out] ___________________
    // F1[out]         ______/""""\_______
    // F2[out]         _________/""""\____
    #[fuchsia::test]
    async fn f1_and_f2_interleaved_conversion() {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (SEARCH_KEY, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Pressed),
                (Key::F2, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
                (Key::F2, KeyEventType::Released),
                (SEARCH_KEY, KeyEventType::Released),
            ],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(43),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (Key::F1, KeyEventType::Pressed),
                (Key::F2, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
                (Key::F2, KeyEventType::Released),
            ],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }

    // SEARCH_KEY[in]  _______/""""""\__
    // F1[in]          ___/""""""""\____
    //
    // SEARCH_KEY[out] _________________
    // F1[out]         _______/"""""\___
    // AcBack[out]     __/""""\_________
    #[fuchsia::test]
    async fn search_pressed_before_f1_released() {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (Key::F1, KeyEventType::Pressed),
                (SEARCH_KEY, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
                (SEARCH_KEY, KeyEventType::Released),
            ],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (Key::AcBack, KeyEventType::Pressed),
                (Key::AcBack, KeyEventType::Released),
                (Key::F1, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
            ],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }

    // When the Search key gets actuated when there are already remappable keys
    // actuated, we de-actuate the remapped versions, and actuate remapped ones.
    // This causes the output to observe both F1, F2 and AcBack and AcRefresh.
    //
    // SEARCH_KEY[in]  _______/""""""""""""\___
    // F1[in]          ___/"""""""""\__________
    // F2[in]          _____/""""""""""\_______
    //
    // SEARCH_KEY[out] ________________________
    // F1[out]         _______/"""""\__________
    // AcBack[out]     ___/"""\________________
    // F2[out]         _______/""""""""\_______
    // AcRefresh[out]  _____/"\________________
    #[fuchsia::test]
    async fn search_pressed_while_f1_and_f2_pressed() {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (Key::F1, KeyEventType::Pressed),
                (Key::F2, KeyEventType::Pressed),
                (SEARCH_KEY, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
                (Key::F2, KeyEventType::Released),
                (SEARCH_KEY, KeyEventType::Released),
            ],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (Key::AcBack, KeyEventType::Pressed),
                (Key::AcRefresh, KeyEventType::Pressed),
                (Key::AcRefresh, KeyEventType::Released),
                (Key::AcBack, KeyEventType::Released),
                (Key::F1, KeyEventType::Pressed),
                (Key::F2, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
                (Key::F2, KeyEventType::Released),
            ],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }

    // An interleaving of remapped (F1, F2) and non-remapped keys (A). In this
    // case, A is presented without a modifier.  This is not strictly correct,
    // but is a simpler implementation for an unlikely key combination.
    //
    // SEARCH_KEY[in]  _______/"""""""""\______
    // F1[in]          ___/""""""\_____________
    // A[in]           _________/"""""\________
    // F2[in]          ___________/""""""""\___
    //
    // SEARCH_KEY[out]  _______________________
    // F1[out]          _______/""\_____________
    // AcBack[out]      __/"""\________________
    // A[out]           ________/"""""\________
    // F2[out]          __________/"""""\______
    // AcRefresh[out]  __________________/""\__
    #[fuchsia::test]
    async fn key_combination() {
        let handler = ChromebookKeyboardHandler::new();
        let input = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (Key::F1, KeyEventType::Pressed),
                (SEARCH_KEY, KeyEventType::Pressed),
                (Key::A, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
                (Key::F2, KeyEventType::Pressed),
                (Key::A, KeyEventType::Released),
                (SEARCH_KEY, KeyEventType::Released),
                (Key::F2, KeyEventType::Released),
            ],
        );
        let actual = run_all_events(&handler, input).await;
        let expected = new_key_sequence(
            zx::Time::from_nanos(42),
            &MATCHING_KEYBOARD_DESCRIPTOR,
            Handled::No,
            vec![
                (Key::AcBack, KeyEventType::Pressed),
                (Key::AcBack, KeyEventType::Released),
                (Key::F1, KeyEventType::Pressed),
                (Key::A, KeyEventType::Pressed),
                (Key::F1, KeyEventType::Released),
                (Key::F2, KeyEventType::Pressed),
                (Key::A, KeyEventType::Released),
                (Key::F2, KeyEventType::Released),
                (Key::AcRefresh, KeyEventType::Pressed),
                (Key::AcRefresh, KeyEventType::Released),
            ],
        );
        pretty_assertions::assert_eq!(expected, actual);
    }
}
