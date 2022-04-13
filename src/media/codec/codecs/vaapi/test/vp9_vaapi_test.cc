// Copyright 2022 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <fuchsia/sysmem/cpp/fidl.h>
#include <lib/fdio/directory.h>
#include <lib/fitx/result.h>
#include <lib/syslog/global.h>
#include <stdio.h>

#include <chrono>
#include <cstdint>
#include <cstring>
#include <memory>
#include <thread>

#include <gtest/gtest.h>

#include "src/lib/files/file.h"
#include "src/media/codec/codecs/test/test_codec_packets.h"
#include "src/media/codec/codecs/vaapi/codec_adapter_vaapi_decoder.h"
#include "src/media/codec/codecs/vaapi/codec_runner_app.h"
#include "src/media/codec/codecs/vaapi/vaapi_utils.h"

namespace {

constexpr char kIvfHeaderSignature[] = "DKIF";

struct __attribute__((packed)) IvfFileHeader {
  char signature[4];      // "DKIF"
  uint16_t version;       // always zero
  uint16_t header_size;   // length of header in bytes
  uint32_t fourcc;        // codec FourCC
  uint16_t width;         // width in pixels
  uint16_t height;        // height in pixels
  uint32_t timebase_dem;  // timebase denumerator that defines the unit of IvfFrameHeader.timestamp
                          // in seconds. If num = 2 and dem = 30 then the unit of
                          // IvfFrameHeader.timestamp is 2/30 seconds.
  uint32_t timebase_num;  // timebase numerator
  uint32_t num_frames;    // number of frames in file
  uint32_t unused;
};
static_assert(sizeof(IvfFileHeader) == 32, "IvfFileHeader is the incorrect size");

struct __attribute__((packed)) IvfFrameHeader {
  uint32_t frame_size;  // Size of frame in bytes (does not include header)
  uint64_t timestamp;   // timestamp in units defined in IvfFileHeader
};
static_assert(sizeof(IvfFrameHeader) == 12, "IvfFrameHeader is the incorrect size");

// IVF is a simple file container for VP9 streams. Since Fuchsia is little endian we can just do
// memcpy's and memcmp's not having to worry about byte swaps.
class IvfParser {
 public:
  IvfParser() = default;
  ~IvfParser() = default;

  IvfParser(const IvfParser&) = delete;
  IvfParser& operator=(const IvfParser&) = delete;
  IvfParser(IvfParser&&) = delete;
  IvfParser& operator=(IvfParser&&) = delete;

  fitx::result<std::string, IvfFileHeader> ReadFileHeader(const uint8_t* stream, size_t size) {
    ptr_ = stream;
    end_ = stream + size;

    if (size < sizeof(IvfFileHeader)) {
      return fitx::error("EOF before file header");
    }

    IvfFileHeader file_header;
    std::memcpy(&file_header, ptr_, sizeof(IvfFileHeader));

    if (std::memcmp(file_header.signature, kIvfHeaderSignature, sizeof(file_header.signature)) !=
        0) {
      return fitx::error("IVF signature not valid");
    }

    if (file_header.version != 0) {
      return fitx::error("IVF version unknown");
    }

    if (file_header.header_size != sizeof(IvfFileHeader)) {
      return fitx::error("IVF invalid header file");
    }

    ptr_ += sizeof(IvfFileHeader);
    return fitx::ok(std::move(file_header));
  }

  fitx::result<std::string, std::pair<IvfFrameHeader, const uint8_t*>> ParseFrame() {
    if (static_cast<std::size_t>(end_ - ptr_) < sizeof(IvfFrameHeader)) {
      return fitx::error("Not enough space to parse frame header");
    }

    IvfFrameHeader frame_header;
    std::memcpy(&frame_header, ptr_, sizeof(IvfFrameHeader));
    ptr_ += sizeof(IvfFrameHeader);

    if (static_cast<uint32_t>(end_ - ptr_) < frame_header.frame_size) {
      return fitx::error("Not enough space to parse frame payload");
    }

    const uint8_t* payload = ptr_;
    ptr_ += frame_header.frame_size;

    return fitx::ok(std::make_pair(frame_header, payload));
  }

 private:
  // Current reading position of input stream.
  const uint8_t* ptr_{nullptr};

  // The end position of input stream.
  const uint8_t* end_{nullptr};
};

constexpr uint32_t kVideoWidth = 320u;
constexpr uint32_t kVideoHeight = 240u;

class FakeCodecAdapterEvents : public CodecAdapterEvents {
 public:
  void onCoreCodecFailCodec(const char* format, ...) override {
    va_list args;
    va_start(args, format);
    printf("Got onCoreCodecFailCodec: ");
    vprintf(format, args);
    printf("\n");
    fflush(stdout);
    va_end(args);

    fail_codec_count_++;
    cond_.notify_all();
  }

  void onCoreCodecFailStream(fuchsia::media::StreamError error) override {
    printf("Got onCoreCodecFailStream %d\n", static_cast<int>(error));
    fflush(stdout);
    fail_stream_count_++;
    cond_.notify_all();
  }

  void onCoreCodecResetStreamAfterCurrentFrame() override {}

  void onCoreCodecMidStreamOutputConstraintsChange(bool output_re_config_required) override {
    // Test a representative value.
    auto output_constraints = codec_adapter_->CoreCodecGetBufferCollectionConstraints(
        CodecPort::kOutputPort, fuchsia::media::StreamBufferConstraints(),
        fuchsia::media::StreamBufferPartialSettings());
    EXPECT_TRUE(output_constraints.buffer_memory_constraints.cpu_domain_supported);
    EXPECT_EQ(kVideoWidth, output_constraints.image_format_constraints[0].required_min_coded_width);

    std::unique_lock<std::mutex> lock(lock_);
    // Wait for buffer initialization to complete to ensure all buffers are staged to be loaded.
    cond_.wait(lock, [&]() { return buffer_initialization_completed_; });

    // Fake out the client setting buffer constraints on sysmem
    fuchsia::sysmem::BufferCollectionInfo_2 buffer_collection;
    buffer_collection.settings.image_format_constraints =
        output_constraints.image_format_constraints.at(0);
    codec_adapter_->CoreCodecSetBufferCollectionInfo(CodecPort::kOutputPort, buffer_collection);
    codec_adapter_->CoreCodecMidStreamOutputBufferReConfigFinish();
  }

  void onCoreCodecOutputFormatChange() override {}

  void onCoreCodecInputPacketDone(CodecPacket* packet) override {
    std::lock_guard lock(lock_);
    input_packets_done_.push_back(packet);
    cond_.notify_all();
  }

  void onCoreCodecOutputPacket(CodecPacket* packet, bool error_detected_before,
                               bool error_detected_during) override {
    auto output_format = codec_adapter_->CoreCodecGetOutputFormat(1u, 1u);

    const auto& image_format =
        output_format.format_details().domain().video().uncompressed().image_format;

    // Test a representative value.
    EXPECT_EQ(kVideoWidth, image_format.coded_width);
    EXPECT_EQ(kVideoHeight, image_format.coded_height);
    EXPECT_EQ(fuchsia::sysmem::PixelFormatType::NV12, image_format.pixel_format.type);
    EXPECT_EQ(fuchsia::sysmem::ColorSpaceType::REC709, image_format.color_space.type);

    std::lock_guard lock(lock_);
    output_packets_done_.push_back(packet);
    cond_.notify_all();
  }

  void onCoreCodecOutputEndOfStream(bool error_detected_before) override {}

  void onCoreCodecLogEvent(
      media_metrics::StreamProcessorEvents2MetricDimensionEvent event_code) override {}

  uint64_t fail_codec_count() const { return fail_codec_count_; }
  uint64_t fail_stream_count() const { return fail_stream_count_; }

  void WaitForInputPacketsDone() {
    std::unique_lock<std::mutex> lock(lock_);
    cond_.wait(lock, [this]() { return !input_packets_done_.empty(); });
  }

  void set_codec_adapter(CodecAdapter* codec_adapter) { codec_adapter_ = codec_adapter; }

  void WaitForOutputPacketCount(size_t output_packet_count) {
    std::unique_lock<std::mutex> lock(lock_);
    cond_.wait_for(lock, std::chrono::seconds(4),
                   [&]() { return output_packets_done_.size() == output_packet_count; });
  }

  size_t output_packet_count() const { return output_packets_done_.size(); }

  void SetBufferInitializationCompleted() {
    std::lock_guard lock(lock_);
    buffer_initialization_completed_ = true;
    cond_.notify_all();
  }

  void WaitForCodecFailure(uint64_t failure_count) {
    std::unique_lock<std::mutex> lock(lock_);
    cond_.wait_for(lock, std::chrono::seconds(4),
                   [&]() { return fail_codec_count_ == failure_count; });
  }

 private:
  CodecAdapter* codec_adapter_ = nullptr;
  uint64_t fail_codec_count_{};
  uint64_t fail_stream_count_{};

  std::mutex lock_;
  std::condition_variable cond_;

  std::vector<CodecPacket*> input_packets_done_;
  std::vector<CodecPacket*> output_packets_done_;
  bool buffer_initialization_completed_ = false;
};

class Vp9VaapiTestFixture : public ::testing::Test {
 protected:
  Vp9VaapiTestFixture() = default;
  ~Vp9VaapiTestFixture() override { decoder_.reset(); }

  void SetUp() override {
    EXPECT_TRUE(VADisplayWrapper::InitializeSingletonForTesting());

    // Have to defer the construction of decoder_ until
    // VADisplayWrapper::InitializeSingletonForTesting is called
    decoder_ = std::make_unique<CodecAdapterVaApiDecoder>(lock_, &events_);
    events_.set_codec_adapter(decoder_.get());
  }

  void CodecAndStreamInit() {
    fuchsia::media::FormatDetails format_details;
    format_details.set_format_details_version_ordinal(1);
    format_details.set_mime_type("video/vp9");
    decoder_->CoreCodecInit(format_details);

    auto input_constraints = decoder_->CoreCodecGetBufferCollectionConstraints(
        CodecPort::kInputPort, fuchsia::media::StreamBufferConstraints(),
        fuchsia::media::StreamBufferPartialSettings());
    EXPECT_TRUE(input_constraints.buffer_memory_constraints.cpu_domain_supported);

    decoder_->CoreCodecStartStream();
    decoder_->CoreCodecQueueInputFormatDetails(format_details);
  }

  void CodecStreamStop() {
    decoder_->CoreCodecStopStream();
    decoder_->CoreCodecEnsureBuffersNotConfigured(CodecPort::kOutputPort);
  }

  fitx::result<std::string, IvfFileHeader> InitializeIvfFile(const std::string& file_name) {
    if (!files::ReadFileToVector(file_name, &ivf_file_data_)) {
      return fitx::error("Could not read file at " + file_name);
    }

    return ivf_parser_.ReadFileHeader(ivf_file_data_.data(), ivf_file_data_.size());
  }

  void ParseIvfFileIntoPackets(uint32_t output_packet_count, size_t output_packet_size) {
    // While we have IVF frames create a new input packet to feed to the decoder. VP9 parser expects
    // the packets to be on VP9Frame boundaries and if not will parse multiple VP9 frames as one
    // frame. The packets will share the same underlying VMO buffer but will be offset in the
    // buffer.
    std::vector<uint8_t> payload;
    uint32_t packet_index = 0;
    while (true) {
      auto parse_frame = ivf_parser_.ParseFrame();

      if (parse_frame.is_error()) {
        break;
      }

      auto [frame_header, frame_payload] = parse_frame.value();
      const std::size_t current_size = payload.size();
      payload.resize(current_size + frame_header.frame_size);
      std::memcpy(&payload[current_size], frame_payload, frame_header.frame_size);

      auto input_packet = std::make_unique<CodecPacketForTest>(packet_index);
      input_packet->SetStartOffset(static_cast<uint32_t>(current_size));
      input_packet->SetValidLengthBytes(frame_header.frame_size);
      input_packets_.packets.push_back(std::move(input_packet));

      packet_index += 1;
    }

    // Create a VMO to hold all the VP9 data parsed from the IVF data file and copy the data into
    // the VMO
    test_buffer_ = std::make_unique<CodecBufferForTest>(payload.size(), 0, false);
    std::memcpy(test_buffer_->base(), payload.data(), payload.size());

    // Retroactively set the buffer for the packet and feed the decoder, in packet order. VP9
    // decoders do not support packet reordering
    for (auto& packet : input_packets_.packets) {
      packet->SetBuffer(test_buffer_.get());
      decoder_->CoreCodecQueueInputPacket(packet.get());
    }

    auto test_packets = Packets(output_packet_count);
    test_buffers_ = Buffers(std::vector<size_t>(output_packet_count, output_packet_size));

    test_packets_ = std::vector<std::unique_ptr<CodecPacket>>(output_packet_count);
    for (size_t i = 0; i < output_packet_count; i++) {
      auto& packet = test_packets.packets[i];
      test_packets_[i] = std::move(packet);
      decoder_->CoreCodecAddBuffer(CodecPort::kOutputPort, test_buffers_.buffers[i].get());
    }

    decoder_->CoreCodecConfigureBuffers(CodecPort::kOutputPort, test_packets_);
    for (size_t i = 0; i < output_packet_count; i++) {
      decoder_->CoreCodecRecycleOutputPacket(test_packets_[i].get());
    }

    decoder_->CoreCodecConfigureBuffers(CodecPort::kOutputPort, test_packets_);
  }

  std::mutex lock_;
  FakeCodecAdapterEvents events_;
  std::vector<uint8_t> ivf_file_data_;
  std::unique_ptr<CodecAdapterVaApiDecoder> decoder_;
  IvfParser ivf_parser_;
  TestPackets input_packets_;
  std::unique_ptr<CodecBufferForTest> test_buffer_;
  TestBuffers test_buffers_;
  std::vector<std::unique_ptr<CodecPacket>> test_packets_;
};

TEST_F(Vp9VaapiTestFixture, DecodeBasic) {
  constexpr uint32_t kExpectedOutputPackets = 250u;

  CodecAndStreamInit();

  auto ivf_file_header_result = InitializeIvfFile("/pkg/data/test-25fps.vp9");

  if (ivf_file_header_result.is_error()) {
    FAIL() << ivf_file_header_result.error_value();
  }

  // Ensure the IVF header is what we are expecting
  auto ivf_file_header = ivf_file_header_result.value();
  EXPECT_EQ(0u, ivf_file_header.version);
  EXPECT_EQ(32u, ivf_file_header.header_size);
  EXPECT_EQ(0x30395056u, ivf_file_header.fourcc);  // VP90
  EXPECT_EQ(kVideoWidth, ivf_file_header.width);
  EXPECT_EQ(kVideoHeight, ivf_file_header.height);
  EXPECT_EQ(kExpectedOutputPackets, ivf_file_header.num_frames);

  // Since each decoded frame will be its own output packet, create enough so we don't have to
  // recycle them.
  constexpr uint32_t kOutputPacketCount = 4096;

  // Nothing writes to the output packet so its size doesn't matter.
  constexpr size_t kOutputPacketSize = 4096;

  ParseIvfFileIntoPackets(kOutputPacketCount, kOutputPacketSize);

  events_.SetBufferInitializationCompleted();
  events_.WaitForInputPacketsDone();
  events_.WaitForOutputPacketCount(kExpectedOutputPackets);

  CodecStreamStop();

  EXPECT_EQ(kExpectedOutputPackets, events_.output_packet_count());
  EXPECT_EQ(0u, events_.fail_codec_count());
  EXPECT_EQ(0u, events_.fail_stream_count());
}

TEST(Vp9VaapiTest, Init) {
  EXPECT_TRUE(VADisplayWrapper::InitializeSingletonForTesting());
  fidl::InterfaceRequest<fuchsia::io::Directory> directory_request;
  async::Loop loop(&kAsyncLoopConfigAttachToCurrentThread);

  auto codec_services = sys::ServiceDirectory::CreateWithRequest(&directory_request);

  std::thread codec_thread([directory_request = std::move(directory_request)]() mutable {
    CodecRunnerApp<CodecAdapterVaApiDecoder, NoAdapter> runner_app;
    runner_app.Init();
    fidl::InterfaceHandle<fuchsia::io::Directory> outgoing_directory;
    EXPECT_EQ(ZX_OK, runner_app.component_context()->outgoing()->Serve(
                         outgoing_directory.NewRequest().TakeChannel()));
    EXPECT_EQ(ZX_OK, fdio_service_connect_at(outgoing_directory.channel().get(), "svc",
                                             directory_request.TakeChannel().release()));
    runner_app.Run();
  });

  fuchsia::mediacodec::CodecFactorySyncPtr codec_factory;
  codec_services->Connect(codec_factory.NewRequest());
  fuchsia::media::StreamProcessorPtr stream_processor;
  fuchsia::mediacodec::CreateDecoder_Params params;
  fuchsia::media::FormatDetails input_details;
  input_details.set_mime_type("video/vp9");
  params.set_input_details(std::move(input_details));
  params.set_require_hw(true);
  EXPECT_EQ(ZX_OK, codec_factory->CreateDecoder(std::move(params), stream_processor.NewRequest()));

  stream_processor.set_error_handler([&](zx_status_t status) {
    loop.Quit();
    EXPECT_TRUE(false);
  });

  stream_processor.events().OnInputConstraints =
      [&](fuchsia::media::StreamBufferConstraints constraints) {
        loop.Quit();
        stream_processor.Unbind();
      };

  loop.Run();
  codec_factory.Unbind();

  codec_thread.join();
}

}  // namespace
