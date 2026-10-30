use crate::{alloc::{alloc_block, alloc_inode}, bitmap::{BITS_PER_BLOCK, mark_tail_used}, dirent::{ENTRY_SIZE, encode_entry}, error::{FsError, FsResult}, inode::{Inode, Type, now_ms}, inode_table, layout::Layout, mem::BufferPooll, raw_device::{BLOCK_SIZE, RawDevice}, superblock::{ROOT_INODE, Superblock}};

fn init_bitmap<D: RawDevice>(start: u32, blocks: u32, valid_bits: usize, pool: &BufferPooll, device: &mut D) -> FsResult<()>{
    let full_blocks = valid_bits / BITS_PER_BLOCK ;
    let bits_in_not_full = valid_bits % BITS_PER_BLOCK;
    for n in 0..blocks{
        let mut buff = pool.take()?;
        if  n < full_blocks as u32 {
        }else if n == full_blocks as u32{
            mark_tail_used(&mut buff, bits_in_not_full);
        }else if n > full_blocks as u32{
            mark_tail_used(&mut buff, 0);
        }
        device.write(start + n, &mut buff)?;
    }
    Ok(())
}

pub fn mkfs<D: RawDevice>(pool: &BufferPooll, device: &mut D, inode_count: Option<u32>) -> FsResult<Superblock>{
    let size = device.total_blocks() as u64 * BLOCK_SIZE as u64;
    let layout = Layout::compute(size, inode_count)?;
    let mut superblock = Superblock::new(&layout);

    let buff = pool.take()?;
    for n in 1..superblock.data_table_start{
        device.write(n, &buff)?;
    }
    drop(buff);

    init_bitmap(superblock.inode_bitmap_blocks_start, superblock.inode_bitmap_blocks, superblock.total_inodes as usize, pool, device)?;
    init_bitmap(superblock.data_bitmap_blocks_start, superblock.data_bitmap_blocks, superblock.data_table_blocks as usize, pool, device)?;

    let mut buff = pool.take()?;
    superblock.encode(&mut buff);
    device.write(0, &buff)?;
    drop(buff);

    let bit = alloc_inode(&mut superblock, pool, device)?;
    if bit != ROOT_INODE { 
        return Err(FsError::CORRUPT) 
    }

    let block = alloc_block(&mut superblock, pool, device)?;
    let mut buff = pool.take()?;

    encode_entry(&mut buff[0..ENTRY_SIZE], bit, ".")?;
    encode_entry(&mut buff[ENTRY_SIZE..2 * ENTRY_SIZE], bit, "..")?;

    device.write(block, &buff)?;

    let mut first_inode = Inode::empty(bit);
    first_inode.inode_type = Type::Dir;
    first_inode.nlink = 2;
    first_inode.mode = 0o777;
    first_inode.ctime = now_ms();
    first_inode.mtime = now_ms();
    first_inode.size = BLOCK_SIZE as u64;
    first_inode.direct[0] = block;

    inode_table::write(&first_inode, device, &superblock, pool)?;


    Ok(superblock)
}

#[cfg(test)]
mod tests {
    use crate::{bitmap::{count_zero_bits, get_bit}, dirent::decode_entry, raw_device::FileDevice, test_utils::{MIB, temp_image}};
    use super::*;

    #[test]
    fn superblock_returned_as_valid_after_mkfs(){
        let image = temp_image("superblock_returned_as_valid_after_mkfs");
        let mut file_device = FileDevice::create(image.to_str().unwrap(), 64 * MIB).unwrap();
        let pool = BufferPooll::new();
        let superblock = mkfs(&pool, &mut file_device, None).unwrap();

        assert_eq!(superblock.free_inodes , superblock.total_inodes - 1);
        assert_eq!(superblock.free_blocks, superblock.data_table_blocks - 1);
        assert_eq!(superblock.clean_unmount, 1);
        assert_eq!(pool.in_use(), 0)
    }

    #[test]
    fn root_dir_has_dot_and_dotdot(){
        let image = temp_image("root_dir_has_dot_and_dotdot");
        let mut file_device = FileDevice::create(image.to_str().unwrap(), 64 * MIB).unwrap();
        let pool = BufferPooll::new();
        let superblock = mkfs(&pool, &mut file_device, None).unwrap();
        let inode = inode_table::read(1, &mut file_device, &superblock, &pool).unwrap();

        let mut buff = pool.take().unwrap();
        file_device.read(inode.direct[0], &mut buff).unwrap();

        let first_slot = decode_entry(&buff[0..ENTRY_SIZE]).unwrap();
        let second_slot = decode_entry(&buff[ENTRY_SIZE..ENTRY_SIZE * 2]).unwrap();

        drop(buff);
        assert_eq!(first_slot, Some((1, ".".to_string())));
        assert_eq!(second_slot, Some((1, "..".to_string())));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn bitmap_tails_are_marked_used(){
        let image = temp_image("bitmap_tails_are_marked_used");
        let mut file_device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let superblock = mkfs(&pool, &mut file_device, Some(100)).unwrap();

        let mut buff = pool.take().unwrap();
        file_device.read(superblock.inode_bitmap_blocks_start, &mut buff).unwrap();

        assert_eq!(count_zero_bits(&buff) as u32, superblock.total_inodes - 1);
        assert!(!get_bit(&buff, (superblock.total_inodes - 1 )as usize));
        assert!(get_bit(&buff, (BITS_PER_BLOCK - 1 )as usize));
        assert!(get_bit(&buff, (superblock.total_inodes )as usize));
        
        drop(buff);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn mkfs_resets_old(){
        {
            let image = temp_image("mkfs_resets_old");
            let mut file_device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
            let pool = BufferPooll::new();
            let superblock = mkfs(&pool, &mut file_device, Some(100)).unwrap();

            let mut buff = pool.take().unwrap();
            buff.fill(0xff);

            file_device.write(superblock.data_table_start + 1, &buff).unwrap();
        }

        let image = temp_image("mkfs_test");
        let mut file_device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let superblock = mkfs(&pool, &mut file_device, Some(100)).unwrap();
        let mut buff = pool.take().unwrap();
        file_device.read(superblock.data_table_start + 1, &mut buff).unwrap();

        assert!(buff.iter().all(|&b| b == 0));
    }

    #[test]
    fn too_small_image_returns_err(){
        let image = temp_image("too_small_image_returns_err");
        let mut file_device = FileDevice::create(image.to_str().unwrap(), MIB).unwrap();
        let pool = BufferPooll::new();
        assert_eq!(mkfs(&pool, &mut file_device, Some(100)), Err(FsError::EINVAL));
    }
}