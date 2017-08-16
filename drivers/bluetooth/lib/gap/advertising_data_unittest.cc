// Copyright 2017 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "apps/bluetooth/lib/gap/advertising_data.h"

#include "gtest/gtest.h"

#include "apps/bluetooth/lib/common/byte_buffer.h"
#include "apps/bluetooth/lib/common/test_helpers.h"

namespace bluetooth {
namespace gap {
namespace {

constexpr uint16_t kId1As16 = 0x0212;
constexpr char kId1AsString[] = "00000212-0000-1000-8000-00805f9b34fb";
constexpr uint16_t kId2As16 = 0x1122;

constexpr char kId3AsString[] = "12341234-0000-1000-8000-00805f9b34fb";

TEST(AdvertisingDataTest, ReaderEmptyData) {
  common::BufferView empty;
  AdvertisingDataReader reader(empty);
  EXPECT_FALSE(reader.is_valid());
  EXPECT_FALSE(reader.HasMoreData());
}

TEST(AdvertisingDataTest, MakeEmpty) {
  AdvertisingData data;

  EXPECT_EQ(0u, data.block_size());
}

TEST(AdvertisingDataTest, EncodeKnownURI) {
  AdvertisingData data;
  data.AddURI("https://abc.xyz");

  auto bytes =
      common::CreateStaticByteBuffer(0x0B, 0x24, 0x17, '/', '/', 'a', 'b', 'c', '.', 'x', 'y', 'z');

  EXPECT_EQ(bytes.size(), data.block_size());
  common::DynamicByteBuffer block(data.block_size());
  data.WriteBlock(&block);
  EXPECT_TRUE(ContainersEqual(bytes, block));
}

TEST(AdvertisingDataTest, EncodeUnknownURI) {
  AdvertisingData data;
  data.AddURI("flubs:xyz");

  auto bytes =
      common::CreateStaticByteBuffer(0x0B, 0x24, 0x01, 'f', 'l', 'u', 'b', 's', ':', 'x', 'y', 'z');

  EXPECT_EQ(bytes.size(), data.block_size());
  common::DynamicByteBuffer block(data.block_size());
  data.WriteBlock(&block);
  EXPECT_TRUE(ContainersEqual(bytes, block));
}

TEST(AdvertisingDataTest, CompressServiceUUIDs) {
  AdvertisingData data;
  data.AddServiceUuid(common::UUID(kId1As16));
  data.AddServiceUuid(common::UUID(kId2As16));

  EXPECT_EQ(1 + 1 + (sizeof(uint16_t) * 2), data.block_size());

  auto bytes = common::CreateStaticByteBuffer(0x05, 0x02, 0x12, 0x02, 0x22, 0x11);

  EXPECT_EQ(bytes.size(), data.block_size());
  common::DynamicByteBuffer block(data.block_size());
  data.WriteBlock(&block);

  EXPECT_TRUE(ContainersEqual(bytes, block));
}

TEST(AdvertisingDataTest, ParseBlock) {
  auto bytes = common::CreateStaticByteBuffer(
      // Complete 16-bit UUIDs
      0x05, 0x03, 0x12, 0x02, 0x22, 0x11,
      // Incomplete list of 32-bit UUIDs
      0x05, 0x04, 0x34, 0x12, 0x34, 0x12,
      // Local name
      0x09, 0x09, 'T', 'e', 's', 't', 0xF0, 0x9F, 0x92, 0x96,
      // TX Power
      0x02, 0x0A, 0x8F);

  AdvertisingData data;

  EXPECT_TRUE(AdvertisingData::FromBytes(bytes, &data));

  EXPECT_EQ(3u, data.service_uuids().size());
  EXPECT_TRUE(data.local_name());
  EXPECT_EQ("Test💖", *(data.local_name()));
  EXPECT_TRUE(data.tx_power());
  EXPECT_EQ(-113, *(data.tx_power()));
}

TEST(AdvertisingDataTest, ParseFIDL) {
  auto fidl_ad = ::btfidl::low_energy::AdvertisingData::New();

  // Confirming UTF-8 codepoints are working as well.
  fidl_ad->name = "Test💖";
  fidl_ad->service_uuids.push_back(kId1AsString);
  fidl_ad->service_uuids.push_back(kId3AsString);

  AdvertisingData data;

  AdvertisingData::FromFidl(fidl_ad, &data);

  EXPECT_EQ(2u, data.service_uuids().size());
  EXPECT_EQ("Test💖", *(data.local_name()));
  EXPECT_FALSE(data.tx_power());
}

TEST(AdvertisingDataTest, ManufacturerZeroLength) {
  auto bytes = common::CreateStaticByteBuffer(
      // Complete 16-bit UUIDs
      0x05, 0x03, 0x12, 0x02, 0x22, 0x11,
      // Manufacturer Data with no data
      0x03, 0xFF, 0x34, 0x12);

  AdvertisingData data;

  EXPECT_EQ(0u, data.manufacturer_data_ids().size());

  EXPECT_TRUE(AdvertisingData::FromBytes(bytes, &data));

  EXPECT_EQ(1u, data.manufacturer_data_ids().count(0x1234));
  EXPECT_EQ(0u, data.manufacturer_data(0x1234).size());
}

TEST(AdvertisingDataTest, ReaderMalformedData) {
  // TLV length exceeds the size of the payload
  auto bytes0 = common::CreateStaticByteBuffer(0x01);
  AdvertisingDataReader reader(bytes0);
  EXPECT_FALSE(reader.is_valid());
  EXPECT_FALSE(reader.HasMoreData());

  auto bytes = common::CreateStaticByteBuffer(0x05, 0x00, 0x00, 0x00, 0x00);
  reader = AdvertisingDataReader(bytes);
  EXPECT_FALSE(reader.is_valid());
  EXPECT_FALSE(reader.HasMoreData());

  // TLV length is 0. This is not considered malformed. Data should be valid but
  // should not return more data.
  bytes = common::CreateStaticByteBuffer(0x00, 0x00, 0x00, 0x00, 0x00);
  reader = AdvertisingDataReader(bytes);
  EXPECT_TRUE(reader.is_valid());
  EXPECT_FALSE(reader.HasMoreData());

  // First field is valid, second field is not.
  DataType type;
  common::BufferView data;
  bytes = common::CreateStaticByteBuffer(0x02, 0x00, 0x00, 0x02, 0x00);
  reader = AdvertisingDataReader(bytes);
  EXPECT_FALSE(reader.is_valid());
  EXPECT_FALSE(reader.HasMoreData());
  EXPECT_FALSE(reader.GetNextField(&type, &data));

  // First field is valid, second field has length 0.
  bytes = common::CreateStaticByteBuffer(0x02, 0x00, 0x00, 0x00, 0x00);
  reader = AdvertisingDataReader(bytes);
  EXPECT_TRUE(reader.is_valid());
  EXPECT_TRUE(reader.HasMoreData());
  EXPECT_TRUE(reader.GetNextField(&type, &data));
  EXPECT_FALSE(reader.HasMoreData());
  EXPECT_FALSE(reader.GetNextField(&type, &data));
}

TEST(AdvertisingDataTest, ReaderParseFields) {
  auto bytes = common::CreateStaticByteBuffer(
      0x02, 0x01, 0x00,
      0x05, 0x09, 'T', 'e', 's', 't');
  AdvertisingDataReader reader(bytes);
  EXPECT_TRUE(reader.is_valid());
  EXPECT_TRUE(reader.HasMoreData());

  DataType type;
  common::BufferView data;
  EXPECT_TRUE(reader.GetNextField(&type, &data));
  EXPECT_EQ(DataType::kFlags, type);
  EXPECT_EQ(1u, data.size());
  EXPECT_TRUE(common::ContainersEqual(common::CreateStaticByteBuffer(0x00), data));

  EXPECT_TRUE(reader.HasMoreData());
  EXPECT_TRUE(reader.GetNextField(&type, &data));
  EXPECT_EQ(DataType::kCompleteLocalName, type);
  EXPECT_EQ(4u, data.size());
  EXPECT_TRUE(common::ContainersEqual(std::string("Test"), data));

  EXPECT_FALSE(reader.HasMoreData());
  EXPECT_FALSE(reader.GetNextField(&type, &data));
}

// Helper for computing the size of a string literal at compile time. sizeof() would have worked
// too but that counts the null character.
template <std::size_t N>
constexpr size_t StringSize(char const (&str)[N]) {
  return N - 1;
}

TEST(AdvertisingDataTest, WriteField) {
  constexpr char kValue0[] = "value zero";
  constexpr char kValue1[] = "value one";
  constexpr char kValue2[] = "value two";
  constexpr char kValue3[] = "value three";

  // Have just enough space for the first three values (+ 6 for 2 extra octets for each TLV field).
  constexpr char kBufferSize = StringSize(kValue0) + StringSize(kValue1) + StringSize(kValue2) + 6;
  common::StaticByteBuffer<kBufferSize> buffer;

  AdvertisingDataWriter writer(&buffer);
  EXPECT_EQ(0u, writer.bytes_written());

  // We write malformed values here for testing purposes.
  EXPECT_TRUE(writer.WriteField(DataType::kFlags, common::BufferView(kValue0)));
  EXPECT_EQ(StringSize(kValue0) + 2, writer.bytes_written());

  EXPECT_TRUE(writer.WriteField(DataType::kShortenedLocalName, common::BufferView(kValue1)));
  EXPECT_EQ(StringSize(kValue0) + 2 + StringSize(kValue1) + 2, writer.bytes_written());

  // Trying to write kValue3 should fail because there isn't enough room left in the buffer.
  EXPECT_FALSE(writer.WriteField(DataType::kCompleteLocalName, common::BufferView(kValue3)));

  // Writing kValue2 should fill up the buffer.
  EXPECT_TRUE(writer.WriteField(DataType::kCompleteLocalName, common::BufferView(kValue2)));
  EXPECT_FALSE(writer.WriteField(DataType::kCompleteLocalName, common::BufferView(kValue3)));
  EXPECT_EQ(buffer.size(), writer.bytes_written());

  // Verify the contents.
  DataType type;
  common::BufferView value;
  AdvertisingDataReader reader(buffer);
  EXPECT_TRUE(reader.is_valid());

  EXPECT_TRUE(reader.GetNextField(&type, &value));
  EXPECT_EQ(DataType::kFlags, type);
  EXPECT_EQ(kValue0, value.AsString());

  EXPECT_TRUE(reader.GetNextField(&type, &value));
  EXPECT_EQ(DataType::kShortenedLocalName, type);
  EXPECT_EQ(kValue1, value.AsString());

  EXPECT_TRUE(reader.GetNextField(&type, &value));
  EXPECT_EQ(DataType::kCompleteLocalName, type);
  EXPECT_EQ(kValue2, value.AsString());

  EXPECT_FALSE(reader.GetNextField(&type, &value));
}

}  // namespace
}  // namespace gap
}  // namespace bluetooth
