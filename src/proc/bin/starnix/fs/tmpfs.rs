// Copyright 2021 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use parking_lot::Mutex;
use std::sync::Arc;

use super::directory_file::MemoryDirectoryFile;
use super::*;
use crate::fs::pipe::Pipe;
use crate::types::*;

pub struct TmpFs(());

impl FileSystemOps for Arc<TmpFs> {
    fn rename(
        &self,
        _fs: &FileSystem,
        _old_parent: &FsNodeHandle,
        _old_name: &FsStr,
        _new_parent: &FsNodeHandle,
        _new_name: &FsStr,
        _renamed: &FsNodeHandle,
        _replaced: Option<&FsNodeHandle>,
    ) -> Result<(), Errno> {
        Ok(())
    }
}

impl TmpFs {
    pub fn new() -> FileSystemHandle {
        let ops = Arc::new(TmpFs(()));
        FileSystem::new(ops, FsNode::new_root(TmpfsDirectory), None, true)
    }
}

struct TmpfsDirectory;

impl FsNodeOps for TmpfsDirectory {
    fn open(&self, _node: &FsNode, _flags: OpenFlags) -> Result<Box<dyn FileOps>, Errno> {
        Ok(Box::new(MemoryDirectoryFile::new()))
    }

    fn lookup(&self, _node: &FsNode, _name: &FsStr) -> Result<FsNodeHandle, Errno> {
        Err(ENOENT)
    }

    fn mkdir(&self, node: &FsNode, _name: &FsStr) -> Result<FsNodeHandle, Errno> {
        node.info_write().link_count += 1;
        Ok(node.fs().create_node(Box::new(TmpfsDirectory), FileMode::IFDIR))
    }

    fn mknod(&self, node: &FsNode, _name: &FsStr, mode: FileMode) -> Result<FsNodeHandle, Errno> {
        let ops: Box<dyn FsNodeOps> = match mode.fmt() {
            FileMode::IFREG => Box::new(VmoFileNode::new()?),
            FileMode::IFIFO => Box::new(FifoNode::new()),
            FileMode::IFBLK => Box::new(DeviceNode),
            FileMode::IFCHR => Box::new(DeviceNode),
            _ => return Err(EACCES),
        };
        Ok(node.fs().create_node(ops, mode))
    }

    fn create_symlink(
        &self,
        node: &FsNode,
        _name: &FsStr,
        target: &FsStr,
    ) -> Result<FsNodeHandle, Errno> {
        Ok(node.fs().create_node(Box::new(SymlinkNode::new(target)), FileMode::IFLNK))
    }

    fn link(&self, _node: &FsNode, _name: &FsStr, child: &FsNodeHandle) -> Result<(), Errno> {
        child.info_write().link_count += 1;
        Ok(())
    }

    fn unlink(&self, node: &FsNode, _name: &FsStr, child: &FsNodeHandle) -> Result<(), Errno> {
        if child.is_dir() {
            node.info_write().link_count -= 1;
        }
        child.info_write().link_count -= 1;
        Ok(())
    }
}

struct FifoNode {
    /// The pipe located at this node.
    pipe: Arc<Mutex<Pipe>>,
}

impl FifoNode {
    fn new() -> FifoNode {
        FifoNode { pipe: Pipe::new() }
    }
}

impl FsNodeOps for FifoNode {
    fn open(&self, _node: &FsNode, flags: OpenFlags) -> Result<Box<dyn FileOps>, Errno> {
        Ok(Pipe::open(&self.pipe, flags))
    }
}

pub struct DeviceNode;

impl FsNodeOps for DeviceNode {
    fn open(&self, _node: &FsNode, _flags: OpenFlags) -> Result<Box<dyn FileOps>, Errno> {
        Err(ENOSYS)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use fuchsia_async as fasync;
    use fuchsia_zircon as zx;
    use std::sync::Arc;
    use zerocopy::AsBytes;

    use crate::mm::*;
    use crate::testing::*;

    #[test]
    fn test_tmpfs() {
        let fs = TmpFs::new();
        let root = fs.root();
        let usr = root.create_dir(b"usr").unwrap();
        let _etc = root.create_dir(b"etc").unwrap();
        let _usr_bin = usr.create_dir(b"bin").unwrap();
        let mut names = root.copy_child_names();
        names.sort();
        assert!(names.iter().eq([b"etc", b"usr"].iter()));
    }

    #[fasync::run_singlethreaded(test)]
    async fn test_write_read() {
        let (_kernel, task_owner) = create_kernel_and_task();
        let task = &task_owner.task;

        let test_mem_size = 0x10000;
        let test_vmo = Arc::new(zx::Vmo::create(test_mem_size).unwrap());

        let path = b"test.bin";
        let _file = task
            .fs
            .root
            .create_node(path, FileMode::IFREG | FileMode::ALLOW_ALL, DeviceType::NONE)
            .unwrap();

        let wr_file = task.open_file(path, OpenFlags::RDWR).unwrap();

        let flags = zx::VmarFlags::PERM_READ | zx::VmarFlags::PERM_WRITE;
        let test_addr = task
            .mm
            .map(
                UserAddress::default(),
                test_vmo,
                0,
                test_mem_size as usize,
                flags,
                MappingOptions::empty(),
            )
            .unwrap();

        let seq_addr = UserAddress::from_ptr(test_addr.ptr() + path.len());
        let test_seq = 0..10000u16;
        let test_vec = test_seq.collect::<Vec<_>>();
        let test_bytes = test_vec.as_slice().as_bytes();
        task.mm.write_memory(seq_addr, test_bytes).unwrap();
        let buf = [UserBuffer { address: seq_addr, length: test_bytes.len() }];

        let written = wr_file.write(task, &buf).unwrap();
        assert_eq!(written, test_bytes.len());

        let mut read_vec = vec![0u8; test_bytes.len()];
        task.mm.read_memory(seq_addr, read_vec.as_bytes_mut()).unwrap();

        assert_eq!(test_bytes, &*read_vec);
    }

    #[fasync::run_singlethreaded(test)]
    async fn test_permissions() {
        let (_kernel, task_owner) = create_kernel_and_task();
        let task = &task_owner.task;

        let path = b"test.bin";
        let file = task
            .open_file_at(
                FdNumber::AT_FDCWD,
                path,
                OpenFlags::CREAT | OpenFlags::RDONLY,
                FileMode::ALLOW_ALL,
            )
            .expect("failed to create file");
        assert_eq!(0, file.read(task, &[]).expect("failed to read"));
        assert!(file.write(task, &[]).is_err());

        let file = task
            .open_file_at(FdNumber::AT_FDCWD, path, OpenFlags::WRONLY, FileMode::ALLOW_ALL)
            .expect("failed to open file WRONLY");
        assert!(file.read(task, &[]).is_err());
        assert_eq!(0, file.write(task, &[]).expect("failed to write"));

        let file = task
            .open_file_at(FdNumber::AT_FDCWD, path, OpenFlags::RDWR, FileMode::ALLOW_ALL)
            .expect("failed to open file RDWR");
        assert_eq!(0, file.read(task, &[]).expect("failed to read"));
        assert_eq!(0, file.write(task, &[]).expect("failed to write"));
    }

    #[fasync::run_singlethreaded(test)]
    async fn test_persistence() {
        let fs = TmpFs::new();
        {
            let root = fs.root();
            let usr = root.create_dir(b"usr").expect("failed to create usr");
            root.create_dir(b"etc").expect("failed to create usr/etc");
            usr.create_dir(b"bin").expect("failed to create usr/bin");
        }

        // At this point, all the nodes are dropped.

        let (_kernel, task_owner) = create_kernel_and_task_with_fs(FsContext::new(fs));
        let task = &task_owner.task;

        task.open_file(b"/usr/bin", OpenFlags::RDONLY | OpenFlags::DIRECTORY)
            .expect("failed to open /usr/bin");
        assert_eq!(ENOENT, task.open_file(b"/usr/bin/test.txt", OpenFlags::RDWR).unwrap_err());
        task.open_file_at(
            FdNumber::AT_FDCWD,
            b"/usr/bin/test.txt",
            OpenFlags::RDWR | OpenFlags::CREAT,
            FileMode::ALLOW_ALL,
        )
        .expect("failed to create test.txt");
        let txt =
            task.open_file(b"/usr/bin/test.txt", OpenFlags::RDWR).expect("failed to open test.txt");

        let usr_bin =
            task.open_file(b"/usr/bin", OpenFlags::RDONLY).expect("failed to open /usr/bin");
        usr_bin
            .name
            .unlink(task, b"test.txt", UnlinkKind::NonDirectory)
            .expect("failed to unlink test.text");
        assert_eq!(ENOENT, task.open_file(b"/usr/bin/test.txt", OpenFlags::RDWR).unwrap_err());
        assert_eq!(
            ENOENT,
            usr_bin.name.unlink(task, b"test.txt", UnlinkKind::NonDirectory).unwrap_err()
        );

        assert_eq!(0, txt.read(task, &[]).expect("failed to read"));
        std::mem::drop(txt);
        assert_eq!(ENOENT, task.open_file(b"/usr/bin/test.txt", OpenFlags::RDWR).unwrap_err());
        std::mem::drop(usr_bin);

        let usr = task.open_file(b"/usr", OpenFlags::RDONLY).expect("failed to open /usr");
        assert_eq!(ENOENT, task.open_file(b"/usr/foo", OpenFlags::RDONLY).unwrap_err());
        usr.name.unlink(task, b"bin", UnlinkKind::Directory).expect("failed to unlink /usr/bin");
    }
}
