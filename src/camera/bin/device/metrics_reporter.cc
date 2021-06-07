// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be found in the LICENSE file.

#include "src/camera/bin/device/metrics_reporter.h"

#include <lib/syslog/cpp/macros.h>

namespace camera {

namespace {

std::mutex factory_mutex;
MetricsReporter* metrics_reporter;
MetricsReporter* metrics_reporter_nop;

inline constexpr const char kConfigurationInspectorActivePropertyName[] = "active";
inline constexpr const char kConfigurationInspectorNodeName[] = "configurations";
inline constexpr const char kFormatInspectorAspectRatioPropertyName[] = "aspect ratio";
inline constexpr const char kFormatInspectorColorSpacePropertyName[] = "color space";
inline constexpr const char kFormatInspectorDisplayResolutionPropertyName[] = "display resolution";
inline constexpr const char kFormatInspectorOutputResolutionPropertyName[] = "output resolution";
inline constexpr const char kFormatInspectorPixelformatPropertyName[] = "pixel format";
inline constexpr const char kStreamInspectorCropPropertyName[] = "supports crop region";
inline constexpr const char kStreamInspectorFrameratePropertyName[] = "frame rate";
inline constexpr const char kStreamInspectorFramesDroppedPropertyName[] = "frames dropped";
inline constexpr const char kStreamInspectorFramesReceivedPropertyName[] = "frames received";
inline constexpr const char kStreamInspectorImageFormatNodeName[] = "image format";
inline constexpr const char kStreamInspectorNodeName[] = "streams";
inline constexpr const char kStreamInspectorResolutionNodeName[] = "supported resolutions";

std::string ConvertPixelFormatToString(const fuchsia::sysmem::PixelFormat& format) {
  switch (format.type) {
    case fuchsia::sysmem::PixelFormatType::R8G8B8A8:
      return "R8G8B8A8";
    case fuchsia::sysmem::PixelFormatType::BGRA32:
      return "BGRA32";
    case fuchsia::sysmem::PixelFormatType::I420:
      return "I420";
    case fuchsia::sysmem::PixelFormatType::M420:
      return "M420";
    case fuchsia::sysmem::PixelFormatType::NV12:
      return "NV12";
    case fuchsia::sysmem::PixelFormatType::YUY2:
      return "YUY2";
    case fuchsia::sysmem::PixelFormatType::MJPEG:
      return "MJPEG";
    case fuchsia::sysmem::PixelFormatType::YV12:
      return "YV12";
    case fuchsia::sysmem::PixelFormatType::BGR24:
      return "BGR24";
    case fuchsia::sysmem::PixelFormatType::RGB565:
      return "RGB565";
    case fuchsia::sysmem::PixelFormatType::RGB332:
      return "RGB332";
    case fuchsia::sysmem::PixelFormatType::RGB2220:
      return "RGB2220";
    case fuchsia::sysmem::PixelFormatType::L8:
      return "L8";
    case fuchsia::sysmem::PixelFormatType::R8:
      return "R8";
    case fuchsia::sysmem::PixelFormatType::R8G8:
      return "R8G8";
    default:
      return "Unknown";
  }
}

std::string ConvertColorSpaceToString(const fuchsia::sysmem::ColorSpace& color_space) {
  switch (color_space.type) {
    case fuchsia::sysmem::ColorSpaceType::INVALID:
      return "INVALID";
    case fuchsia::sysmem::ColorSpaceType::SRGB:
      return "SRGB";
    case fuchsia::sysmem::ColorSpaceType::REC601_NTSC:
      return "REC601_NTSC";
    case fuchsia::sysmem::ColorSpaceType::REC601_NTSC_FULL_RANGE:
      return "REC601_NTSC_FULL_RANGE";
    case fuchsia::sysmem::ColorSpaceType::REC601_PAL:
      return "REC601_PAL";
    case fuchsia::sysmem::ColorSpaceType::REC601_PAL_FULL_RANGE:
      return "REC601_PAL_FULL_RANGE";
    case fuchsia::sysmem::ColorSpaceType::REC709:
      return "REC709";
    case fuchsia::sysmem::ColorSpaceType::REC2020:
      return "REC2020";
    case fuchsia::sysmem::ColorSpaceType::REC2100:
      return "REC2100";
    default:
      return "Unknown";
  }
}

std::string ConvertResolutionToString(size_t width, size_t height) {
  return std::to_string(width) + "x" + std::to_string(height);
}

std::string ConvertResolutionToString(size_t width, size_t height, size_t bpr) {
  std::string output = ConvertResolutionToString(width, height);
  if (bpr > 0) {
    output += ", stride = " + std::to_string(bpr);
  }

  return output;
}

}  // anonymous namespace

// MetricsReporter implementation
MetricsReporter& MetricsReporter::Get() {
  std::lock_guard<std::mutex> lock(factory_mutex);
  if (metrics_reporter) {
    return *metrics_reporter;
  } else {
    // If the reporter has not initialized yet, we're returning a nop version of it.
    FX_LOGS(WARNING) << "MetricsReporter is not initialized yet.";
    if (!metrics_reporter_nop) {
      metrics_reporter_nop = new MetricsReporter();
    }
    return *metrics_reporter_nop;
  }
}

void MetricsReporter::Initialize(sys::ComponentContext& context) {
  std::lock_guard<std::mutex> lock(factory_mutex);
  if (metrics_reporter) {
    FX_LOGS(DEBUG) << "MetricsReporter is initialized already.";
    return;
  }

  metrics_reporter = new MetricsReporter(context);
  FX_LOGS(INFO) << "MetricsReporter is initialized.";
}

MetricsReporter::MetricsReporter(sys::ComponentContext& context)
    : impl_(std::make_unique<Impl>(context)) {
  std::lock_guard<std::mutex> lock(mutex_);
  InitInspector();
  // TODO(fxbug.dev/75535): Initializes the Cobalt logger.
}

void MetricsReporter::InitInspector() {
  // Initializes the inspector and creates the configuration node.
  impl_->inspector_ = std::make_unique<sys::ComponentInspector>(&impl_->context_);
  impl_->node_ = impl_->inspector_->root().CreateChild(kConfigurationInspectorNodeName);
}

MetricsReporter::Impl::Impl(sys::ComponentContext& context) : context_(context) {}

MetricsReporter::Impl::~Impl() {}

MetricsReporter::ConfigurationRecord::ConfigurationRecord(MetricsReporter::Impl& impl,
                                                          uint32_t index, size_t num_streams)
    : node_(impl.node_.CreateChild(std::to_string(index))),
      active_node_(node_.CreateBool(kConfigurationInspectorActivePropertyName, false)),
      stream_node_(node_.CreateChild(kStreamInspectorNodeName)) {
  // Creates the number of the stream records.
  for (size_t i = 0; i < num_streams; ++i) {
    stream_records_.emplace_back(impl, stream_node_, i);
  }
}

std::unique_ptr<MetricsReporter::ConfigurationRecord>
MetricsReporter::CreateConfigurationRecord(uint32_t index, size_t num_streams) {
  std::lock_guard<std::mutex> lock(mutex_);
  return std::make_unique<ConfigurationRecord>(*impl_, index, num_streams);
}

MetricsReporter::StreamRecord::StreamRecord(MetricsReporter::Impl& impl, inspect::Node& parent,
                                            uint32_t stream_index)
    : node_(parent.CreateChild(std::to_string(stream_index))),
      frame_rate_(node_.CreateString(kStreamInspectorFrameratePropertyName, "")),
      supports_crop_region_(node_.CreateBool(kStreamInspectorCropPropertyName, false)),
      supported_resolutions_node_(node_.CreateChild(kStreamInspectorResolutionNodeName)),
      format_record_(node_),
      frames_received_(node_.CreateUint(kStreamInspectorFramesReceivedPropertyName, 0)),
      frames_dropped_(node_.CreateUint(kStreamInspectorFramesDroppedPropertyName, 0)) {}

void MetricsReporter::StreamRecord::SetProperties(
    const fuchsia::camera3::StreamProperties2& props) {
  frame_rate_.Set(std::to_string(props.frame_rate().numerator) + "/" +
                  std::to_string(props.frame_rate().denominator));
  supports_crop_region_.Set(props.supports_crop_region());
  supported_resolutions_.clear();
  for (const auto& resolution : props.supported_resolutions()) {
    supported_resolutions_.push_back(supported_resolutions_node_.CreateString(
        std::to_string(resolution.width) + "x" + std::to_string(resolution.height), ""));
  }

  format_record_.Set(props.image_format());
}

void MetricsReporter::StreamRecord::FrameReceived() { frames_received_.Add(1); }

void MetricsReporter::StreamRecord::FrameDropped() {
  frames_dropped_.Add(1);

  // TODO(fxbug.dev/75535): Reports a frame drop to the Cobalt logger.
}

MetricsReporter::ImageFormatRecord::ImageFormatRecord(inspect::Node& parent)
    : node_(parent.CreateChild(kStreamInspectorImageFormatNodeName)),
      pixel_format_(node_.CreateString(kFormatInspectorPixelformatPropertyName, "")),
      output_resolution_(node_.CreateString(kFormatInspectorOutputResolutionPropertyName, "")),
      display_resolution_(node_.CreateString(kFormatInspectorDisplayResolutionPropertyName, "")),
      color_space_(node_.CreateString(kFormatInspectorColorSpacePropertyName, "")),
      pixel_aspect_ratio_(node_.CreateString(kFormatInspectorAspectRatioPropertyName, "")) {}

void MetricsReporter::ImageFormatRecord::Set(const fuchsia::sysmem::ImageFormat_2& format) {
  pixel_format_.Set(ConvertPixelFormatToString(format.pixel_format));
  output_resolution_.Set(ConvertResolutionToString(format.coded_width, format.coded_height,
                                                   format.bytes_per_row));
  display_resolution_.Set(ConvertResolutionToString(format.display_width, format.display_height,
                                                    format.display_width));
  color_space_.Set(ConvertColorSpaceToString(format.color_space));
  if (format.has_pixel_aspect_ratio) {
    pixel_aspect_ratio_.Set(ConvertResolutionToString(format.pixel_aspect_ratio_width,
                                                      format.pixel_aspect_ratio_height));
  }
}

}  // namespace camera
