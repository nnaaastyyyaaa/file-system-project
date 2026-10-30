use crate::{error::{FsError, FsResult}, inode::{Inode, Type, inode_location}, layout::INODE_SIZE, mem::BufferPooll, raw_device::RawDevice, superblock::Superblock};

fn check_id(id:u32, total_inodes: u32 ) -> FsResult<()>{
    if id < 1 || id > total_inodes { 
        return Err(FsError::EINVAL) 
    }
    Ok(())
}

pub fn read<D: RawDevice>(id:u32, device: &mut D, superblock: &Superblock, pool: &BufferPooll) -> FsResult<Inode>{
    check_id(id, superblock.total_inodes)?;

    let (block, shift ) = inode_location(id, superblock.inode_table_start);

    let mut buff = pool.take()?;
    device.read(block, &mut buff)?;
    let data =   &buff[shift..shift + INODE_SIZE];
    let data_buff = <[u8; INODE_SIZE]>::try_from(data).map_err(|_| FsError::CORRUPT)?;
    let inode = Inode::decode(&data_buff)?;
    if inode.inode_type == Type::Free { 
        return Ok(Inode::empty(id)) 
    };

    Ok(inode)
}

pub fn write<D: RawDevice>(inode: &Inode, device: &mut D, superblock: &Superblock, pool: &BufferPooll) -> FsResult<()>{
    check_id(inode.id, superblock.total_inodes)?;

    let (block, shift ) = inode_location(inode.id, superblock.inode_table_start);
    let mut buff = pool.take()?;

    device.read(block, &mut buff)?;
    let slice = &mut buff[shift..shift + INODE_SIZE];
    let  data_buf = <&mut [u8; INODE_SIZE]>::try_from(slice).map_err(|_| FsError::CORRUPT)?;

    inode.encode( data_buf);
    device.write(block, &buff)?;
   
    Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    use crate::{inode_table::write, test_utils::{MIB, full_image, inode}};

    #[test]
    fn writes_and_reads_corectly(){
        let (mut img, pool, sb) = full_image("writes_and_reads", 4 * MIB);
        let inode = inode(2);
        write(&inode, &mut img, &sb, &pool).unwrap();
        assert_eq!(read(2, &mut img, &sb, &pool).unwrap(), inode );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn works_at_boundaries(){
        let (mut img, pool, sb) = full_image("works_at_boundaries", 4 * MIB);

        for id in [31, 32, 33] {
            let inode = inode(id);
            write(&inode, &mut img, &sb, &pool).unwrap();
        }

        for id in [31, 32, 33] {
            assert_eq!(read(id, &mut img, &sb, &pool).unwrap(), inode(id) );
            assert_eq!(pool.in_use(), 0);
        }
    }

    #[test]
    fn unused_inode_is_free(){
        let (mut img, pool,  sb) = full_image("unused_inode_is_free", 4 * MIB);
        assert_eq!(read(10, &mut img, &sb, &pool).unwrap().inode_type, Type::Free  );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn id_out_of_range_return_error(){
        let (mut img, pool,  sb) = full_image("id_out_of_range_return_error", 4 * MIB);
        
        assert_eq!(read(0, &mut img, &sb, &pool), Err(FsError::EINVAL)  );
        assert_eq!(read(sb.total_inodes + 1, &mut img, &sb, &pool), Err(FsError::EINVAL)  );
    }
}