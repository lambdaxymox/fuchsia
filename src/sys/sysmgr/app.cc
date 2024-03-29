// Copyright 2016 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include "src/sys/sysmgr/app.h"

#include <fuchsia/io/cpp/fidl.h>
#include <fuchsia/sys/cpp/fidl.h>
#include <lib/async-loop/cpp/loop.h>
#include <lib/async/default.h>
#include <lib/async/dispatcher.h>
#include <lib/fdio/directory.h>
#include <lib/fdio/fd.h>
#include <lib/fdio/fdio.h>
#include <lib/fidl/cpp/clone.h>
#include <lib/fidl/cpp/interface_handle.h>
#include <lib/sys/cpp/service_directory.h>
#include <lib/syslog/cpp/macros.h>
#include <lib/vfs/cpp/service.h>
#include <zircon/process.h>
#include <zircon/processargs.h>

namespace sysmgr {
namespace {
constexpr char kDefaultLabel[] = "sys";
}  // namespace

App::App(bool auto_update_packages, Config config,
         std::shared_ptr<sys::ServiceDirectory> incoming_services, async::Loop* loop)
    : loop_(loop),
      incoming_services_(std::move(incoming_services)),
      auto_updates_enabled_(auto_update_packages) {
  // Register services.
  for (auto& pair : config.TakeServices()) {
    RegisterSingleton(pair.first, std::move(pair.second));
  }

  auto env_request = env_.NewRequest();
  fuchsia::sys::ServiceProviderPtr env_services;
  env_->GetLauncher(env_launcher_.NewRequest());
  env_->GetServices(env_services.NewRequest());
  fidl::InterfaceRequest<fuchsia::io::Directory> directory;
  env_services_ = sys::ServiceDirectory::CreateWithRequest(&directory);
  env_->GetDirectory(std::move(directory));

  // Configure loader.
  if (auto_updates_enabled_) {
    package_updating_loader_ = std::make_unique<PackageUpdatingLoader>(
        std::move(env_services), async_get_default_dispatcher());
  }
  static const char* const kLoaderName = fuchsia::sys::Loader::Name_;
  auto child =
      std::make_unique<vfs::Service>([this](zx::channel channel, async_dispatcher_t* dispatcher) {
        if (auto_updates_enabled_) {
          package_updating_loader_->Bind(
              fidl::InterfaceRequest<fuchsia::sys::Loader>(std::move(channel)));
        } else {
          incoming_services_->Connect(kLoaderName, std::move(channel));
        }
      });
  svc_names_.push_back(kLoaderName);
  svc_root_.AddEntry(kLoaderName, std::move(child));

  // Set up environment for the programs we will run.
  fuchsia::sys::ServiceListPtr service_list(new fuchsia::sys::ServiceList);
  service_list->names = std::move(svc_names_);
  service_list->host_directory = OpenAsDirectory();
  fuchsia::sys::EnvironmentPtr environment;
  incoming_services_->Connect(environment.NewRequest());
  // Inherit services from the root appmgr realm, which includes certain
  // services currently implemented by non-component processes that are passed
  // through appmgr to this sys realm. Note that |service_list| will override
  // the inherited services if it includes services also in the root realm.
  fuchsia::sys::EnvironmentOptions options = {.inherit_parent_services = true};
  environment->CreateNestedEnvironment(std::move(env_request), env_controller_.NewRequest(),
                                       kDefaultLabel, std::move(service_list), std::move(options));

  // Connect to startup services
  for (auto& startup_service : config.TakeStartupServices()) {
    FX_VLOGS(1) << "Connecting to startup service " << startup_service;
    zx::channel h1, h2;
    zx::channel::create(0, &h1, &h2);
    ConnectToService(startup_service, std::move(h1));
  }

  // Launch startup applications.
  for (auto& launch_info : config.TakeApps()) {
    LaunchComponent(std::move(*launch_info), nullptr, nullptr);
  }
}

App::~App() = default;

fidl::InterfaceHandle<fuchsia::io::Directory> App::OpenAsDirectory() {
  fidl::InterfaceHandle<fuchsia::io::Directory> dir;
  svc_root_.Serve(fuchsia::io::OpenFlags::RIGHT_READABLE | fuchsia::io::OpenFlags::RIGHT_WRITABLE,
                  dir.NewRequest().TakeChannel());
  return dir;
}

void App::ConnectToService(const std::string& service_name, zx::channel channel) {
  vfs::internal::Node* child;
  auto status = svc_root_.Lookup(service_name, &child);
  if (status == ZX_OK) {
    status = child->Serve(fuchsia::io::OpenFlags::RIGHT_READABLE, std::move(channel));
  } else if (status == ZX_ERR_NOT_FOUND) {
    FX_LOGS(WARNING) << "Service " << service_name
                     << " not in service list, attempting to connect through environment";
    env_services_->Connect(service_name, std::move(channel));
    status = ZX_OK;
  } else {
    FX_LOGS(ERROR) << "Could not serve " << service_name << ": " << status;
  }
}

void App::RegisterSingleton(std::string service_name, fuchsia::sys::LaunchInfoPtr launch_info) {
  // The ComponentController is kept alive under the following functor's state.
  auto child = std::make_unique<vfs::Service>([this, service_name,
                                               launch_info = std::move(launch_info),
                                               controller = fuchsia::sys::ComponentControllerPtr()](
                                                  zx::channel client_handle,
                                                  async_dispatcher_t* dispatcher) mutable {
    FX_VLOGS(2) << "Servicing singleton service request for " << service_name;
    std::shared_ptr<sys::ServiceDirectory> svcs;

    auto it = services_.find(launch_info->url);
    // Start component if it isn't already running
    if (it == services_.end()) {
      FX_VLOGS(1) << "Starting singleton " << launch_info->url << " for service " << service_name;
      LaunchComponent(
          *launch_info,
          [service_name, url = launch_info->url](int64_t, fuchsia::sys::TerminationReason reason) {
            if (reason == fuchsia::sys::TerminationReason::PACKAGE_NOT_FOUND) {
              FX_LOGS(ERROR) << "Could not load package for service " << service_name << " at "
                             << url;
            }
          },
          [url = launch_info->url](zx_status_t) {
            FX_LOGS(ERROR) << "Singleton component " << url << " died";
          });
      it = services_.find(launch_info->url);
      FX_DCHECK(it != services_.end());
    }
    it->second->Connect(service_name, std::move(client_handle));
  });
  svc_names_.push_back(service_name);
  svc_root_.AddEntry(service_name, std::move(child));
}

void App::LaunchComponent(const fuchsia::sys::LaunchInfo& launch_info,
                          fuchsia::sys::ComponentController::OnTerminatedCallback on_terminate,
                          fit::function<void(zx_status_t)> on_ctrl_err) {
  FX_VLOGS(1) << "Launching component " << launch_info.url;

  fuchsia::sys::ComponentControllerPtr ctrl;
  ctrl.events().OnTerminated = std::move(on_terminate);
  ctrl.set_error_handler([this, on_ctrl_err = std::move(on_ctrl_err),
                          url = launch_info.url](zx_status_t status) mutable {
    // move the controller on to the stack first before removing it from |controllers_|; otherwise,
    // this lambda's lifetime ends when we remove the controller from |controllers_|.
    auto ctrl = std::move(controllers_[url]);
    controllers_.erase(url);
    if (on_ctrl_err) {
      on_ctrl_err(status);
    }
    services_.erase(url);
  });

  // Launch the component
  fuchsia::sys::LaunchInfo dup_launch_info;
  dup_launch_info.url = launch_info.url;
  services_.emplace(launch_info.url,
                    sys::ServiceDirectory::CreateWithRequest(&dup_launch_info.directory_request));
  fidl::Clone(launch_info.arguments, &dup_launch_info.arguments);
  env_launcher_->CreateComponent(std::move(dup_launch_info), ctrl.NewRequest());
  controllers_[launch_info.url] = std::move(ctrl);
}

}  // namespace sysmgr
