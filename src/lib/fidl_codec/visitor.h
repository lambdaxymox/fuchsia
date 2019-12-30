// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_LIB_FIDL_CODEC_VISITOR_H_
#define SRC_LIB_FIDL_CODEC_VISITOR_H_

#include "src/lib/fidl_codec/wire_object.h"

namespace fidl_codec {

// Superclass for implementing visitors for Values. Note that the whole class is protected. To use a
// visitor, use the Visit method on the Value object you want to visit.
class Visitor {
 protected:
  virtual void VisitValue(const Value* node) {}
  virtual void VisitInvalidValue(const InvalidValue* node) { VisitValue(node); }
  virtual void VisitNullValue(const NullValue* node) { VisitValue(node); }
  virtual void VisitNullableValue(const NullableValue* node) { VisitValue(node); }
  virtual void VisitRawValue(const RawValue* node) { VisitValue(node); }
  virtual void VisitStructValue(const StructValue* node) { VisitValue(node); }
  virtual void VisitStringValue(const StringValue* node) { VisitValue(node); }
  virtual void VisitBoolValue(const BoolValue* node) { VisitValue(node); }
  virtual void VisitTableValue(const TableValue* node) { VisitValue(node); }
  virtual void VisitUnionValue(const UnionValue* node) { VisitValue(node); }
  virtual void VisitArrayValue(const ArrayValue* node) { VisitValue(node); }
  virtual void VisitVectorValue(const VectorValue* node) { VisitNullableValue(node); }
  virtual void VisitEnumValue(const EnumValue* node) { VisitValue(node); }
  virtual void VisitBitsValue(const BitsValue* node) { VisitValue(node); }
  virtual void VisitHandleValue(const HandleValue* node) { VisitValue(node); }

  virtual void VisitU8Value(const NumericValue<uint8_t>* node) { VisitValue(node); }
  virtual void VisitU16Value(const NumericValue<uint16_t>* node) { VisitValue(node); }
  virtual void VisitU32Value(const NumericValue<uint32_t>* node) { VisitValue(node); }
  virtual void VisitU64Value(const NumericValue<uint64_t>* node) { VisitValue(node); }
  virtual void VisitI8Value(const NumericValue<int8_t>* node) { VisitValue(node); }
  virtual void VisitI16Value(const NumericValue<int16_t>* node) { VisitValue(node); }
  virtual void VisitI32Value(const NumericValue<int32_t>* node) { VisitValue(node); }
  virtual void VisitI64Value(const NumericValue<int64_t>* node) { VisitValue(node); }
  virtual void VisitF32Value(const NumericValue<float>* node) { VisitValue(node); }
  virtual void VisitF64Value(const NumericValue<double>* node) { VisitValue(node); }

  friend class Value;
  friend class InvalidValue;
  friend class NullValue;
  friend class NullableValue;
  friend class RawValue;
  template <typename T>
  friend class NumericValue;
  friend class StringValue;
  friend class BoolValue;
  friend class StructValue;
  friend class TableValue;
  friend class UnionValue;
  friend class ArrayValue;
  friend class VectorValue;
  friend class EnumValue;
  friend class BitsValue;
  friend class HandleValue;
};

}  // namespace fidl_codec

#endif  // SRC_LIB_FIDL_CODEC_VISITOR_H_
