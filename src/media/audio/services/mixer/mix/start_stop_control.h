// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#ifndef SRC_MEDIA_AUDIO_SERVICES_MIXER_MIX_START_STOP_CONTROL_H_
#define SRC_MEDIA_AUDIO_SERVICES_MIXER_MIX_START_STOP_CONTROL_H_

#include <lib/fpromise/result.h>
#include <lib/zx/time.h>

#include <functional>
#include <optional>
#include <variant>

#include "src/media/audio/lib/clock/clock_snapshot.h"
#include "src/media/audio/lib/format2/fixed.h"
#include "src/media/audio/lib/format2/format.h"
#include "src/media/audio/services/mixer/common/basic_types.h"
#include "src/media/audio/services/mixer/mix/ptr_decls.h"

namespace media_audio {

// Controls an audio stream using Start and Stop commands. Commands can be scheduled to happen in
// the future. At most one command (Start or Stop) can be pending at any time. If a new command
// arrives before a pending command takes effect, the pending command is canceled.
class StartStopControl {
 public:
  StartStopControl(const Format& format, TimelineRate media_ticks_per_ns,
                   UnreadableClock reference_clock);

  enum class WhichClock {
    SystemMonotonic,
    Reference,
  };

  // A timestamp relative to either the system monotonic clock or to this control's reference clock.
  struct RealTime {
    WhichClock clock;
    zx::time time;
  };

  // This is used to avoid variant confusion in MediaPosition.
  struct MediaTicks {
    int64_t value;
  };

  // A position in a stream expressed relative to the logical start of the stream, as a
  // zx::duration, a media tick count (defined by `media_ticks_per_ns`), or a frame number.
  using MediaPosition = std::variant<zx::duration, MediaTicks, Fixed>;

  // Describes when a command took effect using all supported units.
  struct When {
    // The real time at which the command took effect, expressed relative to the system monotonic
    // clock and reference clock, respectively.
    zx::time mono_time;
    zx::time reference_time;
    // The position at which the command took effect.
    zx::duration media_time;
    int64_t media_ticks;
    Fixed frame;
  };

  // An error returned by Start.
  enum class StartError {
    Canceled,
  };

  // An error returned by Stop.
  enum class StopError {
    Canceled,
    AlreadyStopped,
  };

  // At `start_time`, start producing or consuming at frame `start_frame`. Put differently,
  // `start_time` is the presentation time of `start_frame`.
  struct StartCommand {
    // When to start. If this is in the past, or is not specified, the command takes effect
    // immediately (during the next call to AdvanceTo).
    std::optional<RealTime> start_time;
    // Which frame to start.
    MediaPosition start_position;
    // This callback is invoked when the start command takes effect (i.e., at `start_time`) or when
    // the command fails. The call back parameter describes when the command was applied (on
    // success) or the error message (on failure). The callback is optional -- it can be nullptr.
    // TODO(fxbug.dev/87651): use fit::inline_callback or a different mechanism
    std::function<void(fpromise::result<When, StartError>)> callback;
  };

  // Stops the control: at `when`, stop producing or consuming frames.
  struct StopCommand {
    // When to stop. This may be a system monotonic time, a reference time, or a position. If not
    // specified, the command takes effect immediately (during the next call to AdvanceTo).
    std::optional<std::variant<RealTime, MediaPosition>> when;
    // This callback is invoked when the start command takes effect (i.e., at `when`), or when the
    // command fails. The call back parameter describes when the command was applied (on success) or
    // the error message (on failure). The callback is optional -- it can be nullptr.
    // TODO(fxbug.dev/87651): use fit::inline_callback or a different mechanism
    std::function<void(fpromise::result<When, StopError>)> callback;
  };

  using Command = std::variant<StartCommand, StopCommand>;
  static void CancelCommand(Command& cmd);

  // Queues a Start or Stop command. The command will remain pending until it is scheduled to occur.
  // If another command arrives before that time, the prior command will be canceled. There is never
  // more than one command pending at a time.
  //
  // If a Start command arrives while the control is already started, the Start command behaves as
  // if it was preceded instantaneously by a Stop.
  //
  // If a Stop command arrives while the control is already stopped, the Stop command fails with
  // error code AlreadyStopped.
  void Start(StartCommand cmd);
  void Stop(StopCommand cmd);

  // Reports if the command is currently started.
  [[nodiscard]] bool is_started() const { return presentation_time_to_frac_frame().has_value(); }

  // Returns a function that translates from reference clock presentation time to frame time, where
  // frame time is represented by a `Fixed::raw_value()` while presentation time is represented by a
  // `zx::time`.
  //
  // Returns std::nullopt if the control is stopped.
  [[nodiscard]] std::optional<TimelineFunction> presentation_time_to_frac_frame() const;

  // Applies all commands scheduled to happen at or before `reference_time`, then advances our
  // current time to `reference_time`.
  //
  // REQUIRED: `reference_time` is >= the last advanced-to time
  void AdvanceTo(const ClockSnapshots& clocks, zx::time reference_time);

  // Reports if there is a command scheduled to execute. If so, returns the scheduled times and
  // `true` if the next command is a StartCommand (or `false` if it's a StopCommand).
  //
  // If the next command is scheduled a long ways in the future on the system monotonic clock, the
  // returned time may be inaccurate because the reference clock may change rate in unpredictable
  // ways between now and the time the command is scheduled. In the worst case, the
  // time-until-scheduled may be off by 0.2% (the maximum rate slew of a zx::clock).
  //
  // REQUIRED: `AdvanceTo` must called at least once before this method (we need a "current time" to
  // report a scheduled time for commands that happen "immediately" and before the first AdvanceTo,
  // the current time is unknown).
  //
  // TODO(fxbug.dev/87651): consider returning an enum instead of a bool
  [[nodiscard]] std::optional<std::pair<When, bool>> PendingCommand(
      const ClockSnapshots& clocks) const;

 private:
  struct LastStartCommand {
    TimelineFunction presentation_time_to_frac_frame;
    zx::time start_reference_time;
    Fixed start_frame;
  };

  void CancelPendingCommand();

  // Reports when the pending command should happen, using `reference_time_for_immediate` as the
  // scheduled time if the pending command should happen immediately.
  std::pair<When, bool> PendingCommand(const ClockSnapshot& ref_clock,
                                       zx::time reference_time_for_immediate) const;
  When PendingStartCommand(const ClockSnapshot& ref_clock, const StartCommand& cmd,
                           zx::time reference_time_for_immediate) const;
  When PendingStopCommand(const ClockSnapshot& ref_clock, const StopCommand& cmd,
                          zx::time reference_time_for_immediate) const;
  void SetMediaPositions(When& when, Fixed frame) const;
  Fixed MediaPositionToFrame(MediaPosition pos) const;

  const Format format_;
  const TimelineRate media_ticks_per_ns_;
  const TimelineRate frac_frames_per_media_ticks_;
  const UnreadableClock reference_clock_;

  std::optional<Command> pending_;
  std::optional<zx::time> reference_time_now_;          // last time passed to AdvanceTo
  std::optional<LastStartCommand> last_start_command_;  // only if currently started
};

}  // namespace media_audio

#endif  // SRC_MEDIA_AUDIO_SERVICES_MIXER_MIX_START_STOP_CONTROL_H_
