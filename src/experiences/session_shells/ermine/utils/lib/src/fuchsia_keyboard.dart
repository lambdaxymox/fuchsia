// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// Defines the mapping of Fuchsia keys to application [Intent]s.
///
/// This is needed because currently the key mapping for Fuchsia in Flutter
/// Framework is broken.
class FuchsiaKeyboard {
  // Fuchsia keyboard HID usage values are defined in (page 53):
  // https://www.usb.org/sites/default/files/documents/hut1_12v2.pdf

  static const int kHidUsagePageMask = 0x70000;
  static const int kFuchsiaKeyIdPlane = LogicalKeyboardKey.fuchsiaPlane;

  static const kEnter = LogicalKeyboardKey(40 | kFuchsiaKeyIdPlane);
  static const kBackspace = LogicalKeyboardKey(42 | kFuchsiaKeyIdPlane);
  static const kDelete = LogicalKeyboardKey(76 | kFuchsiaKeyIdPlane);
  static const kEscape = LogicalKeyboardKey(41 | kFuchsiaKeyIdPlane);
  static const kTab = LogicalKeyboardKey(43 | kFuchsiaKeyIdPlane);
  static const kArrowLeft = LogicalKeyboardKey(80 | kFuchsiaKeyIdPlane);
  static const kArrowRight = LogicalKeyboardKey(79 | kFuchsiaKeyIdPlane);
  static const kArrowDown = LogicalKeyboardKey(81 | kFuchsiaKeyIdPlane);
  static const kArrowUp = LogicalKeyboardKey(82 | kFuchsiaKeyIdPlane);
  static const kPageUp = LogicalKeyboardKey(75 | kFuchsiaKeyIdPlane);
  static const kPageDown = LogicalKeyboardKey(78 | kFuchsiaKeyIdPlane);

  static const Map<ShortcutActivator, Intent> defaultShortcuts =
      <ShortcutActivator, Intent>{
    // Activation
    SingleActivator(kEnter): ActivateIntent(),
    SingleActivator(LogicalKeyboardKey.space): ActivateIntent(),

    // Dismissal
    SingleActivator(kEscape): DismissIntent(),

    // Keyboard traversal.
    SingleActivator(kTab): NextFocusIntent(),
    SingleActivator(kTab, shift: true): PreviousFocusIntent(),
    SingleActivator(kArrowLeft):
        DirectionalFocusIntent(TraversalDirection.left),
    SingleActivator(kArrowRight):
        DirectionalFocusIntent(TraversalDirection.right),
    SingleActivator(kArrowDown):
        DirectionalFocusIntent(TraversalDirection.down),
    SingleActivator(kArrowUp): DirectionalFocusIntent(TraversalDirection.up),

    // Scrolling
    SingleActivator(kArrowUp, control: true):
        ScrollIntent(direction: AxisDirection.up),
    SingleActivator(kArrowDown, control: true):
        ScrollIntent(direction: AxisDirection.down),
    SingleActivator(kArrowLeft, control: true):
        ScrollIntent(direction: AxisDirection.left),
    SingleActivator(kArrowRight, control: true):
        ScrollIntent(direction: AxisDirection.right),
    SingleActivator(kPageUp): ScrollIntent(
        direction: AxisDirection.up, type: ScrollIncrementType.page),
    SingleActivator(kPageDown): ScrollIntent(
        direction: AxisDirection.down, type: ScrollIncrementType.page),
  };

  static const Map<ShortcutActivator, Intent> defaultEditingShortcuts =
      <ShortcutActivator, Intent>{
    SingleActivator(kBackspace): DeleteTextIntent(),
    SingleActivator(kBackspace, control: true): DeleteByWordTextIntent(),
    SingleActivator(kBackspace, alt: true): DeleteByLineTextIntent(),
    SingleActivator(kDelete): DeleteForwardTextIntent(),
    SingleActivator(kDelete, control: true): DeleteForwardByWordTextIntent(),
    SingleActivator(kDelete, alt: true): DeleteForwardByLineTextIntent(),
    SingleActivator(kArrowDown, alt: true): MoveSelectionToEndTextIntent(),
    SingleActivator(kArrowLeft, alt: true): MoveSelectionLeftByLineTextIntent(),
    SingleActivator(kArrowRight, alt: true):
        MoveSelectionRightByLineTextIntent(),
    SingleActivator(kArrowUp, alt: true): MoveSelectionToStartTextIntent(),
    SingleActivator(kArrowDown, shift: true, alt: true):
        ExpandSelectionToEndTextIntent(),
    SingleActivator(kArrowLeft, shift: true, alt: true):
        ExpandSelectionLeftByLineTextIntent(),
    SingleActivator(kArrowRight, shift: true, alt: true):
        ExpandSelectionRightByLineTextIntent(),
    SingleActivator(kArrowUp, shift: true, alt: true):
        ExpandSelectionToStartTextIntent(),
    SingleActivator(kArrowDown): MoveSelectionDownTextIntent(),
    SingleActivator(kArrowLeft): MoveSelectionLeftTextIntent(),
    SingleActivator(kArrowRight): MoveSelectionRightTextIntent(),
    SingleActivator(kArrowUp): MoveSelectionUpTextIntent(),
    SingleActivator(kArrowLeft, control: true):
        MoveSelectionLeftByWordTextIntent(),
    SingleActivator(kArrowRight, control: true):
        MoveSelectionRightByWordTextIntent(),
    SingleActivator(kArrowLeft, shift: true, control: true):
        ExtendSelectionLeftByWordTextIntent(),
    SingleActivator(kArrowRight, shift: true, control: true):
        ExtendSelectionRightByWordTextIntent(),
    SingleActivator(kArrowDown, shift: true): ExtendSelectionDownTextIntent(),
    SingleActivator(kArrowLeft, shift: true): ExtendSelectionLeftTextIntent(),
    SingleActivator(kArrowRight, shift: true): ExtendSelectionRightTextIntent(),
    SingleActivator(kArrowUp, shift: true): ExtendSelectionUpTextIntent(),
  };
}
