// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

import 'package:fidl_fidl_test_unionmemberremove/fidl_async.dart' as fidllib;

// [START contents]
fidllib.JsonValue writer(String s) {
  final asFloat = double.tryParse(s);
  if (asFloat != null) {
    return fidllib.JsonValue.withFloatValue(asFloat);
  }
  final asInt = int.tryParse(s);
  if (asInt != null) {
    return fidllib.JsonValue.withIntValue(asInt);
  }
  return fidllib.JsonValue.withStringValue(s);
}

String reader(fidllib.JsonValue value) {
  switch (value.$tag) {
    case fidllib.JsonValueTag.intValue:
      return '${value.intValue}';
    case fidllib.JsonValueTag.stringValue:
      return value.stringValue;
    case fidllib.JsonValueTag.floatValue:
      return '${value.floatValue}';
    default:
      return '<${value.$unknownData.data.length} unknown bytes>';
  }
}

// [END contents]
