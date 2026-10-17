pub fn put_u16(buff: &mut [u8], off: usize, value: u16 ) {
    buff[off..off + 2].copy_from_slice(&value.to_le_bytes());
}

pub fn get_u16(buff: &[u8], off: usize) -> u16{
    let arr: [u8; 2] = buff[off..off+2].try_into().unwrap();
    let value:u16 = u16::from_le_bytes(arr);
    value
}

pub fn put_u32(buff: &mut [u8], off: usize, value: u32 ) {
    buff[off..off + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn get_u32(buff: &[u8], off: usize) -> u32{
    let arr: [u8; 4] = buff[off..off+4].try_into().unwrap();
    let value:u32 = u32::from_le_bytes(arr);
    value
}

pub fn put_u64(buff: &mut [u8], off: usize, value: u64 ) {
    buff[off..off + 8].copy_from_slice(&value.to_le_bytes());
}

pub fn get_u64(buff: &[u8], off: usize) -> u64{
    let arr: [u8; 8] = buff[off..off+8].try_into().unwrap();
    let value:u64 = u64::from_le_bytes(arr);
    value
}

#[cfg(test)]
mod tests{
    use crate::codec::{get_u16, get_u32, get_u64, put_u16, put_u32, put_u64};

    #[test]
    fn correct_byte_order(){
        let mut buff = [0u8; 8];

        put_u16(&mut buff, 0, 0x0102);
        assert_eq!(&buff[0..2], &[2, 1]);


        put_u32(&mut buff, 0, 0x01020304);
        assert_eq!(&buff[0..4], &[4, 3, 2, 1]);

        put_u64(&mut buff, 0, 0x0102030405060708);
        assert_eq!(&buff[0..8], &[8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn correct_at_boundaries(){
        let mut buff = [0u8; 16];

        for &value in &[0u16, 1, u16::MAX] {
            put_u16(&mut buff, 1, value);
            assert_eq!(get_u16(&buff, 1), value);
        }

        for &value in &[0u32, 1, u32::MAX] {
            put_u32(&mut buff, 3, value);
            assert_eq!(get_u32(&buff, 3), value);
        }

        for &value in &[0u64, 1, u64::MAX] {
            put_u64(&mut buff, 7, value);
            assert_eq!(get_u64(&buff, 7), value);
        }
    }
}