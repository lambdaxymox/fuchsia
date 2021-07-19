// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <lib/zxio/cpp/inception.h>
#include <lib/zxio/cpp/vector.h>
#include <lib/zxio/null.h>
#include <lib/zxio/ops.h>
#include <sys/stat.h>

static zxio_pipe_t& zxio_get_pipe(zxio_t* io) { return *reinterpret_cast<zxio_pipe_t*>(io); }

static constexpr zxio_ops_t zxio_pipe_ops = []() {
  zxio_ops_t ops = zxio_default_ops;
  ops.close = [](zxio_t* io) {
    zxio_get_pipe(io).~zxio_pipe_t();
    return ZX_OK;
  };

  ops.release = [](zxio_t* io, zx_handle_t* out_handle) {
    *out_handle = zxio_get_pipe(io).socket.release();
    return ZX_OK;
  };

  ops.clone = [](zxio_t* io, zx_handle_t* out_handle) {
    zx::socket out_socket;
    zx_status_t status = zxio_get_pipe(io).socket.duplicate(ZX_RIGHT_SAME_RIGHTS, &out_socket);
    if (status != ZX_OK) {
      return status;
    }
    *out_handle = out_socket.release();
    return ZX_OK;
  };

  ops.attr_get = [](zxio_t* io, zxio_node_attributes_t* out_attr) {
    zxio_node_attributes_t attr = {};
    ZXIO_NODE_ATTR_SET(attr, protocols, ZXIO_NODE_PROTOCOL_PIPE);
    ZXIO_NODE_ATTR_SET(
        attr, abilities,
        ZXIO_OPERATION_READ_BYTES | ZXIO_OPERATION_WRITE_BYTES | ZXIO_OPERATION_GET_ATTRIBUTES);
    *out_attr = attr;
    return ZX_OK;
  };

  ops.wait_begin = [](zxio_t* io, zxio_signals_t zxio_signals, zx_handle_t* out_handle,
                      zx_signals_t* out_zx_signals) {
    *out_handle = zxio_get_pipe(io).socket.get();

    zx_signals_t zx_signals = ZX_SIGNAL_NONE;
    if (zxio_signals & ZXIO_SIGNAL_READABLE) {
      zx_signals |= ZX_SOCKET_READABLE;
    }
    if (zxio_signals & ZXIO_SIGNAL_WRITABLE) {
      zx_signals |= ZX_SOCKET_WRITABLE;
    }
    if (zxio_signals & ZXIO_SIGNAL_READ_DISABLED) {
      zx_signals |= ZX_SOCKET_PEER_WRITE_DISABLED;
    }
    if (zxio_signals & ZXIO_SIGNAL_WRITE_DISABLED) {
      zx_signals |= ZX_SOCKET_WRITE_DISABLED;
    }
    if (zxio_signals & ZXIO_SIGNAL_READ_THRESHOLD) {
      zx_signals |= ZX_SOCKET_READ_THRESHOLD;
    }
    if (zxio_signals & ZXIO_SIGNAL_WRITE_THRESHOLD) {
      zx_signals |= ZX_SOCKET_WRITE_THRESHOLD;
    }
    if (zxio_signals & ZXIO_SIGNAL_PEER_CLOSED) {
      zx_signals |= ZX_SOCKET_PEER_CLOSED;
    }
    *out_zx_signals = zx_signals;
  };

  ops.wait_end = [](zxio_t* io, zx_signals_t zx_signals, zxio_signals_t* out_zxio_signals) {
    zxio_signals_t zxio_signals = ZXIO_SIGNAL_NONE;
    if (zx_signals & ZX_SOCKET_READABLE) {
      zxio_signals |= ZXIO_SIGNAL_READABLE;
    }
    if (zx_signals & ZX_SOCKET_WRITABLE) {
      zxio_signals |= ZXIO_SIGNAL_WRITABLE;
    }
    if (zx_signals & ZX_SOCKET_PEER_WRITE_DISABLED) {
      zxio_signals |= ZXIO_SIGNAL_READ_DISABLED;
    }
    if (zx_signals & ZX_SOCKET_WRITE_DISABLED) {
      zxio_signals |= ZXIO_SIGNAL_WRITE_DISABLED;
    }
    if (zx_signals & ZX_SOCKET_READ_THRESHOLD) {
      zxio_signals |= ZXIO_SIGNAL_READ_THRESHOLD;
    }
    if (zx_signals & ZX_SOCKET_WRITE_THRESHOLD) {
      zxio_signals |= ZXIO_SIGNAL_WRITE_THRESHOLD;
    }
    if (zx_signals & ZX_SOCKET_PEER_CLOSED) {
      zxio_signals |= ZXIO_SIGNAL_PEER_CLOSED;
    }
    *out_zxio_signals = zxio_signals;
  };

  ops.get_read_buffer_available = [](zxio_t* io, size_t* out_available) {
    if (out_available == nullptr) {
      return ZX_ERR_INVALID_ARGS;
    }
    zx_info_socket_t info;
    memset(&info, 0, sizeof(info));
    zx_status_t status =
        zxio_get_pipe(io).socket.get_info(ZX_INFO_SOCKET, &info, sizeof(info), nullptr, nullptr);
    if (status != ZX_OK) {
      return status;
    }
    *out_available = info.rx_buf_available;
    return ZX_OK;
  };

  ops.shutdown = [](zxio_t* io, zxio_shutdown_options_t options) {
    // TODO(https://fxbug.dev/78129): Update to zx::socket::set_disposition() once stream sockets
    // in fdio stop using this zxio shutdown operation.
    static_assert(ZX_SOCKET_SHUTDOWN_READ == ZXIO_SHUTDOWN_OPTIONS_READ);
    static_assert(ZX_SOCKET_SHUTDOWN_WRITE == ZXIO_SHUTDOWN_OPTIONS_WRITE);
    if ((options & ZX_SOCKET_SHUTDOWN_MASK) != options) {
      return ZX_ERR_INVALID_ARGS;
    }
    return zxio_get_pipe(io).socket.shutdown(options);
  };

  return ops;
}();

namespace {

zx_status_t zxio_pipe_read_status(zx_status_t status, size_t* out_actual) {
  // We've reached end-of-file, which is signaled by successfully reading zero
  // bytes.
  //
  // If we see |ZX_ERR_BAD_STATE|, that implies reading has been disabled for
  // this endpoint.
  if (status == ZX_ERR_PEER_CLOSED || status == ZX_ERR_BAD_STATE) {
    *out_actual = 0;
    status = ZX_OK;
  }
  return status;
}

}  // namespace

static constexpr zxio_ops_t zxio_datagram_pipe_ops = []() {
  zxio_ops_t ops = zxio_pipe_ops;
  ops.readv = [](zxio_t* io, const zx_iovec_t* vector, size_t vector_count, zxio_flags_t flags,
                 size_t* out_actual) {
    uint32_t zx_flags = 0;
    if (flags & ZXIO_PEEK) {
      zx_flags |= ZX_SOCKET_PEEK;
      flags &= ~ZXIO_PEEK;
    }
    if (flags) {
      return ZX_ERR_NOT_SUPPORTED;
    }

    size_t total = 0;
    for (size_t i = 0; i < vector_count; ++i) {
      total += vector[i].capacity;
    }
    std::unique_ptr<uint8_t[]> buf(new uint8_t[total]);

    size_t actual;
    zx_status_t status = zxio_get_pipe(io).socket.read(zx_flags, buf.get(), total, &actual);
    if (status != ZX_OK) {
      return zxio_pipe_read_status(status, out_actual);
    }

    uint8_t* data = buf.get();
    size_t remaining = actual;
    return zxio_do_vector(vector, vector_count, out_actual,
                          [&](void* buffer, size_t capacity, size_t* out_actual) {
                            size_t actual = std::min(capacity, remaining);
                            memcpy(buffer, data, actual);
                            data += actual;
                            remaining -= actual;
                            *out_actual = actual;
                            return ZX_OK;
                          });
  };

  ops.writev = [](zxio_t* io, const zx_iovec_t* vector, size_t vector_count, zxio_flags_t flags,
                  size_t* out_actual) {
    if (flags) {
      return ZX_ERR_NOT_SUPPORTED;
    }

    size_t total = 0;
    for (size_t i = 0; i < vector_count; ++i) {
      total += vector[i].capacity;
    }
    std::unique_ptr<uint8_t[]> buf(new uint8_t[total]);

    uint8_t* data = buf.get();
    for (size_t i = 0; i < vector_count; ++i) {
      memcpy(data, vector[i].buffer, vector[i].capacity);
      data += vector[i].capacity;
    }

    return zxio_get_pipe(io).socket.write(0, buf.get(), total, out_actual);
  };

  return ops;
}();

static constexpr zxio_ops_t zxio_stream_pipe_ops = []() {
  zxio_ops_t ops = zxio_pipe_ops;
  ops.readv = [](zxio_t* io, const zx_iovec_t* vector, size_t vector_count, zxio_flags_t flags,
                 size_t* out_actual) {
    if (flags & ZXIO_PEEK) {
      return zxio_datagram_pipe_ops.readv(io, vector, vector_count, flags, out_actual);
    }
    if (flags) {
      return ZX_ERR_NOT_SUPPORTED;
    }

    zx::socket& socket = zxio_get_pipe(io).socket;

    return zxio_pipe_read_status(
        zxio_do_vector(vector, vector_count, out_actual,
                       [&](void* buffer, size_t capacity, size_t* out_actual) {
                         return socket.read(0, buffer, capacity, out_actual);
                       }),
        out_actual);
  };

  ops.writev = [](zxio_t* io, const zx_iovec_t* vector, size_t vector_count, zxio_flags_t flags,
                  size_t* out_actual) {
    if (flags) {
      return ZX_ERR_NOT_SUPPORTED;
    }

    return zxio_do_vector(vector, vector_count, out_actual,
                          [&](void* buffer, size_t capacity, size_t* out_actual) {
                            return zxio_get_pipe(io).socket.write(0, buffer, capacity, out_actual);
                          });
  };

  return ops;
}();

zx_status_t zxio_pipe_init(zxio_storage_t* storage, zx::socket socket, zx_info_socket_t info) {
  auto pipe = new (storage) zxio_pipe_t{
      .io = storage->io,
      .socket = std::move(socket),
  };
  zxio_init(&pipe->io,
            info.options & ZX_SOCKET_DATAGRAM ? &zxio_datagram_pipe_ops : &zxio_stream_pipe_ops);
  return ZX_OK;
}
