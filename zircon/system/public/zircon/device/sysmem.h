// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SYSROOT_ZIRCON_DEVICE_SYSMEM_H_
#define SYSROOT_ZIRCON_DEVICE_SYSMEM_H_

#include <fuchsia/sysmem/c/fidl.h>

#include <ddk/metadata.h>

// "SyM"
#define SYSMEM_METADATA (0x53794d00 | DEVICE_METADATA_PRIVATE)

typedef struct {
  uint32_t vid;
  uint32_t pid;
  uint64_t protected_memory_size;
  // Size of the pool used to allocate contiguous memory.
  uint64_t contiguous_memory_size;
} sysmem_metadata_t;

// TODO(fxbug.dev/32526): Deleting this file is blocked by banjo being able to consume
// code generated by fidl.
typedef fuchsia_sysmem_BufferCollectionInfo buffer_collection_info_t;
typedef fuchsia_sysmem_BufferCollectionInfo_2 buffer_collection_info_2_t;
typedef fuchsia_sysmem_ImageFormat image_format_t;
typedef fuchsia_sysmem_ImageFormat_2 image_format_2_t;

#endif  // SYSROOT_ZIRCON_DEVICE_SYSMEM_H_
