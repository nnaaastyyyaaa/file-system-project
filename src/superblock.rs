use crate::{bitmap::BITS_PER_BLOCK, codec::{get_u32, put_u32}, error::{FsError, FsResult}, layout::{INODES_PER_BLOCK, Layout}, raw_device::{BLOCK, BLOCK_SIZE}};

pub const MAGIC: u32 = 0x5253_4653;
pub const ROOT_INODE: u32 = 1;


#[derive(Debug, Clone, PartialEq)]
pub struct Superblock {
    pub total_blocks: u32,
    pub total_inodes: u32,
    pub inode_table_blocks: u32,
    pub data_table_blocks: u32,
    pub inode_bitmap_blocks: u32,
    pub data_bitmap_blocks: u32,
    pub inode_bitmap_blocks_start: u32,
    pub data_bitmap_blocks_start: u32,
    pub data_table_start: u32,
    pub inode_table_start: u32,
    pub journal_start: u32,
    pub journal_blocks: u32,
    pub magic: u32,
    pub block_size: u32,
    pub root_inode: u32,
    pub free_blocks: u32,
    pub free_inodes: u32,
    pub block_cursor: u32,
    pub inode_cursor: u32,
    pub clean_unmount: u32
}

impl Superblock{
   pub fn new(l: &Layout) -> Self{
        Self {
            total_blocks: l.total_blocks,
            total_inodes: l.total_inodes,
            inode_table_blocks: l.inode_table_blocks,
            data_table_blocks: l.data_table_blocks,
            inode_bitmap_blocks: l.inode_bitmap_blocks,
            data_bitmap_blocks: l.data_bitmap_blocks,
            inode_bitmap_blocks_start: l.inode_bitmap_blocks_start,
            data_bitmap_blocks_start: l.data_bitmap_blocks_start,
            data_table_start: l.data_table_start,
            inode_table_start: l.inode_table_start,
            journal_start: l.journal_start,
            journal_blocks: l.journal_blocks,
            magic: MAGIC,
            block_size: BLOCK_SIZE as u32,
            root_inode: ROOT_INODE,
            free_blocks: l.data_table_blocks,
            free_inodes: l.total_inodes,
            block_cursor: 0,
            inode_cursor: 0,
            clean_unmount: 1,
        }
    }

    pub fn encode(&self, buff: &mut BLOCK){
        buff.fill(0);
        put_u32(buff, 0, self.magic);
        put_u32(buff, 4, self.block_size);
        put_u32(buff, 8, self.total_blocks);
        put_u32(buff, 12, self.total_inodes);
        put_u32(buff, 16, self.journal_blocks);
        put_u32(buff, 20, self.journal_start);
        put_u32(buff, 24, self.data_table_start);
        put_u32(buff, 28, self.data_table_blocks);
        put_u32(buff, 32, self.inode_bitmap_blocks_start);
        put_u32(buff, 36, self.inode_bitmap_blocks);
        put_u32(buff, 40, self.data_bitmap_blocks_start);
        put_u32(buff, 44, self.data_bitmap_blocks);
        put_u32(buff, 48, self.inode_table_start);
        put_u32(buff, 52, self.inode_table_blocks);
        put_u32(buff, 56, self.root_inode);
        put_u32(buff, 60, self.free_blocks);
        put_u32(buff, 64, self.free_inodes);
        put_u32(buff, 68, self.block_cursor);
        put_u32(buff, 72, self.inode_cursor);
        put_u32(buff, 76, self.clean_unmount);
    }

    pub fn decode(buff: &BLOCK)-> FsResult<Self>{
        let magic = get_u32(buff,0);
        let block_size = get_u32(buff, 4);

        if magic != MAGIC || block_size != BLOCK_SIZE as u32 {
            return Err(FsError::CORRUPT);
        }

        Ok( Self {
            magic,
            block_size,
            total_blocks: get_u32(buff, 8),
            total_inodes: get_u32(buff, 12),
            journal_blocks: get_u32(buff, 16),
            journal_start: get_u32(buff, 20),
            data_table_start: get_u32(buff, 24),
            data_table_blocks: get_u32(buff, 28),
            inode_bitmap_blocks_start: get_u32(buff, 32),
            inode_bitmap_blocks: get_u32(buff, 36),
            data_bitmap_blocks_start: get_u32(buff, 40),
            data_bitmap_blocks: get_u32(buff, 44),
            inode_table_start: get_u32(buff, 48),
            inode_table_blocks: get_u32(buff, 52),
            root_inode: get_u32(buff, 56),
            free_blocks: get_u32(buff, 60),
            free_inodes: get_u32(buff, 64),
            block_cursor: get_u32(buff, 68),
            inode_cursor: get_u32(buff, 72),
            clean_unmount: get_u32(buff, 76),
        })
    }

    pub fn validate(&self, device_blocks: u32) -> FsResult<()> {
        let sum = |a: u32, b: u32| a as u64 + b as u64;

        let zones_ok = self.journal_start == 1
            && sum(self.journal_start, self.journal_blocks) == self.inode_bitmap_blocks_start as u64
            && sum(self.inode_bitmap_blocks_start, self.inode_bitmap_blocks)
                == self.data_bitmap_blocks_start as u64
            && sum(self.data_bitmap_blocks_start, self.data_bitmap_blocks)
                == self.inode_table_start as u64
            && sum(self.inode_table_start, self.inode_table_blocks) == self.data_table_start as u64
            && sum(self.data_table_start, self.data_table_blocks) == self.total_blocks as u64;
        if !zones_ok {
            return Err(FsError::CORRUPT);
        }

        if self.total_blocks > device_blocks {
            return Err(FsError::CORRUPT);
        }

        let bits = BITS_PER_BLOCK as u64;
        let inode_bitmap_ok = self.inode_bitmap_blocks as u64 * bits >= self.total_inodes as u64;
        let data_bitmap_ok = self.data_bitmap_blocks as u64 * bits >= self.data_table_blocks as u64;
        let table_ok = self.total_inodes as u64
            <= self.inode_table_blocks as u64 * INODES_PER_BLOCK as u64;
        if !(inode_bitmap_ok && data_bitmap_ok && table_ok) {
            return Err(FsError::CORRUPT);
        }

        if self.root_inode < 1 || self.root_inode > self.total_inodes {
            return Err(FsError::CORRUPT);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests{

use super::*;
    const MIB: u64 = 1024 * 1024;

    #[test]
    fn encode_decode_works(){
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  superblock = Superblock::new(&layout);
        let mut buff = [0u8; BLOCK_SIZE];

        superblock.encode(&mut buff);
        assert_eq!(superblock, Superblock::decode(&buff).unwrap());
    }

    #[test]
    fn with_gaps(){
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  mut superblock = Superblock::new(&layout);
        superblock.free_blocks = u32::MAX;
        superblock.free_inodes = 0;
        superblock.block_cursor = u32::MAX;


        let mut buff = [0u8; BLOCK_SIZE];
        superblock.encode(&mut buff);
        assert_eq!(superblock, Superblock::decode(&buff).unwrap());
    }

    #[test]
    fn reject_invalid(){
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  mut superblock = Superblock::new(&layout);
        superblock.magic = 1;

        let mut buff = [0u8; BLOCK_SIZE];
        assert_eq!(Superblock::decode(&buff), Err(FsError::CORRUPT));
        
        superblock.encode(&mut buff);

        assert_eq!(Superblock::decode(&buff), Err(FsError::CORRUPT));

        superblock.magic = MAGIC;
        superblock.block_size = 512;
        superblock.encode(&mut buff);

        assert_eq!(Superblock::decode(&buff), Err(FsError::CORRUPT));
    }

    #[test]
    fn validate_works(){
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  superblock = Superblock::new(&layout);
        assert_eq!(superblock.validate(superblock.total_blocks), Ok(()));

    }

    #[test]
    fn invalid_zones_rejected(){
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  mut superblock = Superblock::new(&layout);

        superblock.journal_start = 2;
        assert_eq!(superblock.validate(superblock.total_blocks), Err(FsError::CORRUPT));

        let  mut superblock = Superblock::new(&layout);
        superblock.inode_bitmap_blocks_start+=1;
        assert_eq!(superblock.validate(superblock.total_blocks), Err(FsError::CORRUPT));

        let  mut superblock = Superblock::new(&layout);
        superblock.data_table_blocks+=1;
        assert_eq!(superblock.validate(superblock.total_blocks), Err(FsError::CORRUPT));
    }

    #[test]
    fn invalid_device_blocks_rejected(){
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  superblock = Superblock::new(&layout);
        assert_eq!(superblock.validate(superblock.total_blocks - 1), Err(FsError::CORRUPT));
    }

    #[test]
    fn too_many_inodes_rejected(){
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  mut superblock = Superblock::new(&layout);
        superblock.total_inodes+=1;
        assert_eq!(superblock.validate(superblock.total_blocks), Err(FsError::CORRUPT));
    }

    #[test]
    fn invalid_root_rejected(){
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  mut superblock = Superblock::new(&layout);
        superblock.root_inode = 0;
        assert_eq!(superblock.validate(superblock.total_blocks), Err(FsError::CORRUPT));
    }

    #[test]
    fn accepts_root_at_boundaries() {
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  mut superblock = Superblock::new(&layout);
        superblock.root_inode = 1;
        assert_eq!(superblock.validate(superblock.total_blocks), Ok(()));

        superblock.root_inode = superblock.total_inodes;
        assert_eq!(superblock.validate(superblock.total_blocks), Ok(()));
    }

    #[test]
    fn garbage_does_not_overflow() {
        let layout = Layout::compute(64*MIB, None).unwrap();
        let  mut superblock = Superblock::new(&layout);
        superblock.journal_blocks = u32::MAX;
        superblock.data_table_blocks = u32::MAX;
        assert_eq!(superblock.validate(u32::MAX), Err(FsError::CORRUPT));
    }

}