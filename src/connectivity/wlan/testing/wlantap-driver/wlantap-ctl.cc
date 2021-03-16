// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <fuchsia/wlan/device/llcpp/fidl.h>
#include <fuchsia/wlan/tap/llcpp/fidl.h>
#include <lib/async-loop/cpp/loop.h>
#include <lib/async-loop/default.h>
#include <lib/async/dispatcher.h>
#include <lib/ddk/debug.h>
#include <lib/ddk/device.h>
#include <zircon/fidl.h>
#include <zircon/status.h>

#include <memory>
#include <mutex>

#include <ddktl/fidl.h>

#include "src/connectivity/wlan/testing/wlantap-driver/wlantapctl_bind.h"
#include "wlantap-phy.h"

namespace {

namespace wlantap = fuchsia_wlan_tap;

class WlantapDriver {
 public:
  zx_status_t GetOrStartLoop(async_dispatcher_t** out) {
    std::lock_guard<std::mutex> guard(mutex_);
    if (!loop_) {
      auto l = std::make_unique<async::Loop>(&kAsyncLoopConfigNoAttachToCurrentThread);
      zx_status_t status = l->StartThread("wlantap-loop");
      if (status != ZX_OK) {
        return status;
      }
      loop_ = std::move(l);
    }
    *out = loop_->dispatcher();
    return ZX_OK;
  }

 private:
  std::mutex mutex_;
  std::unique_ptr<async::Loop> loop_;
};

struct WlantapCtl : wlantap::WlantapCtl::RawChannelInterface {
  WlantapCtl(WlantapDriver* driver) : driver_(driver) {}

  static void DdkRelease(void* ctx) { delete static_cast<WlantapCtl*>(ctx); }

  void CreatePhy(wlantap::wire::WlantapPhyConfig config, ::zx::channel proxy,
                 CreatePhyCompleter::Sync& completer) override {
    zx_status_t status;

    async_dispatcher_t* loop;
    if ((status = driver_->GetOrStartLoop(&loop)) != ZX_OK) {
      completer.Reply(status);
      return;
    }

    // Convert to HLCPP by transiting through fidl bytes.
    auto phy_config = ::fuchsia::wlan::tap::WlantapPhyConfig::New();
    {
      fidl::OwnedEncodedMessage<wlantap::wire::WlantapPhyConfig> encoded(&config);
      if (!encoded.ok()) {
        completer.Reply(encoded.status());
        return;
      }
      auto converted = fidl::OutgoingToIncomingMessage(encoded.GetOutgoingMessage());
      ZX_ASSERT(converted.ok());
      fidl::DecodedMessage<wlantap::wire::WlantapPhyConfig> decoded(converted.incoming_message());
      if (!decoded.ok()) {
        completer.Reply(status);
        return;
      }
      fidl::Decoder dec(fidl::HLCPPIncomingMessage(
          ::fidl::BytePart(decoded.bytes(), decoded.byte_actual(), decoded.byte_actual()),
          fidl::HandleInfoPart()));
      ::fuchsia::wlan::tap::WlantapPhyConfig::Decode(&dec, phy_config.get(), /* offset = */ 0);
    }

    if ((status = wlan::CreatePhy(device_, std::move(proxy), std::move(phy_config), loop)) !=
        ZX_OK) {
      completer.Reply(status);
    } else {
      completer.Reply(ZX_OK);
    }
  };

  static zx_status_t DdkMessage(void* ctx, fidl_incoming_msg_t* msg, fidl_txn_t* txn) {
    auto self = static_cast<WlantapCtl*>(ctx);

    DdkTransaction transaction(txn);
    wlantap::WlantapCtl::Dispatch(self, msg, &transaction);
    return transaction.Status();
  }

  zx_device_t* device_ = nullptr;
  WlantapDriver* driver_;
};

}  // namespace

zx_status_t wlantapctl_init(void** out_ctx) {
  *out_ctx = new WlantapDriver();
  return ZX_OK;
}

zx_status_t wlantapctl_bind(void* ctx, zx_device_t* parent) {
  auto driver = static_cast<WlantapDriver*>(ctx);
  auto wlantapctl = std::make_unique<WlantapCtl>(driver);
  static zx_protocol_device_t device_ops = {
      .version = DEVICE_OPS_VERSION,
      .release = &WlantapCtl::DdkRelease,
      .message = &WlantapCtl::DdkMessage,
  };
  device_add_args_t args = {.version = DEVICE_ADD_ARGS_VERSION,
                            .name = "wlantapctl",
                            .ctx = wlantapctl.get(),
                            .ops = &device_ops};
  zx_status_t status = device_add(parent, &args, &wlantapctl->device_);
  if (status != ZX_OK) {
    zxlogf(ERROR, "%s: could not add device: %d", __func__, status);
    return status;
  }
  // Transfer ownership to devmgr
  wlantapctl.release();
  return ZX_OK;
}

void wlantapctl_release(void* ctx) { delete static_cast<WlantapDriver*>(ctx); }

static constexpr zx_driver_ops_t wlantapctl_driver_ops = []() {
  zx_driver_ops_t ops = {};
  ops.version = DRIVER_OPS_VERSION;
  ops.init = wlantapctl_init;
  ops.bind = wlantapctl_bind;
  ops.release = wlantapctl_release;
  return ops;
}();

ZIRCON_DRIVER(wlantapctl, wlantapctl_driver_ops, "fuchsia", "0.1");
