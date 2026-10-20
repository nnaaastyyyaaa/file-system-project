use crate::{codec::{get_u32, put_u32}, error::{FsError, FsResult}};

pub const ENTRY_SIZE: usize = 64;
pub const NAME_SIZE: usize = 59;

pub fn encode_entry(buff: &mut [u8], id:u32, name: &str) -> FsResult<()> {
    if id == 0 { return Err(FsError::EINVAL)}
    if buff.len() < ENTRY_SIZE { return Err(FsError::EINVAL)}

    let name_bytes = name.as_bytes();
    if name_bytes.is_empty() || name_bytes.contains(&b'/') || name_bytes.contains(&0)  { return Err(FsError::EINVAL) }
    if name_bytes.len() > NAME_SIZE  { return Err(FsError::ENAMETOOLONG) }

    buff[0..ENTRY_SIZE].fill(0);
    put_u32(buff, 0, id);

    buff[4] = name_bytes.len() as u8;
    buff[5..5+name_bytes.len()].copy_from_slice(name_bytes);

    Ok(())
}

pub fn decode_entry(buff: &[u8]) -> FsResult<Option<(u32, String)>>{
    if buff.len() < ENTRY_SIZE { return Err(FsError::EINVAL)}

    let id = get_u32(buff, 0);
    if id == 0 { return Ok(None) }

    let name_len = buff[4] as usize;
    if name_len == 0 { return Err(FsError::CORRUPT) }
    if name_len > NAME_SIZE { return Err(FsError::CORRUPT) }

    let name_bytes = &buff[5..5 + name_len];
    match String::from_utf8(name_bytes.to_vec()){
        Ok(name) => Ok(Some((id, name))),
        Err(_) => Err(FsError::CORRUPT)
    }
}

pub fn clear_entry(buff: &mut [u8]){
    buff[0..ENTRY_SIZE].fill(0);
}

#[cfg(test)]
mod tests{
    use super::*;
    use crate::{ error::FsError, raw_device::BLOCK_SIZE};

    #[test]
    fn encode_decode_dir_works(){
        let name = "hello.txt".to_string();
        let mut buff = [0u8; BLOCK_SIZE];
        encode_entry(&mut buff, 3, &name).unwrap();

        assert_eq!(decode_entry(&buff).unwrap(), Some((3 , name)));
    }

    #[test]
    fn encode_corectly_fills_buffer(){
        let name = "hello.txt".to_string();
        let mut buff = [0xff; BLOCK_SIZE];
        encode_entry(&mut buff, 3, &name).unwrap();

        assert!(buff[ENTRY_SIZE..].iter().all(|&b| b == 0xff));
    }

    #[test]
    fn utf8_length_is_in_bytes() {
        let mut buff = [0u8; BLOCK_SIZE];
        encode_entry(&mut buff, 3, "файл").unwrap();

        assert_eq!(buff[4], 8);
        assert_eq!(decode_entry(&buff).unwrap().unwrap().1, "файл");
    }

    #[test]
    fn name_length_limit_works(){
        let mut buff = [0u8; BLOCK_SIZE];
        let mut name = "a".repeat(59);
        assert_eq!(encode_entry(&mut buff, 3, &name), Ok(()));

        name = "a".repeat(60);
        assert_eq!(encode_entry(&mut buff, 3, &name), Err(FsError::ENAMETOOLONG));

        assert_eq!(encode_entry(&mut buff, 3, ""), Err(FsError::EINVAL));
    }

    #[test]
    fn zero_slot_decoded_corectly(){
        let mut buff = [0u8; BLOCK_SIZE];
        assert_eq!(decode_entry(&buff), Ok(None));

        encode_entry(&mut buff, 12, "name.txt").unwrap();
        clear_entry(&mut buff);

        assert_eq!(decode_entry(&buff), Ok(None));
    }

    #[test]
    fn name_checked_correctly(){
        let mut buff = [0u8; BLOCK_SIZE];
        assert_eq!(encode_entry(&mut buff, 2, "a/b"), Err(FsError::EINVAL));
        assert_eq!(encode_entry(&mut buff, 4, "a\0b"), Err(FsError::EINVAL));
    }

    #[test]
    fn zero_id_returns_error(){
        let mut buff = [0u8; BLOCK_SIZE];
        assert_eq!(encode_entry(&mut buff, 0, "name.txt"), Err(FsError::EINVAL));
    }

    #[test]
    fn decode_return_error_on_invalid_data(){
        let name = "hello.txt".to_string();
        let mut buff = [0u8; BLOCK_SIZE];
        encode_entry(&mut buff, 3, &name).unwrap();

        buff[4] = 0;
        assert_eq!(decode_entry(&buff), Err(FsError::CORRUPT));

        buff[4] = 60;
        assert_eq!(decode_entry(&buff), Err(FsError::CORRUPT));
    }

    #[test]
    fn short_buffer_returns_error(){
        let name = "hello.txt".to_string();
        let mut buff: [u8; 60] = [0u8; 60];
        assert_eq!(encode_entry(&mut buff, 3, &name), Err(FsError::EINVAL));
        assert_eq!(decode_entry(&buff), Err(FsError::EINVAL));
    }

}