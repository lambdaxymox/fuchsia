// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use {
    super::gesture_arena::{
        self, ExamineEventResult, ProcessBufferedEventsResult, RecognizedGesture, TouchpadEvent,
        VerifyEventResult,
    },
    crate::mouse_binding::{MouseButton, MouseEvent, MouseLocation, MousePhase, RelativeLocation},
    crate::utils::Position,
    fuchsia_zircon as zx,
    maplit::hashset,
};

const PRIMARY_BUTTON: MouseButton = 1;

/// The initial state of this recognizer, before a tap has been detected.
#[derive(Debug)]
struct InitialContender {
    /// The maximum displacement that a detected finger can withstand to still
    /// be considered a tap. Measured in millimeters.
    max_finger_displacement: f32,

    /// The maximum time that can elapse between a finger down and finger up
    /// to be considered a tap gesture.
    max_time_elapsed: zx::Duration,
}

impl InitialContender {
    fn into_finger_down_contender(
        self: Box<Self>,
        finger_down_event: TouchpadEvent,
    ) -> Box<dyn gesture_arena::Contender> {
        Box::new(FingerDownContender {
            max_finger_displacement: self.max_finger_displacement,
            max_time_elapsed: self.max_time_elapsed,
            finger_down_event,
        })
    }
}

impl gesture_arena::Contender for InitialContender {
    fn examine_event(self: Box<Self>, event: &TouchpadEvent) -> ExamineEventResult {
        if event.contacts.len() != 1 {
            return ExamineEventResult::Mismatch;
        }

        match event.pressed_buttons.len() {
            0 => ExamineEventResult::Contender(self.into_finger_down_contender(event.clone())),
            _ => ExamineEventResult::Mismatch,
        }
    }
}

/// The state when this recognizer has detected a single finger down.
#[derive(Debug)]
struct FingerDownContender {
    /// The maximum displacement that a detected finger can withstand to still
    /// be considered a tap. Measured in millimeters.
    max_finger_displacement: f32,

    /// The maximum time that can elapse between a finger down and finger up
    /// to be considered a tap gesture.
    max_time_elapsed: zx::Duration,

    /// The TouchpadEvent when a finger down was first detected.
    finger_down_event: TouchpadEvent,
}

impl FingerDownContender {
    fn into_matched_contender(
        self: Box<Self>,
        finger_up_event: TouchpadEvent,
    ) -> Box<dyn gesture_arena::MatchedContender> {
        Box::new(MatchedContender {
            finger_down_event: self.finger_down_event,
            finger_up_event,
            max_time_elapsed: self.max_time_elapsed,
        })
    }
}

impl gesture_arena::Contender for FingerDownContender {
    fn examine_event(self: Box<Self>, event: &TouchpadEvent) -> ExamineEventResult {
        if !is_valid_event_time(event, &self.finger_down_event, self.max_time_elapsed) {
            return ExamineEventResult::Mismatch;
        }

        if event.pressed_buttons.len() != 0 {
            return ExamineEventResult::Mismatch;
        }

        match event.contacts.len() {
            0 => ExamineEventResult::MatchedContender(self.into_matched_contender(event.clone())),
            1 => {
                if !position_is_in_tap_threshold(
                    position_from_event(event),
                    position_from_event(&self.finger_down_event),
                    self.max_finger_displacement,
                ) {
                    return ExamineEventResult::Mismatch;
                }
                ExamineEventResult::Contender(self)
            }
            _ => ExamineEventResult::Mismatch,
        }
    }
}

/// The state when this recognizer has detected a tap, but the gesture arena
/// has not declared this recognizer the winner.
#[derive(Debug)]
struct MatchedContender {
    /// The TouchpadEvent when a finger down was first detected.
    finger_down_event: TouchpadEvent,

    /// The TouchpadEvent when a finger up was first detected.
    finger_up_event: TouchpadEvent,

    /// The maximum time that can elapse between a finger down and finger up
    /// to be considered a tap gesture.
    max_time_elapsed: zx::Duration,
}

impl gesture_arena::MatchedContender for MatchedContender {
    fn verify_event(self: Box<Self>, event: &TouchpadEvent) -> VerifyEventResult {
        if !is_valid_event_time(event, &self.finger_down_event, self.max_time_elapsed) {
            return VerifyEventResult::Mismatch;
        }

        if event.contacts.len() != 0 {
            return VerifyEventResult::Mismatch;
        }

        if event.pressed_buttons.len() != 0 {
            return VerifyEventResult::Mismatch;
        }

        VerifyEventResult::MatchedContender(self)
    }

    fn process_buffered_events(
        self: Box<Self>,
        _events: Vec<TouchpadEvent>,
    ) -> ProcessBufferedEventsResult {
        ProcessBufferedEventsResult {
            generated_events: vec![
                gesture_arena::MouseEvent {
                    _timestamp: self.finger_down_event.timestamp,
                    _mouse_data: MouseEvent {
                        location: MouseLocation::Relative(RelativeLocation {
                            counts: Position::zero(),
                            millimeters: Position::zero(),
                        }),
                        wheel_delta_v: None,
                        wheel_delta_h: None,
                        phase: MousePhase::Down,
                        affected_buttons: hashset! {PRIMARY_BUTTON},
                        pressed_buttons: hashset! {PRIMARY_BUTTON},
                    },
                },
                gesture_arena::MouseEvent {
                    _timestamp: self.finger_up_event.timestamp,
                    _mouse_data: MouseEvent {
                        location: MouseLocation::Relative(RelativeLocation {
                            counts: Position::zero(),
                            millimeters: Position::zero(),
                        }),
                        wheel_delta_v: None,
                        wheel_delta_h: None,
                        phase: MousePhase::Up,
                        affected_buttons: hashset! {PRIMARY_BUTTON},
                        pressed_buttons: hashset! {},
                    },
                },
            ],
            winner: None,
            recognized_gesture: RecognizedGesture::PrimaryTap,
        }
    }
}

/// This function returns the position associated with a TouchpadEvent that is
/// assumed to have a single associated TouchContact.
fn position_from_event(event: &TouchpadEvent) -> Position {
    event.contacts[0].position
}

/// Returns true iff the Euclidean distance for the displacement indicated
/// between (pos1.x, pos1.y) and (pos2.x, pos2.y) is less than the tap
/// threshold.
fn position_is_in_tap_threshold(pos1: Position, pos2: Position, threshold: f32) -> bool {
    let delta_x = pos1.x - pos2.x;
    let delta_y = pos1.y - pos2.y;
    (delta_x * delta_x + delta_y * delta_y).sqrt() < threshold
}

/// Returns true iff the timestamp for new_event has not exceeded
/// the threshold since the old_event timestamp.
fn is_valid_event_time(
    new_event: &TouchpadEvent,
    old_event: &TouchpadEvent,
    threshold: zx::Duration,
) -> bool {
    new_event.timestamp - old_event.timestamp < threshold
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::gestures::gesture_arena::{Contender, MatchedContender as _, TouchpadEvent},
        crate::testing_utilities::create_touch_contact,
        assert_matches::assert_matches,
        std::any::TypeId,
    };

    const MAX_TIME_ELAPSED: zx::Duration = zx::Duration::from_nanos(10000);
    const MAX_FINGER_DISPLACEMENT: f32 = 10.0;
    const HALF_MOTION: f32 = MAX_FINGER_DISPLACEMENT / 2.0;

    fn assert_finger_down_contender(result: ExamineEventResult) {
        match result {
            ExamineEventResult::Contender(boxed) => {
                assert_eq!((&*boxed).as_any().type_id(), TypeId::of::<FingerDownContender>());
            }
            other => panic!("Expected a Contender but found {:?}", other),
        }
    }

    fn assert_examined_matched_contender(result: ExamineEventResult) {
        match result {
            ExamineEventResult::MatchedContender(boxed) => {
                assert_eq!((&*boxed).as_any().type_id(), TypeId::of::<MatchedContender>());
            }
            other => panic!("Expected a MatchedContender but found {:?}", other),
        }
    }

    fn assert_verified_matched_contender(result: VerifyEventResult) {
        match result {
            VerifyEventResult::MatchedContender(boxed) => {
                assert_eq!((&*boxed).as_any().type_id(), TypeId::of::<MatchedContender>());
            }
            other => panic!("Expected a MatchedContender but found {:?}", other),
        }
    }

    /// Tests that an InitialContender with zero touch contacts yields a
    /// Mismatch.
    #[fuchsia::test]
    fn contender_no_touch_contacts() {
        assert_matches!(
            Box::new(InitialContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
            ExamineEventResult::Mismatch
        );
    }

    /// Tests that an InitialContender with multiple touch contacts yields a
    /// Mismatch.
    #[fuchsia::test]
    fn contender_many_touch_contacts() {
        assert_matches!(
            Box::new(InitialContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![
                    create_touch_contact(0, Position::zero()),
                    create_touch_contact(1, Position::zero())
                ],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
            ExamineEventResult::Mismatch
        );
    }

    /// Tests that an InitialContender with a single touch contact and one
    /// pressed button yields a Mismatch.
    #[fuchsia::test]
    fn contender_single_button() {
        assert_matches!(
            Box::new(InitialContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![create_touch_contact(0, Position::zero())],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![0],
            },),
            ExamineEventResult::Mismatch
        );
    }

    /// Tests that an InitialContender with a single touch contact and multiple
    /// pressed button yields a Mismatch.
    #[fuchsia::test]
    fn contender_many_buttons() {
        assert_matches!(
            Box::new(InitialContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![create_touch_contact(0, Position::zero())],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![0, 1],
            },),
            ExamineEventResult::Mismatch
        );
    }

    /// Tests that an InitialContender with a single touch contact and no
    /// pressed buttons yields a FingerDownContender.
    #[fuchsia::test]
    fn contender_no_buttons() {
        assert_finger_down_contender(
            Box::new(InitialContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED,
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![create_touch_contact(0, Position::zero())],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
        );
    }

    /// Tests that a FingerDownContender with an event whose timestamp exceeds
    /// the elapsed threshold yields a Mismatch.
    #[fuchsia::test]
    fn finger_down_contender_too_long() {
        assert_matches!(
            Box::new(FingerDownContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED,
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![],
                timestamp: MAX_TIME_ELAPSED + zx::Time::from_nanos(1),
                pressed_buttons: vec![],
            }),
            ExamineEventResult::Mismatch
        );
    }

    /// Tests that a FingerDownContender with zero touch contacts yields a
    /// MatchedContender.
    #[fuchsia::test]
    fn finger_down_contender_no_touch_contacts() {
        assert_examined_matched_contender(
            Box::new(FingerDownContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED,
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
        );
    }

    /// Tests that a FingerDownContender with multiple touch contacts yields a
    /// Mismatch.
    #[fuchsia::test]
    fn finger_down_contender_many_touch_contacts() {
        assert_matches!(
            Box::new(FingerDownContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED,
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![
                    create_touch_contact(0, Position::zero()),
                    create_touch_contact(1, Position::zero())
                ],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
            ExamineEventResult::Mismatch
        );
    }

    /// Tests that a FingerDownContender with a single touch contact and
    /// too much displacement yields a Mismatch.
    #[fuchsia::test]
    fn finger_down_contender_large_displacement() {
        assert_matches!(
            Box::new(FingerDownContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED,
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![create_touch_contact(
                    0,
                    Position { x: MAX_FINGER_DISPLACEMENT, y: MAX_FINGER_DISPLACEMENT }
                )],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
            ExamineEventResult::Mismatch
        );
    }

    /// Tests that a FingerDownContender with a single touch contact,
    /// no displacement, and no pressed buttons yields a
    /// FingerDownContender.
    #[fuchsia::test]
    fn finger_down_contender_no_buttons_no_displacement() {
        assert_finger_down_contender(
            Box::new(FingerDownContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED,
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![create_touch_contact(0, Position::zero())],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
        );
    }

    /// Tests that a FingerDownContender with a single touch contact,
    /// acceptable displacement, and no pressed buttons yields a
    /// FingerDownContender.
    #[fuchsia::test]
    fn finger_down_contender_no_buttons_some_displacement() {
        assert_finger_down_contender(
            Box::new(FingerDownContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED,
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![create_touch_contact(
                    0,
                    Position { x: HALF_MOTION, y: HALF_MOTION },
                )],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
        );
    }

    /// Tests that a FingerDownContender with a single touch contact,
    /// acceptable displacement, and one pressed button yields a
    /// Mismatch.
    #[fuchsia::test]
    fn finger_down_contender_single_button() {
        assert_matches!(
            Box::new(FingerDownContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED,
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![create_touch_contact(
                    0,
                    Position { x: HALF_MOTION, y: HALF_MOTION }
                )],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![0],
            }),
            ExamineEventResult::Mismatch
        );
    }

    /// Tests that a FingerDownContender with a single touch contact,
    /// acceptable displacement, and multiple pressed buttons yields a
    /// Mismatch.
    #[fuchsia::test]
    fn finger_down_contender_many_buttons() {
        assert_matches!(
            Box::new(FingerDownContender {
                max_finger_displacement: MAX_FINGER_DISPLACEMENT,
                max_time_elapsed: MAX_TIME_ELAPSED,
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
            })
            .examine_event(&TouchpadEvent {
                contacts: vec![create_touch_contact(
                    0,
                    Position { x: HALF_MOTION, y: HALF_MOTION }
                )],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![0, 1],
            }),
            ExamineEventResult::Mismatch
        );
    }

    /// Tests that a MatchedContender with no button pressed yields a Mismatch.
    #[fuchsia::test]
    fn matched_contender_no_buttons() {
        assert_matches!(
            Box::new(MatchedContender {
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                finger_up_event: TouchpadEvent {
                    contacts: vec![],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                max_time_elapsed: MAX_TIME_ELAPSED,
            })
            .verify_event(&TouchpadEvent {
                contacts: vec![],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![0],
            }),
            VerifyEventResult::Mismatch
        );
    }

    /// Tests that a MatchedContender with multiple buttons pressed yields a
    /// Mismatch.
    #[fuchsia::test]
    fn matched_contender_many_buttons() {
        assert_matches!(
            Box::new(MatchedContender {
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                finger_up_event: TouchpadEvent {
                    contacts: vec![],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                max_time_elapsed: MAX_TIME_ELAPSED,
            })
            .verify_event(&TouchpadEvent {
                contacts: vec![],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![0, 1],
            }),
            VerifyEventResult::Mismatch
        );
    }

    /// Tests that a MatchedContender with one touch contact yields a Mismatch.
    #[fuchsia::test]
    fn matched_contender_single_touch_contact() {
        assert_matches!(
            Box::new(MatchedContender {
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                finger_up_event: TouchpadEvent {
                    contacts: vec![],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                max_time_elapsed: MAX_TIME_ELAPSED,
            })
            .verify_event(&TouchpadEvent {
                contacts: vec![create_touch_contact(0, Position::zero())],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
            VerifyEventResult::Mismatch
        );
    }

    /// Tests that a MatchedContender with multiple touch contacts yields a
    /// Mismatch.
    #[fuchsia::test]
    fn matched_contender_many_touch_contacts() {
        assert_matches!(
            Box::new(MatchedContender {
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![0],
                },
                finger_up_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                max_time_elapsed: MAX_TIME_ELAPSED,
            })
            .verify_event(&TouchpadEvent {
                contacts: vec![
                    create_touch_contact(0, Position::zero()),
                    create_touch_contact(1, Position::zero())
                ],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
            VerifyEventResult::Mismatch
        );
    }

    /// Tests that a MatchedContender with an event whose timestamp exceeds
    /// the elapsed threshold yields a Mismatch.
    #[fuchsia::test]
    fn matched_contender_no_contacts_no_buttons_too_long() {
        assert_matches!(
            Box::new(MatchedContender {
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                finger_up_event: TouchpadEvent {
                    contacts: vec![],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                max_time_elapsed: MAX_TIME_ELAPSED,
            })
            .verify_event(&TouchpadEvent {
                contacts: vec![],
                timestamp: MAX_TIME_ELAPSED + zx::Time::from_nanos(1),
                pressed_buttons: vec![],
            }),
            VerifyEventResult::Mismatch
        );
    }

    /// Tests that a MatchedContender with no buttons or touch contacts
    /// yields a MatchedContender.
    #[fuchsia::test]
    fn matched_contender_no_contacts_no_buttons() {
        assert_verified_matched_contender(
            Box::new(MatchedContender {
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                finger_up_event: TouchpadEvent {
                    contacts: vec![],
                    timestamp: zx::Time::from_nanos(0),
                    pressed_buttons: vec![],
                },
                max_time_elapsed: MAX_TIME_ELAPSED,
            })
            .verify_event(&TouchpadEvent {
                contacts: vec![],
                timestamp: zx::Time::from_nanos(0),
                pressed_buttons: vec![],
            }),
        );
    }

    /// Tests that a MatchedContender processes buffered events by
    /// returning mouse down and mouse up events.
    #[fuchsia::test]
    fn matched_contender_process_buffered_events() {
        let timestamp = zx::Time::from_nanos(0);
        let ProcessBufferedEventsResult { generated_events, winner, recognized_gesture } =
            Box::new(MatchedContender {
                finger_down_event: TouchpadEvent {
                    contacts: vec![create_touch_contact(0, Position::zero())],
                    timestamp,
                    pressed_buttons: vec![],
                },
                finger_up_event: TouchpadEvent {
                    contacts: vec![],
                    timestamp,
                    pressed_buttons: vec![],
                },
                max_time_elapsed: MAX_TIME_ELAPSED,
            })
            .process_buffered_events(vec![]);

        assert_eq!(
            generated_events,
            [
                gesture_arena::MouseEvent {
                    _timestamp: timestamp,
                    _mouse_data: MouseEvent {
                        location: MouseLocation::Relative(RelativeLocation {
                            counts: Position { x: 0.0, y: 0.0 },
                            millimeters: Position { x: 0.0, y: 0.0 }
                        }),
                        wheel_delta_v: None,
                        wheel_delta_h: None,
                        phase: MousePhase::Down,
                        affected_buttons: hashset! {PRIMARY_BUTTON},
                        pressed_buttons: hashset! {PRIMARY_BUTTON},
                    },
                },
                gesture_arena::MouseEvent {
                    _timestamp: timestamp,
                    _mouse_data: MouseEvent {
                        location: MouseLocation::Relative(RelativeLocation {
                            counts: Position { x: 0.0, y: 0.0 },
                            millimeters: Position { x: 0.0, y: 0.0 }
                        }),
                        wheel_delta_v: None,
                        wheel_delta_h: None,
                        phase: MousePhase::Up,
                        affected_buttons: hashset! {PRIMARY_BUTTON},
                        pressed_buttons: hashset! {},
                    },
                }
            ]
        );
        assert_matches!(winner, None);
        assert_eq!(recognized_gesture, RecognizedGesture::PrimaryTap);
    }
}
