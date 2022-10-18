// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <zxtest/zxtest.h>

#include "tools/fidl/fidlc/include/fidl/diagnostics.h"
#include "tools/fidl/fidlc/tests/error_test.h"
#include "tools/fidl/fidlc/tests/test_library.h"

// This file is meant to hold standalone tests for each of the "good" examples used in the documents
// at //docs/reference/fidl/language/error-catalog. These cases are redundant with the other tests
// in this suite - their purpose is not to serve as tests for the features at hand, but rather to
// provide well-vetted and tested examples of the "correct" way to fix FIDL errors.

namespace {

TEST(ErrcatTests, Good0003) {
  TestLibrary library;
  library.AddFile("good/fi-0003.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0007) {
  TestLibrary library;
  library.AddFile("good/fi-0007.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0010a) {
  TestLibrary library;
  library.AddFile("good/fi-0010-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0011) {
  TestLibrary library;
  library.AddFile("good/fi-0011.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0012) {
  TestLibrary library;
  library.AddFile("good/fi-0012.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0013) {
  TestLibrary library;
  library.AddFile("good/fi-0013.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0014) {
  TestLibrary library;
  library.AddFile("good/fi-0014.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0015) {
  TestLibrary library;
  library.AddFile("good/fi-0015.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0016) {
  TestLibrary library;
  library.AddFile("good/fi-0016.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0017) {
  TestLibrary library;
  library.AddFile("good/fi-0017.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0020) {
  TestLibrary library;
  library.AddFile("good/fi-0020.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0022) {
  TestLibrary library;
  library.AddFile("good/fi-0022.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0023) {
  TestLibrary library;
  library.AddFile("good/fi-0023.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0025) {
  SharedAmongstLibraries shared;
  TestLibrary dependency(&shared, "dependent.fidl", R"FIDL(library dependent;

type Something = struct {};
)FIDL");
  ASSERT_COMPILED(dependency);
  TestLibrary library(&shared);
  library.AddFile("good/fi-0025.test.fidl");
}

TEST(ErrcatTests, Good0028a) {
  TestLibrary library;
  library.AddFile("good/fi-0028-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0030) {
  TestLibrary library;
  library.AddFile("good/fi-0030.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0031) {
  TestLibrary library;
  library.AddFile("good/fi-0031.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0032) {
  TestLibrary library;
  library.AddFile("good/fi-0032.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0038ab) {
  SharedAmongstLibraries shared;
  TestLibrary dependency(&shared);
  dependency.AddFile("good/fi-0038-a.test.fidl");
  ASSERT_COMPILED(dependency);
  TestLibrary library(&shared);
  library.AddFile("good/fi-0038-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0038ac) {
  SharedAmongstLibraries shared;
  TestLibrary dependency(&shared);
  dependency.AddFile("good/fi-0038-a.test.fidl");
  ASSERT_COMPILED(dependency);
  TestLibrary library(&shared);
  library.AddFile("good/fi-0038-c.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0039ab) {
  SharedAmongstLibraries shared;
  TestLibrary dependency(&shared);
  dependency.AddFile("good/fi-0039-a.test.fidl");
  ASSERT_COMPILED(dependency);
  TestLibrary library(&shared);
  library.AddFile("good/fi-0039-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0039ac) {
  SharedAmongstLibraries shared;
  TestLibrary dependency(&shared);
  dependency.AddFile("good/fi-0039-a.test.fidl");
  ASSERT_COMPILED(dependency);
  TestLibrary library(&shared);
  library.AddFile("good/fi-0039-c.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0041a) {
  TestLibrary library;
  library.AddFile("good/fi-0041-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0041b) {
  TestLibrary library;
  library.AddFile("good/fi-0041-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0042) {
  SharedAmongstLibraries shared;
  TestLibrary dependency(&shared);
  dependency.AddFile("good/fi-0042-a.test.fidl");
  ASSERT_COMPILED(dependency);
  TestLibrary library(&shared);
  library.AddFile("good/fi-0042-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0043) {
  SharedAmongstLibraries shared;
  TestLibrary dependency1(&shared);
  dependency1.AddFile("good/fi-0043-a.test.fidl");
  ASSERT_COMPILED(dependency1);
  TestLibrary dependency2(&shared);
  dependency2.AddFile("good/fi-0043-b.test.fidl");
  ASSERT_COMPILED(dependency2);
  TestLibrary library(&shared);
  library.AddFile("good/fi-0043-c.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0044) {
  SharedAmongstLibraries shared;
  TestLibrary dependency1(&shared);
  dependency1.AddFile("good/fi-0044-a.test.fidl");
  ASSERT_COMPILED(dependency1);
  TestLibrary dependency2(&shared);
  dependency2.AddFile("good/fi-0044-b.test.fidl");
  ASSERT_COMPILED(dependency2);
  TestLibrary library(&shared);
  library.AddFile("good/fi-0044-c.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0045) {
  SharedAmongstLibraries shared;
  TestLibrary dependency(&shared);
  dependency.AddFile("good/fi-0045-a.test.fidl");
  ASSERT_COMPILED(dependency);
  TestLibrary library(&shared);
  library.AddFile("good/fi-0045-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0046) {
  TestLibrary library;
  library.AddFile("good/fi-0046.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0047) {
  TestLibrary library;
  library.AddFile("good/fi-0047.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0048) {
  TestLibrary library;
  library.AddFile("good/fi-0048.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0049) {
  TestLibrary library;
  library.AddFile("good/fi-0049.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0050) {
  TestLibrary library;
  library.AddFile("good/fi-0050.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0058) {
  TestLibrary library;
  library.AddFile("good/fi-0058.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0059) {
  TestLibrary library;
  library.AddFile("good/fi-0059.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0065a) {
  TestLibrary library;
  library.AddFile("good/fi-0065-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0065b) {
  TestLibrary library;
  library.AddFile("good/fi-0065-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0065c) {
  TestLibrary library;
  library.AddFile("good/fi-0065-c.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0068A) {
  TestLibrary library;
  library.AddFile("good/fi-0068-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0068B) {
  TestLibrary library;
  library.AddFile("good/fi-0068-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0069) {
  TestLibrary library;
  library.AddFile("good/fi-0069.test.fidl");
}

TEST(ErrcatTests, Good0070) {
  TestLibrary library;
  library.AddFile("good/fi-0070.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0071a) {
  TestLibrary library;
  library.AddFile("good/fi-0071-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0071b) {
  TestLibrary library;
  library.AddFile("good/fi-0071-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0072a) {
  TestLibrary library;
  library.AddFile("good/fi-0072-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0072b) {
  TestLibrary library;
  library.AddFile("good/fi-0072-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0073) {
  TestLibrary library;
  library.AddFile("good/fi-0073.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0074) {
  TestLibrary library;
  library.AddFile("good/fi-0074.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0075) {
  TestLibrary library;
  library.AddFile("good/fi-0075.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0084) {
  TestLibrary library;
  library.AddFile("good/fi-0084.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0100a) {
  TestLibrary library;
  library.AddFile("good/fi-0100-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0100b) {
  TestLibrary library;
  library.AddFile("good/fi-0100-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0110a) {
  TestLibrary library;
  library.AddFile("good/fi-0110-a.test.fidl");
  library.UseLibraryZx();
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0110b) {
  TestLibrary library;
  library.AddFile("good/fi-0110-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0111) {
  TestLibrary library;
  library.AddFile("good/fi-0111.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0112) {
  TestLibrary library;
  library.AddFile("good/fi-0112.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0113) {
  TestLibrary library;
  library.AddFile("good/fi-0113.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0114a) {
  TestLibrary library;
  library.AddFile("good/fi-0114-a.test.fidl");
  library.EnableFlag(fidl::ExperimentalFlags::Flag::kUnknownInteractions);
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0114b) {
  TestLibrary library;
  library.AddFile("good/fi-0114-b.test.fidl");
  library.EnableFlag(fidl::ExperimentalFlags::Flag::kUnknownInteractions);
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0115a) {
  TestLibrary library;
  library.AddFile("good/fi-0115-a.test.fidl");
  library.EnableFlag(fidl::ExperimentalFlags::Flag::kUnknownInteractions);
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0115b) {
  TestLibrary library;
  library.AddFile("good/fi-0115-b.test.fidl");
  library.EnableFlag(fidl::ExperimentalFlags::Flag::kUnknownInteractions);
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0116a) {
  TestLibrary library;
  library.AddFile("good/fi-0116-a.test.fidl");
  library.EnableFlag(fidl::ExperimentalFlags::Flag::kUnknownInteractions);
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0116b) {
  TestLibrary library;
  library.AddFile("good/fi-0116-b.test.fidl");
  library.EnableFlag(fidl::ExperimentalFlags::Flag::kUnknownInteractions);
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0117a) {
  TestLibrary library;
  library.AddFile("good/fi-0117-a.test.fidl");
  library.UseLibraryZx();
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0117b) {
  TestLibrary library;
  library.AddFile("good/fi-0117-b.test.fidl");
  library.UseLibraryFdf();
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0118) {
  TestLibrary library;
  library.AddFile("good/fi-0118.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0119a) {
  TestLibrary library;
  library.AddFile("good/fi-0119-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0119b) {
  TestLibrary library;
  library.AddFile("good/fi-0119-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0120a) {
  TestLibrary library;
  library.AddFile("good/fi-0120-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0120b) {
  TestLibrary library;
  library.AddFile("good/fi-0120-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0121) {
  TestLibrary library;
  library.AddFile("good/fi-0121.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0122) {
  TestLibrary library;
  library.AddFile("good/fi-0122.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0123) {
  TestLibrary library;
  library.AddFile("good/fi-0123.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0124) {
  TestLibrary library;
  library.AddFile("good/fi-0124.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0125) {
  TestLibrary library;
  library.AddFile("good/fi-0125.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0126) {
  TestLibrary library;
  library.AddFile("good/fi-0126.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0127) {
  TestLibrary library;
  library.AddFile("good/fi-0127.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0128) {
  TestLibrary library;
  library.AddFile("good/fi-0128.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0129a) {
  TestLibrary library;
  library.AddFile("good/fi-0129-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0129b) {
  TestLibrary library;
  library.AddFile("good/fi-0129-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0130) {
  TestLibrary library;
  library.AddFile("good/fi-0130.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0131a) {
  TestLibrary library;
  library.AddFile("good/fi-0131-a.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0131b) {
  TestLibrary library;
  library.AddFile("good/fi-0131-b.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0132) {
  TestLibrary library;
  library.AddFile("good/fi-0132.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0133) {
  TestLibrary library;
  library.AddFile("good/fi-0133.test.fidl");
  ASSERT_COMPILED(library);
}

TEST(ErrcatTests, Good0162) {
  TestLibrary library;
  library.AddFile("good/fi-0162.test.fidl");
}

}  // namespace
