// Copyright 2016 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <fcntl.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <threads.h>

#include <fs/vfs.h>
#include <fs/vnode.h>
#include <zircon/process.h>
#include <zx/event.h>
#include <fdio/debug.h>
#include <fdio/io.h>
#include <fdio/remoteio.h>
#include <fdio/vfs.h>
#include <fbl/auto_lock.h>
#include <fbl/ref_ptr.h>

#define MXDEBUG 0

typedef struct vfs_iostate {
    fbl::RefPtr<fs::Vnode> vn;
    // The VFS state & dispatcher associated with this handle.
    fs::Vfs* vfs;
    // Handle to event which allows client to refer to open vnodes in multi-patt
    // operations (see: link, rename). Defaults to ZX_HANDLE_INVALID.
    // Validated on the server side using cookies.
    zx::event token;
    fs::vdircookie_t dircookie;
    size_t io_off;
    uint32_t io_flags;
} vfs_iostate_t;

static bool writable(uint32_t flags) {
    return ((03 & flags) == O_RDWR) || ((03 & flags) == O_WRONLY);
}

static bool readable(uint32_t flags) {
    return ((03 & flags) == O_RDWR) || ((03 & flags) == O_RDONLY);
}

namespace fs {
namespace {

static void txn_handoff_open(zx_handle_t srv, zx::channel channel,
                             const char* path, uint32_t flags, uint32_t mode) {
    zxrio_msg_t msg;
    memset(&msg, 0, ZXRIO_HDR_SZ);
    size_t len = strlen(path);
    msg.op = ZXRIO_OPEN;
    msg.arg = flags;
    msg.arg2.mode = mode;
    msg.datalen = static_cast<uint32_t>(len) + 1;
    memcpy(msg.data, path, len + 1);
    zxrio_txn_handoff(srv, channel.release(), &msg);
}

// Initializes io state for a vnode and attaches it to a dispatcher.
void vfs_rpc_open(zxrio_msg_t* msg, zx::channel channel, fbl::RefPtr<Vnode> vn,
                  vfs_iostate_t* ios, const char* path, uint32_t flags, uint32_t mode) {
    zx_status_t r;

    // The pipeline directive instructs the VFS layer to open the vnode
    // immediately, rather than describing the VFS object to the caller.
    // We check it early so we can throw away the protocol part of flags.
    bool pipeline = flags & O_PIPELINE;
    uint32_t open_flags = flags & (~O_PIPELINE);

    r = ios->vfs->Open(fbl::move(vn), &vn, path, &path, open_flags, mode);

    zxrio_object_t obj;
    memset(&obj, 0, sizeof(obj));
    if (r < 0) {
        xprintf("vfs: open: r=%d\n", r);
        goto done;
    } else if (r > 0) {
        // Remote handoff, either to a remote device or a remote filesystem node.
        txn_handoff_open(r, fbl::move(channel), path, flags, mode);
        return;
    }

    // Acquire the handles to the VFS object
    if ((r = vn->GetHandles(flags, obj.handle, &obj.type, obj.extra, &obj.esize)) < 0) {
        vn->Close();
        goto done;
    }

done:
    // If r >= 0, then we hold a reference to vn from open.
    // Otherwise, vn is closed, and we're simply responding to the client.

    if (pipeline && r > 0) {
        // If a pipeline open was requested, but extra handles are required, then
        // we cannot complete the open in a pipelined fashion.
        while (r-- > 0) {
            zx_handle_close(obj.handle[r]);
        }
        vn->Close();
        return;
    }

    if (!pipeline) {
        // Describe the VFS object to the caller in the non-pipelined case.
        obj.status = (r < 0) ? r : ZX_OK;
        obj.hcount = (r > 0) ? r : 0;
        channel.write(0, &obj, static_cast<uint32_t>(ZXRIO_OBJECT_MINSIZE + obj.esize),
                      obj.handle, obj.hcount);
    }

    if (r < 0) {
        return;
    }

    vn->Serve(ios->vfs, fbl::move(channel), open_flags);
}

void zxrio_reply_channel_status(zx::channel channel, zx_status_t status) {
    struct {
        zx_status_t status;
        uint32_t type;
    } reply = {status, 0};
    channel.write(0, &reply, ZXRIO_OBJECT_MINSIZE, nullptr, 0);
}

static zx_status_t vfs_handler_vn(zxrio_msg_t* msg, fbl::RefPtr<fs::Vnode> vn, vfs_iostate* ios) {
    uint32_t len = msg->datalen;
    int32_t arg = msg->arg;
    msg->datalen = 0;

    // ensure handle count specified by opcode matches reality
    if (msg->hcount != ZXRIO_HC(msg->op)) {
        for (unsigned i = 0; i < msg->hcount; i++) {
            zx_handle_close(msg->handle[i]);
        }
        return ZX_ERR_IO;
    }
    msg->hcount = 0;

    switch (ZXRIO_OP(msg->op)) {
    case ZXRIO_OPEN: {
        char* path = (char*)msg->data;
        zx::channel channel(msg->handle[0]); // take ownership
        if ((len < 1) || (len > PATH_MAX)) {
            fs::zxrio_reply_channel_status(fbl::move(channel), ZX_ERR_INVALID_ARGS);
        } else if ((arg & O_ADMIN) && !(ios->io_flags & O_ADMIN)) {
            fs::zxrio_reply_channel_status(fbl::move(channel), ZX_ERR_ACCESS_DENIED);
        } else {
            path[len] = 0;
            xprintf("vfs: open name='%s' flags=%d mode=%u\n", path, arg, msg->arg2.mode);
            fs::vfs_rpc_open(msg, fbl::move(channel), vn, ios, path, arg, msg->arg2.mode);
        }
        return ERR_DISPATCHER_INDIRECT;
    }
    case ZXRIO_CLOSE: {
        ios->vfs->TokenDiscard(&ios->token);
        // this will drop the ref on the vn
        zx_status_t status = vn->Close();
        ios->vn = nullptr;
        free(ios);
        return status;
    }
    case ZXRIO_CLONE: {
        zx::channel channel(msg->handle[0]); // take ownership
        if (!(arg & O_PIPELINE)) {
            zxrio_object_t obj;
            memset(&obj, 0, ZXRIO_OBJECT_MINSIZE);
            obj.type = FDIO_PROTOCOL_REMOTE;
            channel.write(0, &obj, ZXRIO_OBJECT_MINSIZE, 0, 0);
        }
        vn->Serve(ios->vfs, fbl::move(channel), ios->io_flags);
        return ERR_DISPATCHER_INDIRECT;
    }
    case ZXRIO_READ: {
        if (!readable(ios->io_flags)) {
            return ZX_ERR_BAD_HANDLE;
        }
        ssize_t r = vn->Read(msg->data, arg, ios->io_off);
        if (r >= 0) {
            ios->io_off += r;
            msg->arg2.off = ios->io_off;
            msg->datalen = static_cast<uint32_t>(r);
        }
        return static_cast<zx_status_t>(r);
    }
    case ZXRIO_READ_AT: {
        if (!readable(ios->io_flags)) {
            return ZX_ERR_BAD_HANDLE;
        }
        ssize_t r = vn->Read(msg->data, arg, msg->arg2.off);
        if (r >= 0) {
            msg->datalen = static_cast<uint32_t>(r);
        }
        return static_cast<zx_status_t>(r);
    }
    case ZXRIO_WRITE: {
        if (!writable(ios->io_flags)) {
            return ZX_ERR_BAD_HANDLE;
        }
        if (ios->io_flags & O_APPEND) {
            vnattr_t attr;
            zx_status_t r;
            if ((r = vn->Getattr(&attr)) < 0) {
                return r;
            }
            ios->io_off = attr.size;
        }
        ssize_t r = vn->Write(msg->data, len, ios->io_off);
        if (r >= 0) {
            ios->io_off += r;
            msg->arg2.off = ios->io_off;
        }
        return static_cast<zx_status_t>(r);
    }
    case ZXRIO_WRITE_AT: {
        if (!writable(ios->io_flags)) {
            return ZX_ERR_BAD_HANDLE;
        }
        ssize_t r = vn->Write(msg->data, len, msg->arg2.off);
        return static_cast<zx_status_t>(r);
    }
    case ZXRIO_SEEK: {
        vnattr_t attr;
        zx_status_t r;
        if ((r = vn->Getattr(&attr)) < 0) {
            return r;
        }
        size_t n;
        switch (arg) {
        case SEEK_SET:
            if (msg->arg2.off < 0) {
                return ZX_ERR_INVALID_ARGS;
            }
            n = msg->arg2.off;
            break;
        case SEEK_CUR:
            n = ios->io_off + msg->arg2.off;
            if (msg->arg2.off < 0) {
                // if negative seek
                if (n > ios->io_off) {
                    // wrapped around. attempt to seek before start
                    return ZX_ERR_INVALID_ARGS;
                }
            } else {
                // positive seek
                if (n < ios->io_off) {
                    // wrapped around. overflow
                    return ZX_ERR_INVALID_ARGS;
                }
            }
            break;
        case SEEK_END:
            n = attr.size + msg->arg2.off;
            if (msg->arg2.off < 0) {
                // if negative seek
                if (n > attr.size) {
                    // wrapped around. attempt to seek before start
                    return ZX_ERR_INVALID_ARGS;
                }
            } else {
                // positive seek
                if (n < attr.size) {
                    // wrapped around
                    return ZX_ERR_INVALID_ARGS;
                }
            }
            break;
        default:
            return ZX_ERR_INVALID_ARGS;
        }
        if (vn->IsDevice()) {
            if (n > attr.size) {
                // devices may not seek past the end
                return ZX_ERR_INVALID_ARGS;
            }
        }
        ios->io_off = n;
        msg->arg2.off = ios->io_off;
        return ZX_OK;
    }
    case ZXRIO_STAT: {
        zx_status_t r;
        msg->datalen = sizeof(vnattr_t);
        if ((r = vn->Getattr((vnattr_t*)msg->data)) < 0) {
            return r;
        }
        return msg->datalen;
    }
    case ZXRIO_SETATTR: {
        zx_status_t r = vn->Setattr((vnattr_t*)msg->data);
        return r;
    }
    case ZXRIO_FCNTL: {
        uint32_t cmd = msg->arg;
        constexpr uint32_t kStatusFlags = O_APPEND;
        switch (cmd) {
        case F_GETFL:
            msg->arg2.mode = ios->io_flags & (kStatusFlags | O_ACCMODE);
            return ZX_OK;
        case F_SETFL:
            ios->io_flags = (ios->io_flags & ~kStatusFlags) | (msg->arg2.mode & kStatusFlags);
            return ZX_OK;
        default:
            return ZX_ERR_NOT_SUPPORTED;
        }
    }
    case ZXRIO_READDIR: {
        if (arg > FDIO_CHUNK_SIZE) {
            return ZX_ERR_INVALID_ARGS;
        }
        if (msg->arg2.off == READDIR_CMD_RESET) {
            memset(&ios->dircookie, 0, sizeof(ios->dircookie));
        }
        zx_status_t r;
        {
            fbl::AutoLock lock(&ios->vfs->vfs_lock_);
            r = vn->Readdir(&ios->dircookie, msg->data, arg);
        }
        if (r >= 0) {
            msg->datalen = r;
        }
        return r;
    }
    case ZXRIO_IOCTL_1H: {
        if ((len > FDIO_IOCTL_MAX_INPUT) ||
            (arg > (ssize_t)sizeof(msg->data)) ||
            (IOCTL_KIND(msg->arg2.op) != IOCTL_KIND_SET_HANDLE)) {
            zx_handle_close(msg->handle[0]);
            return ZX_ERR_INVALID_ARGS;
        }
        if (len < sizeof(zx_handle_t)) {
            len = sizeof(zx_handle_t);
        }

        char in_buf[FDIO_IOCTL_MAX_INPUT];
        // The sending side copied the handle into msg->handle[0]
        // so that it would be sent via channel_write().  Here we
        // copy the local version back into the space in the buffer
        // that the original occupied.
        memcpy(in_buf, msg->handle, sizeof(zx_handle_t));
        memcpy(in_buf + sizeof(zx_handle_t), msg->data + sizeof(zx_handle_t),
               len - sizeof(zx_handle_t));

        switch (msg->arg2.op) {
        case IOCTL_VFS_MOUNT_FS:
        case IOCTL_VFS_MOUNT_MKDIR_FS:
            // Mounting requires iostate privileges
            if (!(ios->io_flags & O_ADMIN)) {
                vfs_unmount_handle(msg->handle[0], 0);
                zx_handle_close(msg->handle[0]);
                return ZX_ERR_ACCESS_DENIED;
            }
            // If our permissions validate, fall through to the VFS ioctl
        }
        ssize_t r = ios->vfs->Ioctl(fbl::move(vn), msg->arg2.op, in_buf, len,
                                    msg->data, arg);

        if (r == ZX_ERR_NOT_SUPPORTED) {
            zx_handle_close(msg->handle[0]);
        }

        return static_cast<zx_status_t>(r);
    }
    case ZXRIO_IOCTL: {
        if (len > FDIO_IOCTL_MAX_INPUT ||
            (arg > (ssize_t)sizeof(msg->data)) ||
            (IOCTL_KIND(msg->arg2.op) == IOCTL_KIND_SET_HANDLE)) {
            return ZX_ERR_INVALID_ARGS;
        }
        char in_buf[FDIO_IOCTL_MAX_INPUT];
        memcpy(in_buf, msg->data, len);

        ssize_t r;
        switch (msg->arg2.op) {
        case IOCTL_VFS_GET_TOKEN: {
            // Ioctls which act on iostate
            if (arg != sizeof(zx_handle_t)) {
                r = ZX_ERR_INVALID_ARGS;
            } else {
                zx::event token;
                r = ios->vfs->VnodeToToken(fbl::move(vn), &ios->token, &token);
                if (r == ZX_OK) {
                    r = sizeof(zx_handle_t);
                    zx_handle_t* out = reinterpret_cast<zx_handle_t*>(msg->data);
                    *out = token.release();
                }
            }
            break;
        }
        case IOCTL_VFS_UNMOUNT_NODE:
        case IOCTL_VFS_UNMOUNT_FS:
        case IOCTL_VFS_GET_DEVICE_PATH:
            // Unmounting ioctls require iostate privileges
            if (!(ios->io_flags & O_ADMIN)) {
                r = ZX_ERR_ACCESS_DENIED;
                break;
            }
            // If our permissions validate, fall through to the VFS ioctl
        default:
            r = ios->vfs->Ioctl(fbl::move(vn), msg->arg2.op, in_buf, len, msg->data, arg);
        }
        if (r >= 0) {
            switch (IOCTL_KIND(msg->arg2.op)) {
            case IOCTL_KIND_DEFAULT:
                break;
            case IOCTL_KIND_GET_HANDLE:
                msg->hcount = 1;
                memcpy(msg->handle, msg->data, sizeof(zx_handle_t));
                break;
            case IOCTL_KIND_GET_TWO_HANDLES:
                msg->hcount = 2;
                memcpy(msg->handle, msg->data, 2 * sizeof(zx_handle_t));
                break;
            case IOCTL_KIND_GET_THREE_HANDLES:
                msg->hcount = 3;
                memcpy(msg->handle, msg->data, 3 * sizeof(zx_handle_t));
                break;
            }
            msg->arg2.off = 0;
            msg->datalen = static_cast<uint32_t>(r);
        }
        return static_cast<uint32_t>(r);
    }
    case ZXRIO_TRUNCATE: {
        if (!writable(ios->io_flags)) {
            return ZX_ERR_BAD_HANDLE;
        }
        if (msg->arg2.off < 0) {
            return ZX_ERR_INVALID_ARGS;
        }
        return vn->Truncate(msg->arg2.off);
    }
    case ZXRIO_RENAME:
    case ZXRIO_LINK: {
        // Regardless of success or failure, we'll close the client-provided
        // vnode token handle.
        zx::event token(msg->handle[0]);

        if (len < 4) { // At least one byte for src + dst + null terminators
            return ZX_ERR_INVALID_ARGS;
        }

        char* data_end = (char*)(msg->data + len - 1);
        *data_end = '\0';
        const char* oldname = (const char*)msg->data;
        size_t oldlen = strlen(oldname);
        const char* newname = (const char*)msg->data + (oldlen + 1);

        if (data_end <= newname) {
            return ZX_ERR_INVALID_ARGS;
        }

        switch (ZXRIO_OP(msg->op)) {
        case ZXRIO_RENAME:
            return ios->vfs->Rename(fbl::move(token), fbl::move(vn),
                                    oldname, newname);
        case ZXRIO_LINK:
            return ios->vfs->Link(fbl::move(token), fbl::move(vn),
                                  oldname, newname);
        }
        assert(false);
    }
    case ZXRIO_MMAP: {
        if (len != sizeof(zxrio_mmap_data_t)) {
            return ZX_ERR_INVALID_ARGS;
        }
        zxrio_mmap_data_t* data = reinterpret_cast<zxrio_mmap_data_t*>(msg->data);
        if (ios->io_flags & O_APPEND && data->flags & FDIO_MMAP_FLAG_WRITE) {
            return ZX_ERR_ACCESS_DENIED;
        } else if (!writable(ios->io_flags) && (data->flags & FDIO_MMAP_FLAG_WRITE)) {
            return ZX_ERR_ACCESS_DENIED;
        } else if (!readable(ios->io_flags)) {
            return ZX_ERR_ACCESS_DENIED;
        }

        zx_status_t status = vn->Mmap(data->flags, data->length, &data->offset,
                                      &msg->handle[0]);
        if (status == ZX_OK) {
            msg->hcount = 1;
        }
        return status;
    }
    case ZXRIO_SYNC: {
        return vn->Sync();
    }
    case ZXRIO_UNLINK:
        return ios->vfs->Unlink(fbl::move(vn), (const char*)msg->data, len);
    default:
        // close inbound handles so they do not leak
        for (unsigned i = 0; i < ZXRIO_HC(msg->op); i++) {
            zx_handle_close(msg->handle[i]);
        }
        return ZX_ERR_NOT_SUPPORTED;
    }
}

zx_status_t vfs_handler(zxrio_msg_t* msg, void* cookie) {
    vfs_iostate_t* ios = static_cast<vfs_iostate_t*>(cookie);

    fbl::RefPtr<fs::Vnode> vn = ios->vn;
    zx_status_t status = vfs_handler_vn(msg, fbl::move(vn), ios);
    return status;
}

} // namespace

zx_status_t Vnode::Serve(fs::Vfs* vfs, zx::channel channel, uint32_t flags) {
    zx_status_t r;
    vfs_iostate_t* ios;

    if ((ios = static_cast<vfs_iostate_t*>(calloc(1, sizeof(vfs_iostate_t)))) == nullptr) {
        return ZX_ERR_NO_MEMORY;
    }
    ios->vn = fbl::RefPtr<fs::Vnode>(this);
    ios->io_flags = flags;
    ios->vfs = vfs;

    if ((r = vfs->Serve(fbl::move(channel), ios)) < 0) {
        free(ios);
        return r;
    }
    return ZX_OK;
}

zx_status_t Vfs::Serve(zx::channel channel, void* ios) {
    return dispatcher_->AddVFSHandler(fbl::move(channel), vfs_handler, ios);
}

zx_status_t Vfs::ServeDirectory(fbl::RefPtr<fs::Vnode> vn,
                                zx::channel channel) {
    // Make sure the Vnode really is a directory.
    zx_status_t r;
    if ((r = vn->Open(O_DIRECTORY)) != ZX_OK) {
        return r;
    }

    // Tell the calling process that we've mounted the directory.
    if ((r = channel.signal_peer(0, ZX_USER_SIGNAL_0)) != ZX_OK) {
        return r;
    }

    return vn->Serve(this, fbl::move(channel), O_ADMIN);
}

} // namespace fs
