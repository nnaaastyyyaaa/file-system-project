use crate::{bitmap::BITS_PER_BLOCK, error::{FsError, FsResult}, raw_device::BLOCK_SIZE};

pub const INODE_SIZE: usize = 128;
pub const INODES_PER_BLOCK: u32 = (BLOCK_SIZE / INODE_SIZE) as u32;
pub const JOURNAL_BLOCKS: u32 = 256;
pub const INODE_RATIO: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
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
}

impl Layout {
    pub fn compute(image_size: u64, inode_count: Option<u32>) -> FsResult<Layout> {
       let total_blocks = u32::try_from(image_size / BLOCK_SIZE as u64).map_err(|_| FsError::EINVAL)?;
       let wanted_inodes = match inode_count{
            Some(0) => return Err(FsError::EINVAL),   
            Some(n) => n,  
            None => total_blocks.div_ceil(INODE_RATIO),  
       };

        if wanted_inodes == 0 { return Err(FsError::EINVAL); }

       let inode_table_blocks = wanted_inodes.div_ceil(INODES_PER_BLOCK);
       let total_inodes = inode_table_blocks.checked_mul(INODES_PER_BLOCK).ok_or(FsError::EINVAL)?;
       let inode_bitmap_blocks = total_inodes.div_ceil(BITS_PER_BLOCK as u32);
       let data_bitmap_blocks = total_blocks.div_ceil(BITS_PER_BLOCK as u32);
       let journal_start = 1;
       let journal_blocks = JOURNAL_BLOCKS;
       let inode_bitmap_blocks_start = journal_blocks + journal_start;
       let data_bitmap_blocks_start = inode_bitmap_blocks_start + inode_bitmap_blocks;
       let inode_table_start = data_bitmap_blocks_start + data_bitmap_blocks;
       let data_table_start = inode_table_start + inode_table_blocks;

       if data_table_start >= total_blocks { return Err(FsError::EINVAL)}

       let data_table_blocks = total_blocks - data_table_start;

       Ok( Layout { total_blocks, total_inodes, inode_table_blocks, data_table_blocks, inode_bitmap_blocks, data_bitmap_blocks, inode_bitmap_blocks_start, data_bitmap_blocks_start, data_table_start, inode_table_start, journal_start, journal_blocks })
    }
}


#[cfg(test)]
mod tests{
    use super::*;
    const MIB: u64 = 1024 * 1024;

    #[test]
    fn return_coreect_values(){
        let image = Layout::compute(64 * MIB, None).unwrap();

        assert_eq!(image.total_blocks, 16384);
        assert_eq!(image.total_inodes, 4096);
        assert_eq!(image.inode_table_blocks, 128);
        assert_eq!(image.inode_bitmap_blocks, 1);
        assert_eq!(image.data_bitmap_blocks, 1);
        assert_eq!(image.inode_table_start, 259);
        assert_eq!(image.data_table_start, 387);
        assert_eq!(image.data_table_blocks + image.data_table_start, image.total_blocks);
    }

    #[test]
    fn return_corect_inode_quantity(){
        let image = Layout::compute(64 * MIB, Some(100)).unwrap();

        assert_eq!(image.inode_table_blocks, 4);
        assert_eq!(image.total_inodes, 128);
    }

    #[test]
    fn layout_without_holes(){
        let image = Layout::compute(64 * MIB, None).unwrap();

        assert_eq!(image.inode_bitmap_blocks_start, image.journal_blocks + image.journal_start);
        assert_eq!(image.data_bitmap_blocks_start, image.inode_bitmap_blocks_start + image.inode_bitmap_blocks);
        assert_eq!(image.inode_table_start, image.data_bitmap_blocks_start + image.data_bitmap_blocks);
        assert_eq!(image.data_table_start, image.inode_table_start + image.inode_table_blocks);
    }

    #[test]
    fn image_too_small(){
        assert_eq!(Layout::compute(MIB, None), Err(FsError::EINVAL));
        assert_eq!(Layout::compute(0, None), Err(FsError::EINVAL));
        assert_eq!(Layout::compute(6*MIB, Some(0)), Err(FsError::EINVAL));
    }
}