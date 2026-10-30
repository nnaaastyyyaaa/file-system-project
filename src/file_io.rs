use std::cmp::{min, max};
use crate::{error::{FsError, FsResult}, indirection::{free_blocks_from, resolve_block}, inode::{Inode, now_ms}, inode_table, mem::BufferPooll, raw_device::{BLOCK_SIZE, RawDevice}, superblock::Superblock};

const MAX_FILE_SIZE: u64 = 1036 * BLOCK_SIZE as u64;

fn count_shift_bytes_to_read(offset: u64, n:u64, bytes_left:&mut usize, available: usize) -> (usize, usize){
        let shift:usize ;
        let bytes_to:usize ;

        if *bytes_left == available as usize{
            shift = offset as usize - (BLOCK_SIZE  * n as usize);
            bytes_to = min(*bytes_left, BLOCK_SIZE - shift);
        }else{
            shift = 0;
            if *bytes_left > BLOCK_SIZE{
                bytes_to = BLOCK_SIZE;
            }else{
                bytes_to = *bytes_left;
            }
        }

        (bytes_to, shift)
}

pub fn read_file<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, inode: &mut Inode, offset: u64, buff: &mut [u8]) -> FsResult<u64>{
    let need_to_read = buff.len();
    let available_to_read = min(inode.size.saturating_sub(offset), need_to_read as u64);
    if available_to_read == 0 { 
        return Ok(0) 
    }

    let blocks_need_to_read = (offset + available_to_read - 1) / BLOCK_SIZE as u64 - offset/BLOCK_SIZE as u64 + 1;
    let first_block = offset / BLOCK_SIZE as u64;

    let mut bytes_left: usize = available_to_read as usize;
    let mut bytes_read:usize = 0;
    
    for n in first_block..first_block + blocks_need_to_read {

        let block = resolve_block(device, pool, superblock, inode, false, u32::try_from(n).map_err(|_| FsError::CORRUPT)?)?;

        let (bytes_to_read, shift) = count_shift_bytes_to_read(offset, n,&mut bytes_left, available_to_read as usize);

        if block == 0{
            buff[bytes_read..bytes_read + bytes_to_read].fill(0);
        }else{
            let mut buffer = pool.take()?;
            device.read(block, &mut buffer)?;
            buff[bytes_read..bytes_read + bytes_to_read].copy_from_slice(&buffer[shift..shift + bytes_to_read]);
        }

        bytes_left -= bytes_to_read;
        bytes_read += bytes_to_read;
    }

    Ok(bytes_read as u64)
}

fn write_chunk<D: RawDevice>(pool: &BufferPooll, device: &mut D, block: u32, shift: usize, chunk:&[u8] ) -> FsResult<()> {
        let mut buffer = pool.take()?;
        device.read(block, &mut buffer)?;
        buffer[shift..shift + chunk.len()].copy_from_slice(chunk);
        device.write(block, &buffer)?;
        Ok(())
}

pub fn write_file<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, inode: &mut Inode, offset: u64, buff: &[u8]) -> FsResult<u64>{
    let need_to_write = buff.len() as u64;
    if need_to_write == 0 { 
        return Ok(0) 
    }

    let end = offset.checked_add(need_to_write).ok_or(FsError::EFBIG)?;
    if end > MAX_FILE_SIZE {
        return Err(FsError::EFBIG);
    }

    let blocks_need_to_write = (offset + need_to_write - 1) / BLOCK_SIZE as u64 - offset/BLOCK_SIZE as u64 + 1;
    let first_block = offset / BLOCK_SIZE as u64;

    let mut bytes_left: usize = need_to_write as usize;
    let mut bytes_written:usize = 0;
     let mut error: Option<FsError> = None;
    
    for n in first_block..first_block + blocks_need_to_write {

        let block;
        match resolve_block(device, pool, superblock, inode, true, n as u32){
            Ok(b) => {
                block = b;
            },
            Err(e) => {
                error = Some(e);
                break;
            }
        };

        let (bytes_to_write, shift) = count_shift_bytes_to_read( offset, n, &mut bytes_left, need_to_write as usize);

        let chunk = &buff[bytes_written..bytes_written + bytes_to_write];

        if let Err(e) = write_chunk(pool, device, block, shift, chunk) {
            error = Some(e);
            break;
        }

        bytes_left -= bytes_to_write;
        bytes_written += bytes_to_write;
    }

    if bytes_written > 0 {
        inode.mtime = now_ms();
        inode.ctime = now_ms();
        inode.size = max(inode.size, offset + bytes_written as u64);
    }
    inode_table::write(inode, device, superblock, pool)?;

    match error {
        Some(e) if bytes_written == 0 => Err(e),
        _ => Ok(bytes_written as u64),
    }
}

pub fn truncate_file<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, inode: &mut Inode, new_size: u64) -> FsResult<()>{
    if new_size > MAX_FILE_SIZE {
        return  Err(FsError::EFBIG);
    }

    if new_size == inode.size {
        return Ok(())
    }

    if new_size < inode.size {
        let block_to_be_zeroed= new_size.div_ceil(BLOCK_SIZE as u64) as u32;
        let tail_offset = new_size as usize % BLOCK_SIZE;
        
        if tail_offset != 0 {
            let tail_block = (new_size / BLOCK_SIZE as u64) as u32;
            let block = resolve_block(device, pool, superblock, inode, false, tail_block)?;

            if block != 0 {
                let mut buff = pool.take()?;
                device.read(block, &mut buff)?;
                buff[tail_offset..].fill(0);
                device.write(block, &buff)?;
            }
        }
        free_blocks_from(device, pool, superblock, inode, block_to_be_zeroed)?;
    }

    let now = now_ms();
    inode.size = new_size;
    inode.ctime = now;
    inode.mtime = now;
    inode_table::write(inode, device, superblock, pool)?;

    Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    use crate::{alloc::alloc_block, raw_device::{BLOCK_SIZE}, test_utils::{MIB, full_image, inode}};

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8 + 1).collect()
    }

    #[test]
    fn write_then_read_across_block_boundary() {
        let (mut device, pool, mut superblock) = full_image("fio_write_read_boundary", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let data = pattern(100);
        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, 4090, &data).unwrap(), 100);
        assert_eq!(inode.size, 4190);

        let mut out = vec![0u8; 100];
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 4090, &mut out).unwrap(), 100);
        assert_eq!(out, data);
        assert_eq!(pool.in_use(), 0);assert_eq!(pool.in_use(), 0);

    }

    #[test]
    fn dont_read_if_offset_invalid(){
        let (mut device, pool, mut superblock) = full_image("dont_read_if_offset_invalid", 4 *MIB);
        let mut inode = inode(2);
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let mut buff = [0xAA; 4096];

        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 987,&mut buff), Ok(0));
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 1087,&mut buff), Ok(0));
        assert!(buff.iter().all(|&b| b == 0xAA));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn read_no_more_than_file_size(){
        let (mut device, pool, mut superblock) = full_image("read_no_more_than_file_size", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 6000;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let data = pattern(6000);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();

        let mut buff = [0xAA; 1000];
        read_file(&mut device, &pool, &mut superblock, &mut inode, 5500, &mut buff).unwrap();
        assert_eq!(&buff[0..500], &data[5500..]);
        assert!(&buff[500..].iter().all(|&b| b == 0xAA));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn read_hole_in_the_middle_gives_zeros(){
        let (mut device, pool, mut superblock) = full_image("read_hole_in_the_middle", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let first = pattern(BLOCK_SIZE);
        let third: Vec<u8> = pattern(BLOCK_SIZE).iter().map(|b| b + 1).collect();

        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &first).unwrap();
        write_file(&mut device, &pool, &mut superblock, &mut inode, 2 * BLOCK_SIZE as u64, &third).unwrap();
        assert_eq!(inode.size, 3 * BLOCK_SIZE as u64);

        let free_before = superblock.free_blocks;

        let mut buff = [0xAA; 3 * BLOCK_SIZE];
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 0, &mut buff), Ok(3 * BLOCK_SIZE as u64));

        assert_eq!(&buff[..BLOCK_SIZE], &first[..]);
        assert!(buff[BLOCK_SIZE..2 * BLOCK_SIZE].iter().all(|&b| b == 0));
        assert_eq!(&buff[2 * BLOCK_SIZE..], &third[..]);

        assert_eq!(superblock.free_blocks, free_before);
        assert_eq!(inode.direct[1], 0);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn read_into_empty_buffer_returns_zero(){
        let (mut device, pool, mut superblock) = full_image("read_into_empty_buffer", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let data = pattern(100);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();

        let mut buff: [u8; 0] = [];
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 0, &mut buff), Ok(0));
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 10, &mut buff), Ok(0));
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 5000, &mut buff), Ok(0));

        assert_eq!(inode.size, 100);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn works_at_the_boundaries(){
        let (mut device, pool, mut superblock) = full_image("works_at_the_boundaries", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        
        let first = pattern(BLOCK_SIZE);
        let second: Vec<u8> = pattern(BLOCK_SIZE).iter().map(|b| b + 1).collect();

        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &first).unwrap();
        write_file(&mut device, &pool, &mut superblock, &mut inode, BLOCK_SIZE as u64, &second).unwrap();

        let mut buff = [0xAA; 2 * BLOCK_SIZE];

        read_file(&mut device, &pool, &mut superblock, &mut inode, 0, &mut buff).unwrap();

        assert_eq!(&buff[..BLOCK_SIZE], &first[..]);
        assert_eq!(&buff[BLOCK_SIZE..], &second[..]);
    }

    #[test]
    fn write_into_empty_file(){
        let (mut device, pool, mut superblock) = full_image("write_into_empty_file", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.mtime = 0;
        inode.ctime = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let free_before = superblock.free_blocks;
        let data = pattern(100);

        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data), Ok(100));
        assert_eq!(inode.size, 100);
        assert_ne!(inode.mtime, 0);
        assert_ne!(inode.ctime, 0);
        assert_eq!(superblock.free_blocks, free_before - 1);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn write_in_the_middle_of_block_keeps_neighbours(){
        let (mut device, pool, mut superblock) = full_image("write_in_the_middle_of_block", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let data = pattern(BLOCK_SIZE);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();

        let patch = [0xFF; 10];
        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, 100, &patch), Ok(10));

        let mut expected = data.clone();
        expected[100..110].fill(0xFF);

        let mut out = vec![0u8; BLOCK_SIZE];
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 0, &mut out), Ok(BLOCK_SIZE as u64));
        assert_eq!(out, expected);
        assert_eq!(inode.size, BLOCK_SIZE as u64);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn write_at_the_start_does_not_shrink_file(){
        let (mut device, pool, mut superblock) = full_image("write_at_the_start_no_shrink", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let data = pattern(5000);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();
        assert_eq!(inode.size, 5000);

        let patch = [0xFFu8; 10];
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &patch).unwrap();
        assert_eq!(inode.size, 5000);
    }

    #[test]
    fn write_after_the_end_leaves_zeros_between(){
        let (mut device, pool, mut superblock) = full_image("write_after_the_end", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let first = pattern(100);
        let last = [0xFFu8; 10];
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &first).unwrap();
        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, 10000, &last), Ok(10));
        assert_eq!(inode.size, 10010);

        let mut gap = vec![0xAAu8; 9900];
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 100, &mut gap), Ok(9900));
        assert!(gap.iter().all(|&b| b == 0));
    }

    #[test]
    fn write_across_direct_and_indirect_boundary(){
        let (mut device, pool, mut superblock) = full_image("write_across_direct_indirect", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let free_before = superblock.free_blocks;
        let offset = 12 * BLOCK_SIZE as u64 - 10;
        let data = pattern(100);

        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, offset, &data), Ok(100));
        assert_ne!(inode.single_indirect, 0);
        assert_eq!(inode.size, offset + 100);
        assert_eq!(superblock.free_blocks, free_before - 3);
    }

    #[test]
    fn write_beyond_max_file_size_gives_efbig(){
        let (mut device, pool, mut superblock) = full_image("write_beyond_max_file_size", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.mtime = 7;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let free_before = superblock.free_blocks;

        let data = pattern(20);
        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, MAX_FILE_SIZE - 10, &data), Err(FsError::EFBIG));
        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, u64::MAX, &data), Err(FsError::EFBIG));

        assert_eq!(superblock.free_blocks, free_before);
        assert_eq!(inode.size, 0);
        assert_eq!(inode.mtime, 7);

        let data = pattern(10);
        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, MAX_FILE_SIZE - 10, &data), Ok(10));
        assert_eq!(inode.size, MAX_FILE_SIZE);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn write_on_full_disk_gives_enospc(){
        let (mut device, pool, mut superblock) = full_image("write_on_full_disk", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.mtime = 7;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let free_blocks = superblock.free_blocks;
        for _ in 0..free_blocks {
            alloc_block(&mut superblock, &pool, &mut device).unwrap();
        }

        let data = pattern(100);
        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data), Err(FsError::ENOSPC));
        assert_eq!(inode.size, 0);
        assert_eq!(inode.mtime, 7);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn partial_write_when_disk_runs_out(){
        let (mut device, pool, mut superblock) = full_image("partial_write", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.mtime = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let free_blocks = superblock.free_blocks;
        for _ in 0..free_blocks - 1 {
            alloc_block(&mut superblock, &pool, &mut device).unwrap();
        }

        let data = pattern(5000);
        assert_eq!(write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data), Ok(BLOCK_SIZE as u64));
        assert_eq!(inode.size, BLOCK_SIZE as u64);
        assert_ne!(inode.mtime, 0);
        assert_ne!(inode.direct[0], 0);
        assert_eq!(inode.direct[1], 0);
    }

    #[test]
    fn truncate_shrink_then_grow_gives_zero_tail(){
        let (mut device, pool, mut superblock) = full_image("truncate_shrink_then_grow", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let data = pattern(5000);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();

        truncate_file(&mut device, &pool, &mut superblock, &mut inode, 4100).unwrap();
        assert_eq!(inode.size, 4100);
        truncate_file(&mut device, &pool, &mut superblock, &mut inode, 5000).unwrap();
        assert_eq!(inode.size, 5000);

        let mut out = vec![0xAAu8; 5000];
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 0, &mut out), Ok(5000));
        assert_eq!(&out[..4100], &data[..4100]);
        assert!(out[4100..].iter().all(|&b| b == 0));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn truncate_to_block_boundary_frees_blocks(){
        let (mut device, pool, mut superblock) = full_image("truncate_to_block_boundary", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let data = pattern(5000);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();
        let free_before = superblock.free_blocks;

        truncate_file(&mut device, &pool, &mut superblock, &mut inode, 4096).unwrap();
        assert_eq!(inode.size, 4096);
        assert_eq!(superblock.free_blocks, free_before + 1);
        assert_eq!(inode.direct[1], 0);

        let mut out = vec![0u8; BLOCK_SIZE];
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 0, &mut out), Ok(BLOCK_SIZE as u64));
        assert_eq!(out, &data[..BLOCK_SIZE]);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn truncate_to_zero_frees_everything(){
        let (mut device, pool, mut superblock) = full_image("truncate_to_zero", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let free_start = superblock.free_blocks;
        let data = pattern(5000);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();
        assert_eq!(superblock.free_blocks, free_start - 2);

        truncate_file(&mut device, &pool, &mut superblock, &mut inode, 0).unwrap();
        assert_eq!(inode.size, 0);
        assert_eq!(superblock.free_blocks, free_start);
        assert_eq!(inode.direct, [0; 12]);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn truncate_with_hole_in_tail_does_not_touch_superblock(){
        let (mut device, pool, mut superblock) = full_image("truncate_hole_in_tail", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let data = pattern(BLOCK_SIZE);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();
        truncate_file(&mut device, &pool, &mut superblock, &mut inode, 5000).unwrap();
             assert_eq!(inode.size, 5000);
        assert_eq!(inode.direct[1], 0);

        assert_eq!(truncate_file(&mut device, &pool, &mut superblock, &mut inode, 4100), Ok(()));
        assert_eq!(inode.size, 4100);

        let mut buff = pool.take().unwrap();
        device.read(0, &mut buff).unwrap();
        let on_disk = Superblock::decode(&buff).unwrap();
        assert_eq!(on_disk, superblock);

        drop(buff);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn truncate_to_same_size_changes_nothing(){
        let (mut device, pool, mut superblock) = full_image("truncate_same_size", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let data = pattern(100);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();
        inode.mtime = 5;
        inode.ctime = 5;
        let free_before = superblock.free_blocks;

        assert_eq!(truncate_file(&mut device, &pool, &mut superblock, &mut inode, 100), Ok(()));
        assert_eq!(inode.size, 100);
        assert_eq!(inode.mtime, 5);
        assert_eq!(inode.ctime, 5);
        assert_eq!(superblock.free_blocks, free_before);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn truncate_beyond_max_file_size_gives_efbig(){
        let (mut device, pool, mut superblock) = full_image("truncate_beyond_max", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.mtime = 5;
        inode.ctime = 5;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let free_before = superblock.free_blocks;

        assert_eq!(truncate_file(&mut device, &pool, &mut superblock, &mut inode, MAX_FILE_SIZE + 1), Err(FsError::EFBIG));
        assert_eq!(inode.size, 0);
        assert_eq!(inode.mtime, 5);
        assert_eq!(inode.ctime, 5);
        assert_eq!(superblock.free_blocks, free_before);

        assert_eq!(truncate_file(&mut device, &pool, &mut superblock, &mut inode, MAX_FILE_SIZE), Ok(()));
        assert_eq!(inode.size, MAX_FILE_SIZE);
        assert_eq!(superblock.free_blocks, free_before);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn truncate_across_indirect_boundary_frees_pointer_block(){
        let (mut device, pool, mut superblock) = full_image("truncate_across_indirect", 4 *MIB);
        let mut inode = inode(2);
        inode.size = 0;
        inode.direct = [0; 12];
        inode.single_indirect = 0;

        let free_start = superblock.free_blocks;
        let data = pattern(20 * BLOCK_SIZE);
        write_file(&mut device, &pool, &mut superblock, &mut inode, 0, &data).unwrap();
        assert_eq!(superblock.free_blocks, free_start - 21);
        assert_ne!(inode.single_indirect, 0);

        truncate_file(&mut device, &pool, &mut superblock, &mut inode, 5 * BLOCK_SIZE as u64).unwrap();
        assert_eq!(inode.size, 5 * BLOCK_SIZE as u64);
        assert_eq!(inode.single_indirect, 0);
        assert!(inode.direct[5..].iter().all(|&b| b == 0));
        assert!(inode.direct[..5].iter().all(|&b| b != 0));
        assert_eq!(superblock.free_blocks, free_start - 5);

        let mut out = vec![0u8; 5 * BLOCK_SIZE];
        assert_eq!(read_file(&mut device, &pool, &mut superblock, &mut inode, 0, &mut out), Ok(5 * BLOCK_SIZE as u64));
        assert_eq!(out, &data[..5 * BLOCK_SIZE]);
        assert_eq!(pool.in_use(), 0);
    }
}