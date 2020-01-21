// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "blob-compressor.h"

#include <zircon/types.h>

#include <memory>

#include <fbl/algorithm.h>
#include <fbl/auto_call.h>
#include <fbl/macros.h>
#include <fs/trace.h>

#include "lz4.h"
#include "zstd-plain.h"
#include "zstd-rac.h"

namespace blobfs {

std::optional<BlobCompressor> BlobCompressor::Create(CompressionAlgorithm algorithm,
                                                     size_t blob_size) {
  switch (algorithm) {
    case CompressionAlgorithm::LZ4: {
      fzl::OwnedVmoMapper compressed_blob;
      size_t max = LZ4Compressor::BufferMax(blob_size);
      zx_status_t status = compressed_blob.CreateAndMap(max, "lz4-blob");
      if (status != ZX_OK) {
        return std::nullopt;
      }
      std::unique_ptr<LZ4Compressor> compressor;
      status = LZ4Compressor::Create(blob_size, compressed_blob.start(), compressed_blob.size(),
                                     &compressor);
      if (status != ZX_OK) {
        return std::nullopt;
      }
      auto result = BlobCompressor(std::move(compressor), std::move(compressed_blob));
      return std::make_optional(std::move(result));
    }
    case CompressionAlgorithm::ZSTD: {
      fzl::OwnedVmoMapper compressed_blob;
      size_t max = ZSTDCompressor::BufferMax(blob_size);
      zx_status_t status = compressed_blob.CreateAndMap(max, "zstd-blob");
      if (status != ZX_OK) {
        return std::nullopt;
      }
      std::unique_ptr<ZSTDCompressor> compressor;
      status = ZSTDCompressor::Create(blob_size, compressed_blob.start(), compressed_blob.size(),
                                      &compressor);
      if (status != ZX_OK) {
        return std::nullopt;
      }
      auto result = BlobCompressor(std::move(compressor), std::move(compressed_blob));
      return std::make_optional(std::move(result));
    }
    case CompressionAlgorithm::ZSTD_SEEKABLE: {
      fzl::OwnedVmoMapper compressed_blob;
      size_t max = ZSTDSeekableCompressor::BufferMax(blob_size);
      zx_status_t status = compressed_blob.CreateAndMap(max, "zstd-seekable-blob");
      if (status != ZX_OK) {
        return std::nullopt;
      }
      std::unique_ptr<ZSTDSeekableCompressor> compressor;
      status = ZSTDSeekableCompressor::Create(blob_size, compressed_blob.start(),
                                              compressed_blob.size(), &compressor);
      if (status != ZX_OK) {
        return std::nullopt;
      }
      auto result = BlobCompressor(std::move(compressor), std::move(compressed_blob));
      return std::make_optional(std::move(result));
    }
    default:
      return std::nullopt;
  }
}

BlobCompressor::BlobCompressor(std::unique_ptr<Compressor> compressor,
                               fzl::OwnedVmoMapper compressed_blob)
    : compressor_(std::move(compressor)), compressed_blob_(std::move(compressed_blob)) {}

BlobCompressor::~BlobCompressor() = default;

}  // namespace blobfs
