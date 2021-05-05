// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_CONNECTIVITY_NETWORK_DRIVERS_NETWORK_DEVICE_DEVICE_RX_QUEUE_H_
#define SRC_CONNECTIVITY_NETWORK_DRIVERS_NETWORK_DEVICE_DEVICE_RX_QUEUE_H_

#include <fuchsia/hardware/network/device/cpp/banjo.h>
#include <lib/zx/port.h>

#include <fbl/alloc_checker.h>
#include <fbl/auto_lock.h>
#include <fbl/mutex.h>

#include "data_structs.h"
#include "definitions.h"
#include "device_interface.h"

namespace network::internal {

class Session;

class RxQueue {
 public:
  static constexpr uint64_t kTriggerRxKey = 1;
  static constexpr uint64_t kSessionSwitchKey = 2;
  static constexpr uint64_t kFifoWatchKey = 3;
  static constexpr uint64_t kQuitWatchKey = 4;

  static zx::status<std::unique_ptr<RxQueue>> Create(DeviceInterface* parent);
  ~RxQueue();

  // Helper function with TA annotations that bridges the gap between parent's locks and local
  // locking requirements; TA is not otherwise able to tell that the |parent| and |parent_| are the
  // same entity.
  void AssertParentRxLocked(DeviceInterface& parent) __TA_REQUIRES(parent.rx_lock())
      __TA_ASSERT(parent_->rx_lock()) {
    ZX_DEBUG_ASSERT(parent_ == &parent);
  }

  // Reclaims all buffers currently held by the device.
  void Reclaim() __TA_REQUIRES(parent_->rx_lock());
  // Drops all queued buffers attributed to the given session, and marks the session as rx-disabled.
  // Called by the DeviceInterface parent when the session is marked as dead.
  void PurgeSession(Session& session);
  // Returns rx buffers to their respective sessions.
  void CompleteRxList(const rx_buffer_t* rx, size_t count) __TA_EXCLUDES(parent_->rx_lock());
  // Notifies watcher thread that the primary session changed.
  void TriggerSessionChanged();
  // Poke watcher thread to try to fetch more rx descriptors.
  void TriggerRxWatch();
  // Kills and joins the watcher thread.
  void JoinThread();

  // A transaction to add buffers from a session to the RxQueue.
  class SessionTransaction {
   public:
    explicit SessionTransaction(RxQueue* parent) __TA_REQUIRES(parent->parent_->rx_lock())
        : queue_(parent) {}
    ~SessionTransaction() __TA_REQUIRES(queue_->parent_->rx_lock()) = default;
    uint32_t remaining();
    void Push(Session* session, uint16_t descriptor);
    void AssertLock(DeviceInterface& parent) __TA_ASSERT(parent.rx_lock()) {
      ZX_DEBUG_ASSERT(queue_->parent_ == &parent);
    }

   private:
    // Pointer to parent queue, not owned.
    RxQueue* const queue_;
    DISALLOW_COPY_ASSIGN_AND_MOVE(SessionTransaction);
  };

 private:
  static constexpr uint16_t kDeviceHasBuffer = 0x01;

  explicit RxQueue(DeviceInterface* parent) : parent_(parent) {}

  struct InFlightBuffer {
    InFlightBuffer() = default;
    InFlightBuffer(Session* session, uint16_t descriptor_index)
        : session(session), descriptor_index(descriptor_index), flags(0) {}
    Session* session;
    uint16_t descriptor_index;
    uint16_t flags;
  };
  // Get a single buffer from the queue, along with its identifier. On success, the buffer is popped
  // from the queue. The returned buffer pointer is still owned by the queue and the pointer should
  // not outlive the currently held lock.
  std::tuple<InFlightBuffer*, uint32_t> GetBuffer() __TA_REQUIRES(parent_->rx_lock());
  // Pops a buffer from the queue, if any are available, and stores the space information in `buff`.
  // Returns ZX_ERR_NO_RESOURCES if there are no buffers available.
  zx_status_t PrepareBuff(rx_space_buffer_t* buff) __TA_REQUIRES(parent_->rx_lock());
  int WatchThread();
  // Reclaims the buffer with `id` from the device. If the buffer's session is still valid, gives it
  // to the session, otherwise drops it.
  void ReclaimBuffer(uint32_t id) __TA_REQUIRES(parent_->rx_lock());

  // pointer to parent device, not owned.
  DeviceInterface* const parent_;
  std::unique_ptr<IndexedSlab<InFlightBuffer>> in_flight_ __TA_GUARDED(parent_->rx_lock());
  std::unique_ptr<RingQueue<uint32_t>> available_queue_ __TA_GUARDED(parent_->rx_lock());
  size_t device_buffer_count_ __TA_GUARDED(parent_->rx_lock()) = 0;

  // there are pre-allocated buffers that are only used by the rx watch thread.
  std::unique_ptr<rx_space_buffer_t[]> space_buffers_;
  std::unique_ptr<BufferParts[]> buffer_parts_;

  zx::port rx_watch_port_;
  std::optional<thrd_t> rx_watch_thread_{};
  std::atomic<bool> running_;

  DISALLOW_COPY_ASSIGN_AND_MOVE(RxQueue);
};

// Newtype for the internal SessionTransaction class to allow other header files to forward declare
// it.
class RxSessionTransaction : public RxQueue::SessionTransaction {
 protected:
  friend RxQueue;
  explicit RxSessionTransaction(RxQueue* parent) : RxQueue::SessionTransaction(parent) {}
};

}  // namespace network::internal

#endif  // SRC_CONNECTIVITY_NETWORK_DRIVERS_NETWORK_DEVICE_DEVICE_RX_QUEUE_H_
