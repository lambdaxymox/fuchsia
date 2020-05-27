// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_DEVELOPER_FEEDBACK_CRASH_REPORTS_INFO_CRASH_REGISTER_INFO_H_
#define SRC_DEVELOPER_FEEDBACK_CRASH_REPORTS_INFO_CRASH_REGISTER_INFO_H_

#include <memory>
#include <string>

#include "src/developer/feedback/crash_reports/info/info_context.h"
#include "src/developer/feedback/crash_reports/product.h"

namespace feedback {

// Information about the crash register we want to export.
struct CrashRegisterInfo {
 public:
  CrashRegisterInfo(std::shared_ptr<InfoContext> context);

  void UpsertComponentToProductMapping(const std::string& component_url, const Product& product);

 private:
  std::shared_ptr<InfoContext> context_;
};

}  // namespace feedback

#endif  // SRC_DEVELOPER_FEEDBACK_CRASH_REPORTS_INFO_CRASH_REGISTER_INFO_H_
