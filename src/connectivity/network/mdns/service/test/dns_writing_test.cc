// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/connectivity/network/mdns/service/encoding/dns_writing.h"

#include <gtest/gtest.h>

#include "src/connectivity/network/mdns/service/common/type_converters.h"

namespace mdns {
namespace test {

constexpr char kInstanceFullName[] = "testinstance._testservice._tcp.local.";
const std::vector<std::string> kTextStrings{"test string 1", "test string 2", "etc"};

// Tests writing of TXT records (regression test for fxb/102543).
TEST(DnsWritingTest, Regression102543) {
  std::vector<uint8_t> expected_message_as_written{
      0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x74,
      0x65, 0x73, 0x74, 0x69, 0x6e, 0x73, 0x74, 0x61, 0x6e, 0x63, 0x65, 0x0c, 0x5f, 0x74,
      0x65, 0x73, 0x74, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63, 0x65, 0x04, 0x5f, 0x74, 0x63,
      0x70, 0x05, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x00, 0x00, 0x10, 0x80, 0x01, 0x00, 0x00,
      0x11, 0x94, 0x00, 0x20, 0x0d, 0x74, 0x65, 0x73, 0x74, 0x20, 0x73, 0x74, 0x72, 0x69,
      0x6e, 0x67, 0x20, 0x31, 0x0d, 0x74, 0x65, 0x73, 0x74, 0x20, 0x73, 0x74, 0x72, 0x69,
      0x6e, 0x67, 0x20, 0x32, 0x03, 0x65, 0x74, 0x63};

  DnsMessage message;
  auto txt_resource = std::make_shared<DnsResource>(kInstanceFullName, DnsType::kTxt);
  txt_resource->txt_.strings_ = fidl::To<std::vector<std::vector<uint8_t>>>(kTextStrings);
  message.answers_.push_back(std::move(txt_resource));
  message.UpdateCounts();

  PacketWriter writer;
  writer << message;

  auto message_as_written = writer.GetResizedPacket();

  EXPECT_EQ(expected_message_as_written, message_as_written);
}

// Tests writing of TXT records with no text strings (regression test for fxb/102543).
TEST(DnsWritingTest, Regression102543NoStrings) {
  std::vector<uint8_t> expected_message_as_written{
      0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x0c,
      0x74, 0x65, 0x73, 0x74, 0x69, 0x6e, 0x73, 0x74, 0x61, 0x6e, 0x63, 0x65, 0x0c,
      0x5f, 0x74, 0x65, 0x73, 0x74, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63, 0x65, 0x04,
      0x5f, 0x74, 0x63, 0x70, 0x05, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x00, 0x00, 0x10,
      0x80, 0x01, 0x00, 0x00, 0x11, 0x94, 0x00, 0x01, 0x00};

  DnsMessage message;
  auto txt_resource = std::make_shared<DnsResource>(kInstanceFullName, DnsType::kTxt);
  message.answers_.push_back(std::move(txt_resource));
  message.UpdateCounts();

  PacketWriter writer;
  writer << message;

  auto message_as_written = writer.GetResizedPacket();

  EXPECT_EQ(expected_message_as_written, message_as_written);
}

}  // namespace test
}  // namespace mdns
