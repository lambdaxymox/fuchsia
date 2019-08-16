// Copyright 2017 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <arpa/inet.h>
#include <fcntl.h>
#include <fuchsia/hardware/ethernet/c/fidl.h>
#include <lib/fdio/directory.h>
#include <lib/fdio/fd.h>
#include <lib/fdio/fdio.h>
#include <lib/zx/channel.h>
#include <lib/zx/clock.h>
#include <lib/zx/fifo.h>
#include <lib/zx/vmo.h>
#include <limits.h>
#include <netinet/if_ether.h>
#include <netinet/ip.h>
#include <netinet/ip6.h>
#include <netinet/tcp.h>
#include <netinet/udp.h>
#include <stdlib.h>
#include <unistd.h>
#include <zircon/assert.h>
#include <zircon/boot/netboot.h>
#include <zircon/device/ethernet.h>
#include <zircon/process.h>
#include <zircon/syscalls.h>

#include <iomanip>
#include <iostream>
#include <iterator>
#include <sstream>
#include <string>
#include <vector>

#include <pretty/hexdump.h>

#include "filter_builder_impl.h"

static constexpr size_t BUFSIZE = 2048;
static constexpr size_t STRBUFSIZE = 256;
static constexpr int64_t DEFAULT_TIMEOUT_SECONDS = 60;

namespace netdump {

class NetdumpOptions {
 public:
  std::string device;
  bool raw = false;
  bool link_level = false;
  bool promisc = false;
  std::optional<uint64_t> packet_count = std::nullopt;
  size_t verbose_level = 0;
  int dumpfile = 0;
  zx::time timeout_deadline = zx::time::infinite();
  Tokenizer tokenizer{};
  parser::Parser parser{tokenizer};
  FilterPtr filter = nullptr;
  FilterPtr highlight_filter = nullptr;
};

typedef struct {
  uint32_t type;
  uint32_t blk_tot_len;
  uint32_t magic;
  uint16_t major;
  uint16_t minor;
  uint64_t section_len;
  // TODO(smklein): Add options here
  uint32_t blk_tot_len2;
} __attribute__((packed)) pcap_shb_t;

typedef struct {
  uint32_t type;
  uint32_t blk_tot_len;
  uint16_t linktype;
  uint16_t reserved;
  uint32_t snaplen;
  uint32_t blk_tot_len2;
} __attribute__((packed)) pcap_idb_t;

typedef struct {
  uint32_t type;
  uint32_t blk_tot_len;
  uint32_t pkt_len;
} __attribute__((packed)) simple_pkt_t;

static constexpr size_t SIMPLE_PKT_MIN_SIZE = sizeof(simple_pkt_t) + sizeof(uint32_t);

static std::string mac_to_string(const uint8_t mac[ETH_ALEN]) {
  std::stringstream stream;
  stream.flags(std::ios::hex);
  stream.fill('0');
  // clang-format off
  // Cast is required to make `stream` not interpret input as a char.
  stream << std::setw(2) << static_cast<uint16_t>(mac[0]) << ":"
         << std::setw(2) << static_cast<uint16_t>(mac[1]) << ":"
         << std::setw(2) << static_cast<uint16_t>(mac[2]) << ":"
         << std::setw(2) << static_cast<uint16_t>(mac[3]) << ":"
         << std::setw(2) << static_cast<uint16_t>(mac[4]) << ":"
         << std::setw(2) << static_cast<uint16_t>(mac[5]);
  // clang-format on
  return stream.str();
}

static std::string ethtype_to_string(uint16_t ethtype) {
  switch (ethtype) {
    case ETH_P_IP:
      return "IPv4";
    case ETH_P_ARP:
      return "ARP";
    case ETH_P_IPV6:
      return "IPv6";
    case ETH_P_8021Q:
      return "802.1Q";
    default:
      return "Unknown";
  }
}

static std::string protocol_to_string(uint8_t protocol) {
  switch (protocol) {
    case IPPROTO_HOPOPTS:
      return "HOPOPTS";
    case IPPROTO_TCP:
      return "TCP";
    case IPPROTO_UDP:
      return "UDP";
    case IPPROTO_ICMP:
      return "ICMP";
    case IPPROTO_ROUTING:
      return "ROUTING";
    case IPPROTO_FRAGMENT:
      return "FRAGMENT";
    case IPPROTO_ICMPV6:
      return "ICMPV6";
    case IPPROTO_NONE:
      return "NONE";
    default:
      return "Transport Unknown";
  }
}

static std::string port_to_string(uint16_t port) {
  switch (port) {
    case 7:
      return "Echo";
    case 20:
      return "FTP xfer";
    case 21:
      return "FTP ctl";
    case 22:
      return "SSH";
    case 23:
      return "Telnet";
    case 53:
      return "DNS";
    case 69:
      return "TFTP";
    case 80:
      return "HTTP";
    case 115:
      return "SFTP";
    case 123:
      return "NTP";
    case 194:
      return "IRC";
    case 443:
      return "HTTPS";
    case DEBUGLOG_PORT:
      return "Netboot Debug";
    case DEBUGLOG_ACK_PORT:
      return "Netboot Debug ack";
    default:
      return "";
  }
}

static inline std::string port_string_by_verbosity(uint16_t port, size_t verbosity) {
  std::string port_name = port_to_string(port);
  std::stringstream stream;
  stream << port;
  if (verbosity && !port_name.empty()) {
    stream << " (" << port_name << ")";
  }
  return stream.str();
}

// Return a `stringstream` with highlighting applied, as appropriate.
inline std::stringstream highlighted_stream(const Packet& packet, const NetdumpOptions& options) {
  std::stringstream stream;
  if (options.highlight_filter != nullptr && options.highlight_filter->match(packet)) {
    stream << parser::ANSI_HIGHLIGHT;
  }
  return stream;
}

// Write link level information to `stream` and return the ethtype.
uint16_t parse_l2_packet(const Packet& packet, const NetdumpOptions& options,
                         std::stringstream* stream) {
  ZX_ASSERT(stream != nullptr);
  uint16_t ethtype = ntohs(packet.frame->h_proto);
  if (options.link_level) {
    *stream << mac_to_string(packet.frame->h_source) << " > " << mac_to_string(packet.frame->h_dest)
            << ", ethertype " << ethtype_to_string(ethtype) << " (0x" << std::hex << ethtype
            << std::dec << "), ";
  }
  return ethtype;
}

// Write L3 information to `stream` and return the transport protocol number if L3 protocol is IP.
uint8_t parse_l3_packet(uint16_t ethtype, const Packet& packet, const NetdumpOptions& options,
                        std::stringstream* stream) {
  ZX_ASSERT(stream != nullptr);
  const struct iphdr* ip = packet.ip;
  char buf[STRBUFSIZE];
  switch (ip->version) {
    case 4: {
      *stream << "IP4 " << inet_ntop(AF_INET, &ip->saddr, buf, sizeof(buf)) << " > "
              << inet_ntop(AF_INET, &ip->daddr, buf, sizeof(buf)) << ": "
              << protocol_to_string(ip->protocol) << ", "
              << "length " << ntohs(ip->tot_len) << ", ";
      return ip->protocol;
    }
    case 6: {
      const struct ip6_hdr* ipv6 = packet.ipv6;
      *stream << "IP6 " << inet_ntop(AF_INET6, &ipv6->ip6_src.s6_addr, buf, sizeof(buf)) << " > "
              << inet_ntop(AF_INET6, &ipv6->ip6_dst.s6_addr, buf, sizeof(buf)) << ": "
              << protocol_to_string(ipv6->ip6_nxt) << ", "
              << "length " << ntohs(ipv6->ip6_plen) << ", ";
      return ipv6->ip6_nxt;
    }
    default: {
      ZX_DEBUG_ASSERT_MSG(false, "Ethtype recognized but IP headers malformed.");
      return 0;
    }
  }
}

void parse_l4_packet(uint8_t transport_protocol, const Packet& packet,
                     const NetdumpOptions& options, std::stringstream* stream) {
  ZX_ASSERT(stream != nullptr);
  switch (transport_protocol) {
    case IPPROTO_TCP: {
      const struct tcphdr* tcp = packet.tcp;
      *stream << "Ports: " << port_string_by_verbosity(ntohs(tcp->source), options.verbose_level)
              << " > " << port_string_by_verbosity(ntohs(tcp->dest), options.verbose_level);
      return;
    }
    case IPPROTO_UDP: {
      const struct udphdr* udp = packet.udp;
      *stream << "Ports: " << port_string_by_verbosity(ntohs(udp->uh_sport), options.verbose_level)
              << " > " << port_string_by_verbosity(ntohs(udp->uh_dport), options.verbose_level);
      return;
    }
    default: {
      ZX_DEBUG_ASSERT_MSG(false, "Transport protocol recognized but headers malformed.");
      return;
    }
  }
}

void parse_packet(const Packet& packet, const NetdumpOptions& options) {
  std::stringstream stream = highlighted_stream(packet, options);
  uint16_t ethtype = parse_l2_packet(packet, options, &stream);
  if (packet.ip != nullptr) {
    uint8_t transport_protocol = parse_l3_packet(ethtype, packet, options, &stream);
    if (packet.transport != nullptr) {
      parse_l4_packet(transport_protocol, packet, options, &stream);
    } else {
      stream << "L4 headers incomplete or unhandled";  // Protocol is displayed in L3 parsing.
    }
  } else {
    stream << "L3 headers incomplete or unhandled";  // Ethtype is displayed in L2 parsing.
  }
  std::cout << stream.str() << parser::ANSI_RESET << std::endl;
}

inline bool filter_packet(const NetdumpOptions& options, Packet* packet) {
  if (options.filter == nullptr) {
    return true;
  }
  return options.filter->match(*packet);
}

int write_shb(int fd) {
  if (fd == -1) {
    return 0;
  }
  pcap_shb_t shb = {
      .type = 0x0A0D0D0A,
      .blk_tot_len = sizeof(pcap_shb_t),
      .magic = 0x1A2B3C4D,
      .major = 1,
      .minor = 0,
      .section_len = 0xFFFFFFFFFFFFFFFF,
      .blk_tot_len2 = sizeof(pcap_shb_t),
  };

  if (write(fd, &shb, sizeof(shb)) != sizeof(shb)) {
    std::cerr << "Couldn't write PCAP Section Header block" << std::endl;
    return -1;
  }
  return 0;
}

int write_idb(int fd) {
  if (fd == -1) {
    return 0;
  }
  pcap_idb_t idb = {
      .type = 0x00000001,
      .blk_tot_len = sizeof(pcap_idb_t),
      .linktype = 1,
      .reserved = 0,
      // We can't use a zero here, but tcpdump also rejects 2^32 - 1. Try 2^16 - 1.
      // See http://seclists.org/tcpdump/2012/q2/8.
      .snaplen = 0xFFFF,
      .blk_tot_len2 = sizeof(pcap_idb_t),
  };

  if (write(fd, &idb, sizeof(idb)) != sizeof(idb)) {
    std::cerr << "Couldn't write PCAP Interface Description block" << std::endl;
    return -1;
  }

  return 0;
}

inline size_t roundup(size_t a, size_t b) { return ((a) + ((b)-1)) & ~((b)-1); }

int write_packet(int fd, void* data, size_t len) {
  if (fd == -1) {
    return 0;
  }

  size_t padded_len = roundup(len, 4);
  simple_pkt_t pkt = {
      .type = 0x00000003,
      .blk_tot_len = static_cast<uint32_t>(SIMPLE_PKT_MIN_SIZE + padded_len),
      .pkt_len = static_cast<uint32_t>(len),
  };

  // TODO(tkilbourn): rewrite this to offload writing to another thread, and also deal with
  // partial writes
  if (write(fd, &pkt, sizeof(pkt)) != sizeof(pkt)) {
    std::cerr << "Couldn't write packet header" << std::endl;
    return -1;
  }
  if (write(fd, data, len) != static_cast<ssize_t>(len)) {
    std::cerr << "Couldn't write packet" << std::endl;
    return -1;
  }
  if (padded_len > len) {
    size_t padding = padded_len - len;
    ZX_DEBUG_ASSERT(padding <= 3);
    static const uint32_t zero = 0;
    if (write(fd, &zero, padding) != static_cast<ssize_t>(padding)) {
      std::cerr << "Couldn't write padding" << std::endl;
      return -1;
    }
  }
  if (write(fd, &pkt.blk_tot_len, sizeof(pkt.blk_tot_len)) != sizeof(pkt.blk_tot_len)) {
    std::cerr << "Couldn't write packet footer" << std::endl;
    return -1;
  }

  return 0;
}

void handle_rx(const zx::fifo& rx_fifo, char* iobuf, unsigned count,
               const NetdumpOptions& options) {
  eth_fifo_entry_t entries[count];

  if (write_shb(options.dumpfile)) {
    return;
  }
  if (write_idb(options.dumpfile)) {
    return;
  }

  Packet packet{};
  uint64_t packets_remaining =
      (options.packet_count == std::nullopt ? std::numeric_limits<uint64_t>::max()
                                            : *options.packet_count);
  for (; packets_remaining > 0;) {
    size_t n;
    zx_status_t status;
    if ((status = rx_fifo.read(sizeof(entries[0]), entries, countof(entries), &n)) < 0) {
      if (status == ZX_ERR_SHOULD_WAIT) {
        rx_fifo.wait_one(ZX_FIFO_READABLE | ZX_FIFO_PEER_CLOSED, options.timeout_deadline, nullptr);
        if (zx::clock::get_monotonic() >= options.timeout_deadline) {
          return;
        }
        continue;
      }
      std::cerr << "netdump: failed to read rx packets: " << status << std::endl;
      return;
    }

    eth_fifo_entry_t* e = entries;
    for (size_t i = 0; i < n; ++i, ++e) {
      if (e->flags & ETH_FIFO_RX_OK) {
        void* buffer = iobuf + e->offset;
        uint16_t length = e->length;
        bool do_write = true;  // Whether the packet is included for write-out.
        packet.populate(buffer, length);
        if (packet.frame == nullptr) {
          std::cout << "Packet size (" << length << ") too small for Ethernet headers" << std::endl;
          if (options.verbose_level == 2) {
            hexdump8_ex(buffer, length, 0);
          }
          // Not setting `write` to false in order to record small frames.
        } else {
          if (filter_packet(options, &packet)) {
            if (options.raw) {
              std::cout << "---" << std::endl;
              hexdump8_ex(buffer, length, 0);
            } else {
              parse_packet(packet, options);
            }
            --packets_remaining;
          } else {
            do_write = false;
          }
        }

        if (do_write && write_packet(options.dumpfile, buffer, length)) {
          return;
        }

        if (packets_remaining == 0 || zx::clock::get_monotonic() >= options.timeout_deadline) {
          return;
        }
      }

      e->length = BUFSIZE;
      e->flags = 0;
      if ((status = rx_fifo.write(sizeof(*e), e, 1, nullptr)) < 0) {
        std::cerr << "netdump: failed to queue rx packet: " << status << std::endl;
        break;
      }
    }
  }
}

int usage() {
  std::cerr << "usage: netdump [ <option>* ] <network-device>" << std::endl
            << " -t {sec}  : Exit after sec seconds, default " << DEFAULT_TIMEOUT_SECONDS
            << std::endl
            << " -w file   : Write packet output to file in pcapng format" << std::endl
            << " -c count  : Exit after receiving count packets" << std::endl
            << " -e        : Print link-level header information" << std::endl
            << " -f filter : Capture only packets specified by filter" << std::endl
            << " -i filter : Highlight packets specified by filter" << std::endl
            << " -p        : Use promiscuous mode" << std::endl
            << " -v        : Print verbose output" << std::endl
            << " -vv       : Print extra verbose output" << std::endl
            << " --raw     : Print raw bytes of all incoming packets" << std::endl
            << " --fhelp   : Show filter syntax usage" << std::endl
            << " --help    : Show this help message" << std::endl
            << "Filter syntax usage:" << std::endl;
  parser::parser_syntax(&std::cerr);
  return -1;
}

using StringIterator = std::vector<const std::string>::iterator;

int parse_args(StringIterator begin, StringIterator end, NetdumpOptions* options) {
  if (begin >= end) {
    return usage();
  }
  --end;
  std::string last = *end;

  for (; begin < end; ++begin) {
    std::string arg = *begin;
    if (arg == "-c") {
      ++begin;
      if (begin == end) {
        return usage();
      }
      size_t num_end;
      int64_t packet_count = stoll(*begin, &num_end, 10);
      if (packet_count < 0 || num_end < begin->length()) {
        return usage();
      }
      options->packet_count = std::optional(static_cast<uint64_t>(packet_count));
    } else if (arg == "-e") {
      options->link_level = true;
    } else if (arg == "-f" || arg == "-i") {
      ++begin;
      if (begin == end) {
        return usage();
      }
      parser::FilterTreeBuilder builder(options->tokenizer);
      auto parsed = options->parser.parse(*begin, &builder);
      if (auto error = std::get_if<parser::ParseError>(&parsed)) {
        std::cerr << *error << "Use '--fhelp' to see the filter syntax." << std::endl;
        return -1;
      }
      if (arg == "-f") {
        options->filter = std::move(std::get<FilterPtr>(parsed));
      } else {
        options->highlight_filter = std::move(std::get<FilterPtr>(parsed));
      }
    } else if (arg == "-p") {
      options->promisc = true;
    } else if (arg == "-w") {
      ++begin;
      if (begin == end || options->dumpfile != -1) {
        return usage();
      }
      options->dumpfile = open(begin->c_str(), O_WRONLY | O_CREAT);
      if (options->dumpfile < 0) {
        std::cerr << "Error: Could not output to file: " << *begin << std::endl;
        return usage();
      }
    } else if (arg == "-v") {
      options->verbose_level = 1;
    } else if (!arg.compare(0, sizeof("-vv"), "-vv")) {
      // Since this is the max verbosity, adding extra 'v's does nothing.
      options->verbose_level = 2;
    } else if (arg == "--raw") {
      options->raw = true;
    } else if (arg == "-t") {
      int64_t timeout_seconds = DEFAULT_TIMEOUT_SECONDS;
      if (begin < end - 1) {
        size_t num_end;
        int64_t input = stoll(begin[1], &num_end, 10);
        if (num_end == begin[1].length()) {
          // Valid number.
          if (input < 0) {
            return usage();
          }
          timeout_seconds = input;
          ++begin;
        }
      }
      options->timeout_deadline = zx::clock::get_monotonic() + zx::sec(timeout_seconds);
    } else {
      return usage();
    }
  }

  if (last == "--fhelp") {
    parser::parser_syntax(&std::cerr);
    return -1;
  }
  if (last == "--help") {
    return usage();
  }

  options->device = last;
  return 0;
}

}  // namespace netdump

int main(int argc, const char** argv) {
  netdump::NetdumpOptions options;
  options.dumpfile = -1;
  std::vector<std::string> args(argv + 1, argv + argc);
  if (parse_args(args.begin(), args.end(), &options)) {
    return -1;
  }

  int fd;
  if ((fd = open(options.device.c_str(), O_RDWR)) < 0) {
    std::cerr << "netdump: cannot open '" << options.device << "'" << std::endl;
    return -1;
  }

  zx::channel svc;
  zx_status_t status = fdio_get_service_handle(fd, svc.reset_and_get_address());
  if (status != ZX_OK) {
    std::cerr << "netdump: failed to get service handle" << std::endl;
    return -1;
  }

  fuchsia_hardware_ethernet_Fifos fifos;
  zx_status_t call_status = ZX_OK;
  status = fuchsia_hardware_ethernet_DeviceGetFifos(svc.get(), &call_status, &fifos);
  if (status != ZX_OK || call_status != ZX_OK) {
    std::cerr << "netdump: failed to get fifos: " << status << ", " << call_status << std::endl;
    return -1;
  }
  zx::fifo rx_fifo = zx::fifo(fifos.rx);

  unsigned count = fifos.rx_depth / 2;
  zx::vmo iovmo;
  // Allocate shareable ethernet buffer data heap with no options. Non-resizable by default.
  if ((status = zx::vmo::create(count * BUFSIZE, 0, &iovmo)) < 0) {
    return -1;
  }

  char* iobuf;
  if ((status = zx_vmar_map(zx_vmar_root_self(), ZX_VM_PERM_READ | ZX_VM_PERM_WRITE, 0, iovmo.get(),
                            0, count * BUFSIZE, reinterpret_cast<uintptr_t*>(&iobuf))) < 0) {
    return -1;
  }

  status = fuchsia_hardware_ethernet_DeviceSetIOBuffer(svc.get(), iovmo.get(), &call_status);
  if (status != ZX_OK || call_status != ZX_OK) {
    std::cerr << "netdump: failed to set iobuf: " << status << ", " << call_status << std::endl;
    return -1;
  }

  status = fuchsia_hardware_ethernet_DeviceSetClientName(svc.get(), "netdump", 7, &call_status);
  if (status != ZX_OK || call_status != ZX_OK) {
    std::cerr << "netdump: failed to set client name: " << status << ", " << call_status
              << std::endl;
  }

  if (options.promisc) {
    status = fuchsia_hardware_ethernet_DeviceSetPromiscuousMode(svc.get(), true, &call_status);
    if (status != ZX_OK || call_status != ZX_OK) {
      std::cerr << "netdump: failed to set promisc mode: " << status << ", " << call_status
                << std::endl;
    }
  }

  // assign data chunks to ethbufs
  for (unsigned n = 0; n < count; ++n) {
    eth_fifo_entry_t entry = {
        .offset = static_cast<uint32_t>(n * BUFSIZE),
        .length = BUFSIZE,
        .flags = 0,
        .cookie = 0,
    };
    if ((status = zx_fifo_write(fifos.rx, sizeof(entry), &entry, 1, nullptr)) < 0) {
      std::cerr << "netdump: failed to queue rx packet: " << status << std::endl;
      return -1;
    }
  }

  status = fuchsia_hardware_ethernet_DeviceStart(svc.get(), &call_status);
  if (status != ZX_OK || call_status != ZX_OK) {
    std::cerr << "netdump: failed to start network interface" << std::endl;
    return -1;
  }

  status = fuchsia_hardware_ethernet_DeviceListenStart(svc.get(), &call_status);
  if (status != ZX_OK || call_status != ZX_OK) {
    std::cerr << "netdump: failed to start listening" << std::endl;
    return -1;
  }

  handle_rx(rx_fifo, iobuf, count, options);

  zx_handle_close(rx_fifo.get());
  if (options.dumpfile != -1) {
    close(options.dumpfile);
  }
  return 0;
}
