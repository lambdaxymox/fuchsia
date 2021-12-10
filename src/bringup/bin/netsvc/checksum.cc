// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <stdint.h>

#include <limits>

#include "src/bringup/bin/netsvc/inet6.h"

static uint16_t checksum(const void* _data, size_t len, uint16_t _sum) {
  uint32_t sum = _sum;
  const uint16_t* data = static_cast<const uint16_t*>(_data);
  while (len > 1) {
    sum += *data++;
    len -= 2;
  }
  if (len) {
    sum += (*data & 0xFF);
  }
  while (sum > std::numeric_limits<uint16_t>::max()) {
    sum = (sum & 0xFFFF) + (sum >> 16);
  }
  return static_cast<uint16_t>(sum);
}

uint16_t ip6_checksum(ip6_hdr_t* ip, unsigned type, size_t length) {
  uint16_t sum;

  // length and protocol field for pseudo-header
  sum = checksum(&ip->length, 2, htons(type));
  // src/dst for pseudo-header + payload
  sum = checksum(&ip->src, 32 + length, sum);

  // 0 is illegal, so 0xffff remains 0xffff
  if (sum != 0xffff) {
    return ~sum;
  }
  return sum;
}
