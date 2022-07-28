// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_FIRMWARE_GIGABOOT_CPP_GPT_H_
#define SRC_FIRMWARE_GIGABOOT_CPP_GPT_H_

#include <lib/fitx/result.h>
#include <zircon/hw/gpt.h>

#include <optional>
#include <string_view>

#include <fbl/vector.h>

#include "utils.h"

namespace gigaboot {

class EfiGptBlockDevice {
 public:
  static fitx::result<efi_status, EfiGptBlockDevice> Create(efi_handle device_handle);

  // No copy.
  EfiGptBlockDevice(const EfiGptBlockDevice &) = delete;
  EfiGptBlockDevice &operator=(const EfiGptBlockDevice &) = delete;

  EfiGptBlockDevice(EfiGptBlockDevice &&) = default;
  EfiGptBlockDevice &operator=(EfiGptBlockDevice &&) = default;

  fitx::result<efi_status> ReadPartition(std::string_view name, size_t offset, size_t length,
                                         void *out);
  fitx::result<efi_status> WritePartition(std::string_view name, const void *data, size_t offset,
                                          size_t length);

  // Find partition info.
  const gpt_entry_t *FindPartition(std::string_view name);

  // Load GPT from device.
  fitx::result<efi_status> Load();

  // TODO(b/238334864): Add support for initializing/updating GPT.

 private:
  // The parameters we need for reading/writing partitions live in both block and disk io protocols.
  EfiProtocolPtr<efi_block_io_protocol> block_io_protocol_;
  EfiProtocolPtr<efi_disk_io_protocol> disk_io_protocol_;

  gpt_header_t gpt_header_;

  struct GptEntryInfo {
    gpt_entry_t entry;
    char utf8_name[GPT_NAME_LEN / 2];
  };

  fbl::Vector<GptEntryInfo> entries_;

  EfiGptBlockDevice() {}
  size_t BlockSize() { return block_io_protocol_->Media->BlockSize; }
  efi_status Read(void *buffer, size_t offset, size_t length);
  efi_status Write(const void *data, size_t offset, size_t length);
  const fbl::Vector<GptEntryInfo> &GetGptEntries() { return entries_; }

  // Check that the given range is within boundary of a partition and returns the absolute offset
  // relative to the storage start.
  fitx::result<efi_status, size_t> CheckAndGetPartitionAccessRangeInStorage(std::string_view name,
                                                                            size_t offset,
                                                                            size_t length);
};

fitx::result<efi_status, EfiGptBlockDevice> FindEfiGptDevice();

}  // namespace gigaboot

#endif  // SRC_FIRMWARE_GIGABOOT_CPP_GPT_H_
