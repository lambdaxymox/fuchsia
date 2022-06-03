// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/developer/debug/debug_agent/zircon_system_interface.h"

#include <fuchsia/kernel/cpp/fidl.h>
#include <lib/fdio/directory.h>
#include <lib/fdio/fdio.h>

#include "src/developer/debug/debug_agent/zircon_binary_launcher.h"
#include "src/developer/debug/debug_agent/zircon_job_handle.h"
#include "src/developer/debug/debug_agent/zircon_process_handle.h"
#include "src/developer/debug/debug_agent/zircon_utils.h"
#include "src/lib/files/file.h"

namespace debug_agent {

namespace {

// Returns an !is_valid() job object on failure.
zx::job GetRootZxJob() {
  zx::job root_job;
  fuchsia::kernel::RootJobSyncPtr root_job_ptr;

  std::string root_job_path("/svc/");
  root_job_path.append(fuchsia::kernel::RootJob::Name_);

  zx_status_t status = fdio_service_connect(root_job_path.c_str(),
                                            root_job_ptr.NewRequest().TakeChannel().release());
  if (status != ZX_OK) {
    FX_NOTREACHED();
    return zx::job();
  }

  status = root_job_ptr->Get(&root_job);
  if (status != ZX_OK) {
    FX_NOTREACHED();
    return zx::job();
  }
  return root_job;
}

}  // namespace

ZirconSystemInterface::ZirconSystemInterface()
    : services_(sys::ServiceDirectory::CreateFromNamespace()),
      component_manager_(services_),
      limbo_provider_(services_) {
  if (zx::job zx_root = GetRootZxJob(); zx_root.is_valid())
    root_job_ = std::make_unique<ZirconJobHandle>(std::move(zx_root));
}

uint32_t ZirconSystemInterface::GetNumCpus() const { return zx_system_get_num_cpus(); }

uint64_t ZirconSystemInterface::GetPhysicalMemory() const { return zx_system_get_physmem(); }

std::unique_ptr<JobHandle> ZirconSystemInterface::GetRootJob() const {
  if (root_job_)
    return std::make_unique<ZirconJobHandle>(*root_job_);

  FX_LOGS(WARNING) << "Failed to get the root job";
  return nullptr;
}

std::unique_ptr<BinaryLauncher> ZirconSystemInterface::GetLauncher() const {
  return std::make_unique<ZirconBinaryLauncher>(services_);
}

ComponentManager& ZirconSystemInterface::GetComponentManager() { return component_manager_; }

std::string ZirconSystemInterface::GetSystemVersion() { return zx_system_get_version_string(); }

}  // namespace debug_agent
