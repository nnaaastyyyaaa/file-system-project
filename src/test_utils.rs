use std::{env::temp_dir, path::PathBuf, process::id};

use crate::{bitmap::mark_tail_used, inode::{DIRECT_POINTERS, Inode, Type}, layout::Layout, mem::BufferPooll, raw_device::{FileDevice, RawDevice}, superblock::Superblock};
pub const MIB: u64 = 1024 * 1024;

fn temp_image(name: &str) -> PathBuf{
    let mut buf = temp_dir();
    buf.push(format!("fs_{}_{}.img", name, id()));
    buf
}

pub fn full_image(name: &str, size: u64) -> (FileDevice, BufferPooll, Superblock){
    let image = temp_image(name);
    let mut file_device = FileDevice::create(image.to_str().unwrap(), size).unwrap();
    let pool = BufferPooll::new();
    let layout = Layout::compute(size, None).unwrap();
    let superblock = Superblock::new(&layout);

    let mut buff = pool.take().unwrap();
    mark_tail_used(&mut buff, superblock.data_table_blocks as usize);
    file_device.write(superblock.data_bitmap_blocks_start, &buff).unwrap();
    drop(buff);

    let mut buff = pool.take().unwrap();
    mark_tail_used(&mut buff, superblock.total_inodes as usize);
    file_device.write(superblock.inode_bitmap_blocks_start, &buff).unwrap();
    drop(buff);

    (file_device, pool, superblock)
}

pub fn inode(id: u32) -> Inode
{
    let mut direct = [0u32; DIRECT_POINTERS];
    for n in 0..12{
        direct[n] = 100 + n as u32;
    }
    Inode { 
        id, 
        inode_type: Type::Symlink, 
        mode: 8, 
        uid: 1, 
        gid: 2, 
        size: 987, 
        nlink: 6, 
        mtime: 123456789, 
        ctime: 987654321, 
        direct, 
        single_indirect: 098, 
        double_indirect: 257 
    }
}