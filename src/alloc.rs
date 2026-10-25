use crate::{bitmap::{BITS_PER_BLOCK, find_zero_bit, get_bit, set_bit}, error::{FsError, FsResult}, mem::BufferPooll, raw_device::{FileDevice, RawDevice}, superblock::{self, Superblock}};

fn alloc_bit<D: RawDevice>(bitmap_start: u32, bitmap_blocks:u32, cursor: &mut u32, pool: &BufferPooll, device: &mut D) -> FsResult<u32>{
    if bitmap_blocks <= 0 { return Err(FsError::EINVAL)};

    let first_block = *cursor % bitmap_blocks;
    let mut buff = pool.take()?;
    for n in 0..bitmap_blocks{
        let current_block = (first_block + n) % bitmap_blocks;
        device.read(bitmap_start + current_block, &mut buff)?;
        match find_zero_bit(&buff, 0){
            Some(bit) => {
                set_bit(&mut buff, bit, true);
                device.write(bitmap_start + current_block, &buff)?;
                *cursor = current_block;
                return Ok(current_block * BITS_PER_BLOCK as u32 + bit as u32 )
            },
            None => continue,
        }
    }
    drop(buff);
    Err(FsError::ENOSPC)
}

fn free_bit<D: RawDevice>(bitmap_start: u32, bitmap_blocks:u32, bit: u32, pool: &BufferPooll, device: &mut D) -> FsResult<()>{
    let block = (bit / BITS_PER_BLOCK as u32) as u32;
    if block >= bitmap_blocks { return Err(FsError::EINVAL) };

    let shift = (bit % BITS_PER_BLOCK as u32) as usize;

    let mut buff = pool.take()?;
    device.read(bitmap_start + block, &mut buff)?;
    let is_free = get_bit(&buff, shift);
    if !is_free { return Err(FsError::CORRUPT) };

    set_bit(&mut buff, shift, false);
    device.write(bitmap_start + block, &buff)?;

    drop(buff);
    Ok(())
}

pub fn alloc_inode<D: RawDevice>(superblock: &mut Superblock, pool: &BufferPooll, device: &mut D)-> FsResult<u32>{
    let bit = alloc_bit(superblock.inode_bitmap_blocks_start, superblock.inode_bitmap_blocks, &mut superblock.inode_cursor, pool, device)?;
    superblock.free_inodes = superblock.free_inodes.checked_sub(1).ok_or(FsError::CORRUPT)?;
    
    let mut buff = pool.take()?;
    superblock.encode(&mut buff);

    device.write(0, &buff)?;
    drop(buff);

    Ok(bit + 1)
}

pub fn alloc_block<D: RawDevice>(superblock: &mut Superblock, pool: &BufferPooll, device: &mut D)-> FsResult<u32>{
    if superblock.free_blocks == 0 { return Err(FsError::ENOSPC) }
    let bit = alloc_bit(superblock.data_bitmap_blocks_start, superblock.data_bitmap_blocks, &mut superblock.block_cursor, pool, device)?;
    superblock.free_blocks = superblock.free_blocks.checked_sub(1).ok_or(FsError::CORRUPT)?;

    let shift = bit + superblock.data_table_start;
    
    let mut zeroed = pool.take()?;
    zeroed.fill(0);
    device.write(shift as u32, &zeroed)?;
    drop(zeroed);

    let mut buff = pool.take()?;
    superblock.encode(&mut buff);
    device.write(0, &buff)?;
    drop(buff);

    Ok(shift)
}

pub fn free_inode<D: RawDevice>(superblock: &mut Superblock, bit: u32, pool:&BufferPooll, device: &mut D) -> FsResult<()>{
    if bit < 1 || bit > superblock.total_inodes { return Err(FsError::EINVAL) };
    free_bit(superblock.inode_bitmap_blocks_start, superblock.inode_bitmap_blocks, bit - 1, pool, device)?;
    superblock.free_inodes += 1;
    let mut buff = pool.take()?;
    superblock.encode(&mut buff);
    device.write(0, & mut buff);
    drop(buff);
    Ok(())
}

pub fn free_block<D: RawDevice>(superblock: &mut Superblock, bit: u32, pool:&BufferPooll, device: &mut D) -> FsResult<()>{
    if bit < superblock.data_table_start || bit > superblock.total_blocks { return Err(FsError::EINVAL) };

    free_bit(superblock.data_bitmap_blocks_start, superblock.data_bitmap_blocks, bit - superblock.data_table_start, pool, device)?;
    superblock.free_blocks += 1;
    let mut buff = pool.take()?;
    superblock.encode(&mut buff);
    device.write(0, & mut buff);
    drop(buff);
    Ok(())
}


#[cfg(test)]
mod test{
    use crate::{test_utils::{MIB, full_image}};
    use super::*;

    #[test]
    fn first_inode_returned_correctly() {
        let (mut img, pool, mut sb) = full_image("alloc_first_inode", 4 * MIB);

        assert_eq!(alloc_inode(&mut sb, &pool, &mut img), Ok(2));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn first_block_returned_correctly() {
        let (mut img, pool, mut sb) = full_image("alloc_first_block", 4 * MIB);

        assert_eq!(alloc_block(&mut sb, &pool, &mut img), Ok(sb.data_table_start + 1));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn alloc_return_released_block() {
        let (mut img, pool, mut sb) = full_image("released_block", 4 * MIB);
        let bit = alloc_block(&mut sb, &pool, &mut img).unwrap();
        free_block(&mut sb, bit, &pool, &mut img).unwrap();

        assert_eq!(alloc_block(&mut sb, &pool, &mut img).unwrap(), bit);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn alloc_return_released_inode() {
        let (mut img, pool, mut sb) = full_image("released_inode", 4 * MIB);
        let bit = alloc_inode(&mut sb, &pool, &mut img).unwrap();
        free_inode(&mut sb, bit, &pool, &mut img).unwrap();

        assert_eq!(alloc_inode(&mut sb, &pool, &mut img).unwrap(), bit);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_inode_bitmap_returns_enospc() {
        let (mut img, pool, mut sb) = full_image("inode_bitmap_returns_enospc", 4 * MIB);
        for n in 0..sb.total_inodes - 1{
            println!("n = {}", n);
            alloc_inode(&mut sb, &pool, &mut img).unwrap();
        }
        assert_eq!(alloc_inode(&mut sb, &pool, &mut img), Err(FsError::ENOSPC));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_block_bitmap_returns_enospc() {
        let (mut img, pool, mut sb) = full_image("block_bitmap_returns_enospc", 4 * MIB);
        let free_blocks = sb.free_blocks;
        for n in 0..free_blocks {
            alloc_block(&mut sb, &pool, &mut img).unwrap();
        }
        assert_eq!(alloc_block(&mut sb, &pool, &mut img), Err(FsError::ENOSPC));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn block_zeroed_when_allocated() {
        let (mut img, pool, mut sb) = full_image("zeroed_when_allocated", 4 * MIB);
        let mut bit = alloc_block(&mut sb, &pool, &mut img).unwrap();
        free_block(&mut sb, bit, &pool, &mut img).unwrap();
        let mut buff = pool.take().unwrap();
        buff.fill(0xff);
        img.write(bit, &buff).unwrap();
        bit = alloc_block(&mut sb, &pool, &mut img).unwrap();

        buff = pool.take().unwrap();
        img.read(bit, &mut buff);

        assert!(buff.iter().all(|&b| b == 0));
        drop(buff);
        assert_eq!(pool.in_use(), 0);
    }


    #[test]
    fn double_block_free_returns_corrupt() {
        let (mut img, pool, mut sb) = full_image("double_block_free", 4 * MIB);
        let mut bit = alloc_block(&mut sb, &pool, &mut img).unwrap();
        free_block(&mut sb, bit, &pool, &mut img).unwrap();

        assert_eq!(free_block(&mut sb, bit, &pool, &mut img), Err(FsError::CORRUPT));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn double_inode_free_returns_corrupt() {
        let (mut img, pool, mut sb) = full_image("double_inode_free", 4 * MIB);
        let mut bit = alloc_inode(&mut sb, &pool, &mut img).unwrap();
        free_inode(&mut sb, bit, &pool, &mut img).unwrap();

        assert_eq!(free_inode(&mut sb, bit, &pool, &mut img), Err(FsError::CORRUPT));
        assert_eq!(pool.in_use(), 0);
    }

}
