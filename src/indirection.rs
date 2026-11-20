use crate::{alloc::{alloc_block, free_block}, codec::{get_u32, put_u32}, error::{FsError, FsResult}, inode::Inode, mem::BufferPooll, raw_device::{BLOCK_SIZE, RawDevice}, superblock::Superblock};

const POINTERS_PER_BLOCK: u32 = BLOCK_SIZE as u32 / 4;
const SINGLE_INDIRECT_END: u32 = 12 + POINTERS_PER_BLOCK;
pub const MAX_BLOCKS: u32 = 12 + 1024 + 1024 * 1024;

pub fn resolve_block<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, inode: &mut Inode, allocate: bool, block: u32) -> FsResult<u32>{
    match block {
        0..12 => {
          let n = inode.direct[block as usize];

        if n == 0 && !allocate { 
            return Ok(0) 
        }

        if n == 0 && allocate {
            let allocated = alloc_block(superblock, pool,  device)?;
            inode.direct[block as usize] = allocated;
            return  Ok(allocated);
        }
          
          return  Ok(n);
        },

        12..SINGLE_INDIRECT_END => {
            let slot = (block - 12) as usize;

            if inode.single_indirect == 0 {
                if !allocate {
                    return Ok(0);
                }
                inode.single_indirect = alloc_block(superblock, pool, device)?;
            }

            let mut buff = pool.take()?;
            device.read(inode.single_indirect, &mut buff)?;

            let current = get_u32(&buff[..], slot * 4);
            if current != 0 {
                return Ok(current);
            }

            if !allocate {
                return Ok(0);
            }

            let data_block = alloc_block(superblock, pool, device)?;
            put_u32(&mut buff[..], slot * 4, data_block);
            device.write(inode.single_indirect, &buff)?;
            Ok(data_block)
        },

        SINGLE_INDIRECT_END..MAX_BLOCKS => {
            let mut num_double = inode.double_indirect;
            if num_double == 0 {
                if !allocate {
                    return Ok(0);
                }
                inode.double_indirect = alloc_block(superblock, pool, device)?;
                num_double = inode.double_indirect;
            }

            let slot_in_double = (block - SINGLE_INDIRECT_END) / POINTERS_PER_BLOCK;
            let shift_in_slot = (block - SINGLE_INDIRECT_END) % POINTERS_PER_BLOCK;

            let mut double = pool.take()?;
            device.read(num_double, &mut double)?;

            let mut num_single = get_u32(&double[..], slot_in_double as usize * 4);

            if num_single == 0 {
                if !allocate {
                    return Ok(0);
                }
                num_single = alloc_block(superblock, pool, device)?;
                put_u32(&mut double[..], slot_in_double as usize * 4, num_single);
                device.write(num_double, &double)?;
            }

            let mut buff = pool.take()?;
            device.read(num_single, &mut buff)?;

            let data = get_u32(&buff[..], shift_in_slot as usize * 4);
            if data != 0 {
                return Ok(data);
            }
            if !allocate {
                return Ok(0);
            }

            let new_block = alloc_block(superblock, pool, device)?;
            put_u32(&mut buff[..], shift_in_slot as usize * 4, new_block);
            device.write(num_single, &buff)?;
            Ok(new_block)
        },
        _ => {
            if allocate {
                Err(FsError::EFBIG)
            } else {
                Ok(0)
            }
        }
    }
}

pub fn free_blocks_from<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, inode: &mut Inode,  from: u32) -> FsResult<()>{
    if from >= MAX_BLOCKS {
        return Ok(())
    }

    if from < 12 {
        for n in from..12{
            let block = inode.direct[n as usize];

            if block == 0 {
                continue;
            }

            free_block(superblock, block, pool, device)?;
            inode.direct[n as usize] = 0;
        }
    }

    if from < SINGLE_INDIRECT_END && inode.single_indirect != 0 {
        let ptr = inode.single_indirect;
        let slot = from.saturating_sub(12);
        
        let mut buff = pool.take()?;
        device.read(ptr, &mut buff)?;
    
        for n in slot..POINTERS_PER_BLOCK {
            let data_block  = get_u32(&buff[..], (n * 4) as usize);
    
            if data_block == 0 {
                continue;
            }
    
            free_block(superblock, data_block, pool, device)?;
            put_u32(&mut buff[..], (n * 4) as usize, 0);
        }
    
        device.write(ptr, &buff)?;

        if from <= 12 {
            free_block(superblock, inode.single_indirect, pool, device)?;
            inode.single_indirect = 0;
        }
    }

    let ptr = inode.double_indirect;

    if ptr == 0 {
        return Ok(());
    }

    let slot_in_double = (from.saturating_sub(SINGLE_INDIRECT_END)) / POINTERS_PER_BLOCK;
    let shift_in_slot = (from.saturating_sub(SINGLE_INDIRECT_END)) % POINTERS_PER_BLOCK;

    let mut buff = pool.take()?;
    device.read(ptr, &mut buff)?;

    for n in slot_in_double..POINTERS_PER_BLOCK {
        let num_single = get_u32(&buff[..], n as usize * 4);

        if num_single == 0 {
            continue;
        }

        let mut single_buff = pool.take()?;
        device.read(num_single, &mut single_buff)?;

        let start;
        if n == slot_in_double {
            start = shift_in_slot;
        } else {
            start = 0;
        }

        for k in start..POINTERS_PER_BLOCK{
            let data_block = get_u32(&single_buff[..], k as usize * 4);

            if data_block == 0 {
                continue;
            }

            free_block(superblock, data_block, pool, device)?;
            put_u32(&mut single_buff[..], k as usize * 4, 0);
        }

        if start == 0 {
            free_block(superblock, num_single, pool, device)?;
            put_u32(&mut buff[..], n as usize * 4, 0);
        }else{
            device.write(num_single, &single_buff)?;
        }
    }
    device.write(ptr, &buff)?;

    if from <= SINGLE_INDIRECT_END {
        free_block(superblock, inode.double_indirect, pool, device)?;
        inode.double_indirect = 0;
    }

    Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    use std::collections::HashSet;

use crate::{alloc::alloc_block, codec::get_u32, error::FsError, indirection::{free_blocks_from, resolve_block}, raw_device::RawDevice, test_utils::{MIB, full_image, inode}};

    #[test]
    fn trying_to_read_hole_do_nothing(){
        let (mut file_device, pool, mut superblock) = full_image("check_reading_hole", 4*MIB);
        let mut inode = inode(2);

        for n in 8..12{
            inode.direct[n] = 0;
        }

        let n = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 10).unwrap();

        assert_eq!(n, 0);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn direct_allocate_works(){
        let (mut file_device, pool, mut superblock) = full_image("direct_allocate", 4*MIB);
        let mut inode = inode(2);

        for n in 0..12{
            inode.direct[n] = 0;
        }

        let n_blocks = superblock.free_blocks;

        let n = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 0).unwrap();
        assert!(n > superblock.data_table_start);
        assert_eq!(inode.direct[0], n);
        assert_eq!(superblock.free_blocks, n_blocks - 1);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn allocate_same_block_twice_works_correctly(){
        let (mut file_device, pool, mut superblock) = full_image("allocate_same_block_twice", 4*MIB);
        let mut inode = inode(2);

        for n in 0..12{
            inode.direct[n] = 0;
        }

        let n_blocks = superblock.free_blocks;
        let n = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 0).unwrap();
        let b = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 0).unwrap();

        assert_eq!(n, b);
        assert_eq!(inode.direct[0], n);
        assert_eq!(superblock.free_blocks, n_blocks - 1);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn allocate_write_then_read_same_data(){
        let (mut file_device, pool, mut superblock) = full_image("allocate_write_then_read", 4*MIB);
        let mut inode = inode(2);

        for n in 0..12{
            inode.direct[n] = 0;
        }

        let created = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 0).unwrap();
        assert_eq!(created, resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 0).unwrap() );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn different_logical_blocks_get_different_physical() {
        let (mut file_device, pool, mut superblock) = full_image("different_logical_blocks", 4*MIB);
        let mut inode = inode(2);

        let mut blocks = Vec::new();
        for n in 0..12{
            inode.direct[n] = 0;
            blocks.push(resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, n as u32).unwrap());
        }

        let unique: HashSet<u32> = blocks.iter().copied().collect();
        assert_eq!(blocks.iter().len(), unique.iter().len());
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn indirection_first_allocate_creates_two_new_blocks() {
        let (mut file_device, pool, mut superblock) = full_image("indirection_first_allocate", 4*MIB);
        let mut inode = inode(2);

        let n_blocks = superblock.free_blocks;

        resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 12).unwrap();
        assert_eq!(superblock.free_blocks, n_blocks - 2);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn indirection_with_existing_single_indirect_creates_one_new_block() {
        let (mut file_device, pool, mut superblock) = full_image("indirection_with_existing_single_indirect", 4*MIB);
        let mut inode = inode(2);

        
        resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 12).unwrap();
        let n_blocks = superblock.free_blocks;
        resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 13).unwrap();

        assert_eq!(superblock.free_blocks, n_blocks - 1);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn allocate_creates_blocks_on_disk() {
        let (mut file_device, pool, mut superblock) = full_image("allocate_creates_blocks_on_disk", 4*MIB);
        let mut inode = inode(2);

        let n = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 12).unwrap();
        let mut buff = pool.take().unwrap();
        file_device.read(inode.single_indirect, &mut buff).unwrap();

        assert_eq!(get_u32(&buff[..], 0), n);

        drop(buff);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn allocate_in_hole_indirect_left_both_sides_free() {
        let (mut file_device, pool, mut superblock) = full_image("allocate_in_hole_indirect", 4*MIB);
        let mut inode = inode(2);

        let a = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 15).unwrap();
        let b = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 14).unwrap();
        let c = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 16).unwrap();

        assert!(a > superblock.data_table_start);
        assert_eq!(b, 0);
        assert_eq!(c, 0);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn boundaries_works_correctly() {
        let (mut file_device, pool, mut superblock) = full_image("boundaries_works", 4*MIB);
        let mut inode = inode(2);

        inode.single_indirect = 0;
        for n in 0..12{
            inode.direct[n] = 0;
        }

        let n_blocks = superblock.free_blocks;

        resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 11).unwrap();
        assert_eq!(superblock.free_blocks, n_blocks - 1);

        resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 12).unwrap();
        assert_eq!(superblock.free_blocks, n_blocks - 3);

        resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 1035).unwrap();
        assert_eq!(superblock.free_blocks, n_blocks - 4);

        let n = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 1036).unwrap();
        assert_eq!(n, 0);
        assert_eq!(resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, MAX_BLOCKS), Err(FsError::EFBIG));

        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn out_of_space_propagates(){
        let (mut file_device, pool, mut superblock) = full_image("out_of_space", 4*MIB);
        let mut inode = inode(2);

        let free_blocks = superblock.free_blocks;
        for _ in 0..free_blocks {
            alloc_block(&mut superblock, &pool, &mut file_device).unwrap();
        }

        assert_eq!(resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 15), Err(FsError::ENOSPC));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_free_blocks_from_works(){
        let (mut file_device, pool, mut superblock) = full_image("full_free_blocks", 4*MIB);
        let mut inode = inode(2);

        let n_blocks = superblock.free_blocks;
        for n in 0..12{
            inode.direct[n] = 0;
            resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, n as u32).unwrap();
            resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, (n + 12) as u32).unwrap();
        }

        free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 0).unwrap();
        assert_eq!(superblock.free_blocks, n_blocks);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn free_from_13_dont_touch_previous_blocks(){
        let (mut file_device, pool, mut superblock) = full_image("ree_from_13", 4*MIB);
        let mut inode = inode(2);

        for n in 0..12{
            inode.direct[n] = 0;
            resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, n as u32).unwrap();
            resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, (n + 12) as u32).unwrap();
        }

        free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 13).unwrap();
        let n = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 12 as u32).unwrap();
        assert!(n > superblock.data_table_start);

        let b = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 11 as u32).unwrap();
        assert!(b > superblock.data_table_start);

        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn free_from_12_drops_single_indirect(){
        let (mut file_device, pool, mut superblock) = full_image("free_from_12", 4*MIB);
        let mut inode = inode(2);

        resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 12 as u32).unwrap();
        resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 13 as u32).unwrap();

        free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 12).unwrap();
        assert_eq!(inode.single_indirect, 0);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn free_from_5_drops_blocks_correctly(){
        let (mut file_device, pool, mut superblock) = full_image("ree_from_5", 4*MIB);
        let mut inode = inode(2);

        for n in 0..12{
            inode.direct[n] = 0;
            resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, n as u32).unwrap();
            resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, (n + 12) as u32).unwrap();
        }

        free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 5).unwrap();
        assert_eq!(inode.single_indirect, 0);
        assert!(resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 4 as u32).unwrap() > superblock.data_table_start);
        assert_eq!(resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, false, 5 as u32).unwrap(), 0);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn free_on_empty_or_invalid_number_inode(){
        let (mut file_device, pool, mut superblock) = full_image("ree_on_empty_or_invalid_number", 4*MIB);
        let mut inode = inode(2);

        for n in 0..12{
            inode.direct[n] = 0;
        }

        free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 5).unwrap();
        assert_eq!(free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 5), Ok(()));
        assert_eq!(free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 15), Ok(()));
        assert_eq!(free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 1036), Ok(()));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn free_twice_is_safe(){
        let (mut file_device, pool, mut superblock) = full_image("free_twice_is_safe", 4*MIB);
        let mut inode = inode(2);

        for n in 0..12{
            inode.direct[n] = 0;
        }

        resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 14 as u32).unwrap();
        assert_eq!(free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 14), Ok(()));
        assert_eq!(free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 14), Ok(()));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn free_then_allocate_works(){
        let (mut file_device, pool, mut superblock) = full_image("free_then_allocate", 4*MIB);
        let mut inode = inode(2);

        for n in 0..12{
            inode.direct[n] = 0;
            resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, n as u32).unwrap();
            resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, (n + 12) as u32).unwrap();
        }

        free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 14).unwrap();
        let n = resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, 15 as u32).unwrap();
        assert!(n > superblock.data_table_start);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn superblock_on_disk_matches_after_operations(){
        let (mut file_device, pool, mut superblock) = full_image("superblock_on_disk", 4*MIB);
        let mut inode = inode(2);

        for n in 0..12{
            inode.direct[n] = 0;
        }

        for n in [0u32, 5, 12, 13]{
            resolve_block(&mut file_device, &pool, &mut superblock, &mut inode, true, n).unwrap();
        }
        free_blocks_from(&mut file_device, &pool, &mut superblock, &mut inode, 13).unwrap();

        let mut buff = pool.take().unwrap();
        file_device.read(0, &mut buff).unwrap();
        let on_disk = Superblock::decode(&buff).unwrap();

        assert_eq!(on_disk, superblock);

        drop(buff);
        assert_eq!(pool.in_use(), 0);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn double_allocation_costs() {
        let (mut dev, pool, mut sb) = full_image("dbl_alloc_costs", 4 * MIB);
        let mut inode = Inode::empty(2);
        let start = sb.free_blocks;

        resolve_block(&mut dev, &pool, &mut sb, &mut inode, true, 1036).unwrap();
        assert_eq!(sb.free_blocks, start - 3);
        assert_ne!(inode.double_indirect, 0);

        resolve_block(&mut dev, &pool, &mut sb, &mut inode, true, 1037).unwrap();
        assert_eq!(sb.free_blocks, start - 4);

        resolve_block(&mut dev, &pool, &mut sb, &mut inode, true, 1036 + 1024).unwrap();
        assert_eq!(sb.free_blocks, start - 6);

        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn double_max_block_boundary() {
        let (mut dev, pool, mut sb) = full_image("dbl_max", 4 * MIB);
        let mut inode = Inode::empty(2);
        let start = sb.free_blocks;

        let last = resolve_block(&mut dev, &pool, &mut sb, &mut inode, true, MAX_BLOCKS - 1).unwrap();
        assert_ne!(last, 0);
        assert_eq!(sb.free_blocks, start - 3);

        assert_eq!(
            resolve_block(&mut dev, &pool, &mut sb, &mut inode, true, MAX_BLOCKS),
            Err(FsError::EFBIG)
        );
        assert_eq!(resolve_block(&mut dev, &pool, &mut sb, &mut inode, false, MAX_BLOCKS).unwrap(), 0);
        assert_eq!(sb.free_blocks, start - 3);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn double_enospc_in_the_middle_is_recoverable() {
        let (mut dev, pool, mut sb) = full_image("dbl_enospc", 4 * MIB);
        let mut inode = Inode::empty(2);

        let free = sb.free_blocks;
        for _ in 0..free - 1 {
            alloc_block(&mut sb, &pool, &mut dev).unwrap();
        }

        assert_eq!(
            resolve_block(&mut dev, &pool, &mut sb, &mut inode, true, 1036),
            Err(FsError::ENOSPC)
        );
        assert_eq!(pool.in_use(), 0);
        assert_eq!(sb.free_blocks, 0);
        assert_ne!(inode.double_indirect, 0);

        free_blocks_from(&mut dev, &pool, &mut sb, &mut inode, 0).unwrap();
        assert_eq!(inode.double_indirect, 0);
        assert_eq!(sb.free_blocks, 1);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn free_blocks_from_double_zone_table() {
        let blocks = [0u32, 12, 1036, 1037, 2060, 2061];
        let cases = [(0u32, 10u32), (12, 9), (13, 7), (1036, 7), (1037, 4), (2060, 3), (2061, 1)];

        for (from, expected) in cases {
            let (mut dev, pool, mut sb) = full_image(&format!("dbl_free_{from}"), 4 * MIB);
            let mut inode = Inode::empty(2);
            let start = sb.free_blocks;

            for b in blocks {
                resolve_block(&mut dev, &pool, &mut sb, &mut inode, true, b).unwrap();
            }
            assert_eq!(sb.free_blocks, start - 10);

            free_blocks_from(&mut dev, &pool, &mut sb, &mut inode, from).unwrap();
            assert_eq!(sb.free_blocks, start - 10 + expected);

            for b in blocks {
                let r = resolve_block(&mut dev, &pool, &mut sb, &mut inode, false, b).unwrap();
                if b < from {
                    assert_ne!(r, 0);
                } else {
                    assert_eq!(r, 0);
                }
            }

            if from <= 1036 {
                assert_eq!(inode.double_indirect, 0);
            } else {
                assert_ne!(inode.double_indirect, 0);
            }

            let free_now = sb.free_blocks;
            free_blocks_from(&mut dev, &pool, &mut sb, &mut inode, from).unwrap();
            assert_eq!(sb.free_blocks, free_now);
            assert_eq!(pool.in_use(), 0);
        }
    }

    #[test]
    fn double_free_then_allocate_again() {
        let (mut dev, pool, mut sb) = full_image("dbl_free_realloc", 4 * MIB);
        let mut inode = Inode::empty(2);
        let start = sb.free_blocks;

        for b in [1036, 1037, 2060] {
            resolve_block(&mut dev, &pool, &mut sb, &mut inode, true, b).unwrap();
        }
        free_blocks_from(&mut dev, &pool, &mut sb, &mut inode, 0).unwrap();
        assert_eq!(sb.free_blocks, start);

        resolve_block(&mut dev, &pool, &mut sb, &mut inode, true, 1036).unwrap();
        assert_eq!(sb.free_blocks, start - 3);
        assert_eq!(resolve_block(&mut dev, &pool, &mut sb, &mut inode, false, 1037).unwrap(), 0);
        assert_eq!(pool.in_use(), 0);
    }
}