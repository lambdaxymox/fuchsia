// Copyright 2021 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#include <ctype.h>
#include <inttypes.h>
#include <lib/elfldltl/self.h>
#include <stdint.h>
#include <stdlib.h>
#include <zircon/assert.h>

#include <ktl/array.h>
#include <ktl/iterator.h>
#include <ktl/optional.h>
#include <ktl/string_view.h>
#include <phys/main.h>
#include <phys/stdio.h>
#include <phys/symbolize.h>
#include <pretty/cpp/sizes.h>

#include "test-main.h"
#include "turducken.h"

#include <ktl/enforce.h>

// Declared in turducken.h.
const char* kTestName = "trampoline-boot-test";

namespace {

constexpr ktl::string_view kKernelLoadAddressOpt = "trampoline.kernel_load_address=";
constexpr ktl::string_view kDataLoadAddressOpt = "trampoline.data_load_address=";

}  // namespace

int TurduckenTest::Main(Zbi::iterator kernel_item) {
  auto load_addr_opt = OptionWithPrefix(kKernelLoadAddressOpt);

  ZX_ASSERT(load_addr_opt);
  ZX_ASSERT(load_addr_opt->length() <= 19);
  // 0x[16] + NUL character.
  ktl::array<char, 19> hex_str = {};
  memcpy(hex_str.data(), load_addr_opt->data(), load_addr_opt->length());
  uint64_t expected_load_address = strtoul(hex_str.data(), nullptr, 16);
  auto actual_load_address = reinterpret_cast<uint64_t>(PHYS_LOAD_ADDRESS);
  ZX_ASSERT_MSG(actual_load_address == expected_load_address,
                "actual_load_address (0x%016" PRIx64 ") != expected_load_address (0x%016" PRIx64
                ")",
                actual_load_address, expected_load_address);

  load_addr_opt = OptionWithPrefix(kDataLoadAddressOpt);
  ZX_ASSERT(load_addr_opt);
  ZX_ASSERT(load_addr_opt->length() <= 18);
  memcpy(hex_str.data(), load_addr_opt->data(), load_addr_opt->length());
  expected_load_address = strtoul(hex_str.data(), nullptr, 16);
  actual_load_address = reinterpret_cast<uint64_t>(boot_zbi().storage().data());
  ZX_ASSERT_MSG(actual_load_address == expected_load_address,
                "actual_load_address (0x%016" PRIx64 ") != expected_load_address (0x%016" PRIx64
                ")",
                actual_load_address, expected_load_address);
  return 0;
}
