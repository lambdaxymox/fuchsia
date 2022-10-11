// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_MEDIA_AUDIO_SERVICES_MIXER_FIDL_REALTIME_GAIN_CONTROL_SERVER_H_
#define SRC_MEDIA_AUDIO_SERVICES_MIXER_FIDL_REALTIME_GAIN_CONTROL_SERVER_H_

#include <fidl/fuchsia.audio/cpp/wire.h>

#include <memory>
#include <string_view>

#include "lib/fidl/cpp/wire/internal/transport_channel.h"
#include "lib/fidl/cpp/wire/wire_messaging_declarations.h"
#include "src/media/audio/lib/clock/unreadable_clock.h"
#include "src/media/audio/services/common/base_fidl_server.h"
#include "src/media/audio/services/common/fidl_thread.h"
#include "src/media/audio/services/mixer/mix/gain_control.h"

namespace media_audio {

class GainControlServer
    : public BaseFidlServer<GainControlServer, fidl::WireServer, fuchsia_audio::GainControl> {
 public:
  // The returned server will live until the `server_end` channel is closed.
  static std::shared_ptr<GainControlServer> Create(
      std::shared_ptr<const FidlThread> thread,
      fidl::ServerEnd<fuchsia_audio::GainControl> server_end, UnreadableClock reference_clock);

  // Wraps `GainControl::Advance`.
  void Advance(zx::time reference_time);

  // Implements `fidl::WireServer<fuchsia_audio::GainControl>`.
  void SetGain(SetGainRequestView request, SetGainCompleter::Sync& completer) final;
  void SetMute(SetMuteRequestView request, SetMuteCompleter::Sync& completer) final;

  // Returns the internal gain control.
  const GainControl& gain_control() const { return gain_control_; }

 private:
  static inline constexpr std::string_view kClassName = "GainControlServer";
  template <typename ServerT, template <typename T> typename FidlServerT, typename ProtocolT>
  friend class BaseFidlServer;

  explicit GainControlServer(UnreadableClock reference_clock);

  GainControl gain_control_;
};

}  // namespace media_audio

#endif  // SRC_MEDIA_AUDIO_SERVICES_MIXER_FIDL_REALTIME_GAIN_CONTROL_SERVER_H_
