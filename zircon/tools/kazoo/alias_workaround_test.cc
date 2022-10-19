// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "tools/kazoo/output_util.h"
#include "tools/kazoo/syscall_library.h"
#include "tools/kazoo/test.h"
#include "tools/kazoo/test_ir_test_aliases.test.h"

namespace {

TEST(AliasWorkaround, Mappings) {
  SyscallLibrary library;
  ASSERT_TRUE(SyscallLibraryLoader::FromJson(k_test_aliases, &library));

  EXPECT_EQ(library.name(), "zx");
  ASSERT_EQ(library.syscalls().size(), 1u);

  const auto& sc = library.syscalls()[0];
  EXPECT_EQ(sc->snake_name(), "aliases_some_func");
  EXPECT_EQ(GetCUserModeName(sc->kernel_return_type()), "zx_status_t");

  // See test_aliases.test.fidl for this giant function's fidl spec. This covers all the aliases
  // required to map all syscalls today. We should be able to whittle these down over time and
  // eventually delete this mapping and test entirely.
  size_t cur_arg = 0;

#define CHECK_ARG(_type, _name)                                               \
  EXPECT_EQ(sc->kernel_arguments()[cur_arg].name(), _name);                   \
  EXPECT_EQ(GetCUserModeName(sc->kernel_arguments()[cur_arg].type()), _type); \
  ++cur_arg;

  // ConstFutexPtr
  CHECK_ARG("const zx_futex_t*", "b");

  // VectorPaddr
  CHECK_ARG("const zx_paddr_t*", "n");
  CHECK_ARG("size_t", "num_n");

#undef CHECK_ARG

  EXPECT_EQ(cur_arg, 3u);
}

}  // namespace
