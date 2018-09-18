// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef GARNET_LIB_MACHINA_VIRTIO_WL_H_
#define GARNET_LIB_MACHINA_VIRTIO_WL_H_

#include <unordered_map>

#include <lib/async/cpp/wait.h>
#include <lib/fit/function.h>
#include <lib/zx/channel.h>
#include <virtio/virtio_ids.h>
#include <virtio/wl.h>
#include <zircon/compiler.h>
#include <zircon/types.h>

#include "garnet/lib/machina/virtio_device.h"
#include "garnet/lib/machina/virtio_queue_waiter.h"

#define VIRTWL_VQ_IN 0
#define VIRTWL_VQ_OUT 1
#define VIRTWL_QUEUE_COUNT 2
#define VIRTWL_NEXT_VFD_ID_BASE (1 << 31)
#define VIRTWL_VFD_ID_HOST_MASK VIRTWL_NEXT_VFD_ID_BASE

namespace machina {

// Virtio wayland device.
class VirtioWl : public VirtioInprocessDevice<VIRTIO_ID_WL, VIRTWL_QUEUE_COUNT,
                                              virtio_wl_config_t> {
 public:
  class Vfd {
   public:
    virtual ~Vfd() {}

    // Begin waiting on VFD to become ready. Returns ZX_ERR_NOT_SUPPORTED
    // if VFD type doesn't support waiting.
    virtual zx_status_t BeginWait(async_dispatcher_t* dispatcher) = 0;

    const zx::handle& handle() const { return handle_; }

   protected:
    explicit Vfd(zx_handle_t handle) : handle_(handle) {}

    zx::handle handle_;
  };
  using OnNewConnectionCallback = fit::function<void(zx::channel)>;

  VirtioWl(const PhysMem& phys_mem, zx::vmar vmar,
           async_dispatcher_t* dispatcher,
           OnNewConnectionCallback on_new_connection_callback);
  ~VirtioWl() override = default;

  VirtioQueue* in_queue() { return queue(VIRTWL_VQ_IN); }
  VirtioQueue* out_queue() { return queue(VIRTWL_VQ_OUT); }

  zx::vmar* vmar() { return &vmar_; }

  // Begins processing any descriptors that become available in the queues.
  zx_status_t Init();

 private:
  zx_status_t HandleCommand(VirtioQueue* queue, uint16_t head, uint32_t* used);
  void HandleNew(const virtio_wl_ctrl_vfd_new_t* request,
                 virtio_wl_ctrl_vfd_new_t* response);
  void HandleClose(const virtio_wl_ctrl_vfd_t* request,
                   virtio_wl_ctrl_hdr_t* response);
  void HandleSend(const virtio_wl_ctrl_vfd_send_t* request,
                  uint32_t request_len, virtio_wl_ctrl_hdr_t* response);
  void HandleNewCtx(const virtio_wl_ctrl_vfd_new_t* request,
                    virtio_wl_ctrl_vfd_new_t* response);
  void HandleNewPipe(const virtio_wl_ctrl_vfd_new_t* request,
                     virtio_wl_ctrl_vfd_new_t* response);
  void HandleNewDmabuf(const virtio_wl_ctrl_vfd_new_t* request,
                       virtio_wl_ctrl_vfd_new_t* response);
  void HandleDmabufSync(const virtio_wl_ctrl_vfd_dmabuf_sync_t* request,
                        virtio_wl_ctrl_hdr_t* response);

  void OnConnectionReady(uint32_t vfd_id, async::Wait* wait, zx_status_t status,
                         const zx_packet_signal_t* signal);
  void BeginWaitOnQueue();
  void OnQueueReady(zx_status_t status, uint16_t index);

  zx::vmar vmar_;
  async_dispatcher_t* const dispatcher_;
  OnNewConnectionCallback on_new_connection_callback_;
  async::Wait out_queue_wait_;
  VirtioQueueWaiter in_queue_wait_;
  std::unordered_map<uint32_t, std::unique_ptr<Vfd>> vfds_;
  std::unordered_map<uint32_t, zx_signals_t> ready_vfds_;
  uint32_t next_vfd_id_ = VIRTWL_NEXT_VFD_ID_BASE;
};

}  // namespace machina

#endif  // GARNET_LIB_MACHINA_VIRTIO_WL_H_
