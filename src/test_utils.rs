use std::{env::temp_dir, path::PathBuf, process::id};

use crate::{ alloc::alloc_inode, directory::add_entry, inode::{DIRECT_POINTERS, Inode, Type}, inode_table, mem::BufferPooll, mkfs::mkfs, raw_device::{FileDevice, RawDevice}, superblock::{ROOT_INODE, Superblock}};
pub const MIB: u64 = 1024 * 1024;

pub fn temp_image(name: &str) -> PathBuf{
    let mut buf = temp_dir();
    buf.push(format!("fs_{}_{}.img", name, id()));
    buf
}

pub fn full_image(name: &str, size: u64) -> (FileDevice, BufferPooll, Superblock){
    let image = temp_image(name);
    let mut file_device = FileDevice::create(image.to_str().unwrap(), size).unwrap();
    let pool = BufferPooll::new();
    let superblock = mkfs(&pool, &mut file_device, None).unwrap();

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

    pub struct Tree {
       pub a: u32,
       pub b: u32,
       pub f: u32,
       pub g: u32,
       pub top: u32,
    }

    pub fn mk_node<D: RawDevice>(device: &mut D, pool: &BufferPooll, sb: &mut Superblock, parent: u32, name: &str, kind: Type,) -> u32 {
        let id = alloc_inode(sb, pool, device).unwrap();
        let mut inode = Inode::empty(id);
        inode.inode_type = kind;
        inode.nlink = 1;
        inode.mode = 0o755;
        inode_table::write(&inode, device, sb, pool).unwrap();

        if kind == Type::Dir {
            add_entry(device, sb, &mut inode, pool, id, ".").unwrap();
            add_entry(device, sb, &mut inode, pool, parent, "..").unwrap();
        }

        let mut p = inode_table::read(parent, device, sb, pool).unwrap();
        add_entry(device, sb, &mut p, pool, id, name).unwrap();
        id
    }

    pub fn setup(name: &str) -> (FileDevice, BufferPooll, Superblock, Tree) {
        let (mut device, pool, mut sb) = full_image(name, 4 * MIB);
        let top = mk_node(&mut device, &pool, &mut sb, ROOT_INODE, "top", Type::File);
        let a = mk_node(&mut device, &pool, &mut sb, ROOT_INODE, "a", Type::Dir);
        let g = mk_node(&mut device, &pool, &mut sb, a, "g", Type::File);
        let b = mk_node(&mut device, &pool, &mut sb, a, "b", Type::Dir);
        let f = mk_node(&mut device, &pool, &mut sb, b, "f", Type::File);
        assert_eq!(pool.in_use(), 0);
        (device, pool, sb, Tree { a, b, f, g, top })
    }

    pub fn mk_node_tree<D: RawDevice>(device: &mut D, pool: &BufferPooll, sb: &mut Superblock) -> Tree {
        let top = mk_node(device, &pool, sb, ROOT_INODE, "top", Type::File);
        let a = mk_node(device, &pool, sb, ROOT_INODE, "a", Type::Dir);
        let g = mk_node(device, &pool, sb, a, "g", Type::File);
        let b = mk_node(device, &pool, sb, a, "b", Type::Dir);
        let f = mk_node(device, &pool, sb, b, "f", Type::File);
        Tree { a, b, f, g, top }
    }