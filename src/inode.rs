use crate::{codec::{get_u16, get_u32, get_u64, put_u16, put_u32, put_u64}, error::{FsError, FsResult}, layout::{INODE_SIZE, INODES_PER_BLOCK}};

pub const DIRECT_POINTERS:usize = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum Type { Free = 0, File = 1, Dir = 2, Symlink = 3 }

fn type_from_u8(value: u8) -> FsResult<Type>{
    match value {
        0 => Ok(Type::Free),
        1 => Ok(Type::File),
        2 => Ok(Type::Dir),
        3 => Ok(Type::Symlink),
        _ => Err(FsError::CORRUPT)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inode{
    pub id: u32,
    pub inode_type: Type,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub nlink: u32,
    pub mtime: u64,
    pub ctime: u64,
    pub direct: [u32; DIRECT_POINTERS],
    pub single_indirect: u32,
    pub double_indirect: u32,
}

impl Inode {
    pub fn empty(id: u32) -> Self {
        Inode {
            id,
            inode_type: Type::Free,
            mode: 0,
            uid: 0,
            gid: 0,
            size: 0,
            nlink: 0,
            mtime: 0,
            ctime: 0,
            direct: [0u32; DIRECT_POINTERS],
            single_indirect: 0,
            double_indirect : 0
        }
    }

    pub fn encode (&self, buff: &mut [u8; INODE_SIZE]){
        buff.fill(0);
        put_u32(buff, 0, self.id);
        buff[4] = self.inode_type as u8;
        put_u16(buff, 6,self.mode);
        put_u32(buff, 8, self.uid);
        put_u32(buff, 12, self.gid);
        put_u64(buff, 16, self.size);
        put_u32(buff, 24, self.nlink);
        put_u64(buff, 28, self.mtime);
        put_u64(buff, 36, self.ctime);
        put_u32(buff, 44, self.direct[0]);
        put_u32(buff, 48, self.direct[1]);
        put_u32(buff, 52, self.direct[2]);
        put_u32(buff, 56, self.direct[3]);
        put_u32(buff, 60, self.direct[4]);
        put_u32(buff, 64, self.direct[5]);
        put_u32(buff, 68, self.direct[6]);
        put_u32(buff, 72, self.direct[7]);
        put_u32(buff, 76, self.direct[8]);
        put_u32(buff, 80, self.direct[9]);
        put_u32(buff, 84, self.direct[10]);
        put_u32(buff, 88, self.direct[11]);
        put_u32(buff, 92, self.single_indirect);
        put_u32(buff, 96, self.double_indirect);
    }

    pub fn decode (buff: &[u8; INODE_SIZE]) -> FsResult<Self>{
        Ok( Self {        
            id: get_u32(buff, 0),
            inode_type: type_from_u8(buff[4])?,
            mode: get_u16(buff, 6),
            uid: get_u32(buff, 8),
            gid: get_u32(buff, 12),
            size: get_u64(buff, 16),
            nlink: get_u32(buff, 24),
            mtime: get_u64(buff, 28),
            ctime: get_u64(buff, 36),
            direct: [
                get_u32(buff, 44),
                get_u32(buff, 48), 
                get_u32(buff, 52),
                get_u32(buff, 56),
                get_u32(buff, 60),
                get_u32(buff, 64), 
                get_u32(buff, 68),
                get_u32(buff, 72),
                get_u32(buff, 76),
                get_u32(buff, 80), 
                get_u32(buff, 84), 
                get_u32(buff, 88)
                ],
            single_indirect: get_u32(buff, 92),
            double_indirect: get_u32(buff, 96)
        })
    }
}


pub fn inode_location (id: u32, inode_table_start: u32) -> (u32, usize){
    let index = id - 1;
    let block = inode_table_start + (index / INODES_PER_BLOCK);
    let shift_in_block = (index % INODES_PER_BLOCK) * INODE_SIZE as u32;
    (block, shift_in_block as usize)
}

#[cfg(test)]
mod tests{
use super::*;

    fn inode() -> Inode
    {
        let mut direct = [0u32; DIRECT_POINTERS];
        for n in 0..12{
            direct[n] = 100 + n as u32;
        }
        Inode { 
            id: 2, 
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
    #[test]
    fn encode_decode_works() {
        let inode = inode();
        let mut buff = [0u8; INODE_SIZE];

        inode.encode(&mut buff);

        assert_eq!(Inode::decode(&buff).unwrap(), inode);
    }

    #[test]
    fn encode_decode_works_on_gap() {
        let mut inode = inode();
        inode.size = u64::MAX;
        inode.mtime = u64::MAX;

        for n in 0..12{
            inode.direct[n] = u32::MAX
        }

        inode.ctime = 0;
        inode.mode = 0o777;

        let mut buff = [0u8; INODE_SIZE];

        inode.encode(&mut buff);

        assert_eq!(Inode::decode(&buff).unwrap(), inode);
    }

    #[test]
    fn block_with_zeros_is_free(){
        let buff = [0u8; INODE_SIZE];
        assert_eq!(Inode::decode(&buff).unwrap().inode_type, Type::Free);
    }

    #[test]
    fn unknown_type_returns_err() {
        let mut buff = [0u8; INODE_SIZE];
        buff[4] = 9;

        assert_eq!(Inode::decode(&buff), Err(FsError::CORRUPT));
    }

    #[test]
    fn inode_layout_is_correct() {
        let inode = inode();
        let mut buff = [0u8; INODE_SIZE];

        inode.encode(&mut buff);

        assert_eq!(get_u32(&buff, 0), inode.id);
        assert_eq!(buff[4], Type::Symlink as u8);
        assert_eq!(get_u64(&buff, 16), inode.size);
        assert_eq!(get_u32(&buff, 44), inode.direct[0]);
        assert_eq!(get_u32(&buff, 96), inode.double_indirect);
        assert!(buff[100..].iter().all(|&b| b == 0));
    }

    #[test]
    fn inode_location_works_correct() {
        assert_eq!(inode_location(1,100), (100, 0));
        assert_eq!(inode_location(2,100), (100, 128));
        assert_eq!(inode_location(32,100), (100, 31*128));
        assert_eq!(inode_location(33,100), (101, 0));
    }


}