use std::fs::{ File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

use crate::error::{FsError, FsResult};
pub const  BLOCK_SIZE:usize = 4096;
pub type BLOCK = [u8; BLOCK_SIZE];

pub trait RawDevice {
    fn read(&mut self, n:u32, buff:&mut BLOCK) -> FsResult<()>;
    fn write(&mut self, n:u32,buff:&BLOCK)-> FsResult<()>;
    fn sync(&mut self) -> FsResult<()>;
    fn total_blocks(&self) -> u32;
}

pub struct FileDevice{
    opened_file: File,
    n_blocks: u32
}

impl FileDevice {
   pub fn open(path:&str) -> FsResult<Self>{
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {opened_file: file, n_blocks:( len / BLOCK_SIZE as u64) as u32})
    }

   pub fn create(path: &str, size: u64) -> FsResult<Self>{
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        file.set_len(size)?;
        Ok(Self {opened_file: file, n_blocks:( size / BLOCK_SIZE as u64) as u32})
    }

    fn seek_to(&mut self, n:u32) -> FsResult<()> {
        if n >= self.n_blocks {
          return Err(FsError::EINVAL);
        }
        let offset = n as u64 * BLOCK_SIZE as u64;
        self.opened_file.seek(SeekFrom::Start(offset))?;
        Ok(())
    }
}

impl RawDevice for FileDevice {
    fn read(&mut self, n:u32, buff:&mut BLOCK) -> FsResult<()> {
        self.seek_to(n)?;
        self.opened_file.read_exact(buff)?;
        Ok(())
    }

    fn write(&mut self, n:u32, buff:&BLOCK)-> FsResult<()> {
        self.seek_to(n)?;
        self.opened_file.write_all(buff)?;
        Ok(())
    }

    fn sync(&mut self) -> FsResult<()> {
        self.opened_file.sync_all()?;
        Ok(())
    }

    fn total_blocks(&self) -> u32 {
        self.n_blocks
    }
}

#[cfg(test)]
mod tests{
    use std::path::PathBuf;
    use std::fs::{remove_file};
    use std::env::{temp_dir };
    use std::process::{id};

use super::*;

    fn temp_image(name: &str) -> PathBuf{
        let mut buf = temp_dir();
        buf.push(format!("fs_{}_{}.img", name, id()));
        buf
    }

    fn pattern() -> BLOCK{
        let mut array = [0u8; BLOCK_SIZE];
        for n in 0..BLOCK_SIZE{
            array[n] = (n % 251) as u8;
        }
        array
    }

    #[test]
    fn device_save_return_same_data(){
        let path = temp_image("test");
        let mut store = FileDevice::create(path.to_str().unwrap(), (5 * BLOCK_SIZE)as u64).unwrap();

        let data = pattern();
        store.write(2, &data).unwrap();

        let mut out_buff = [0u8; BLOCK_SIZE];
        store.read(2, &mut out_buff).unwrap();

        assert_eq!(data, out_buff);

        remove_file(path).unwrap();
    }

    #[test]
    fn untouched_blocks_equaals_to_zero(){
        let path = temp_image("equals_zero");
        let mut store = FileDevice::create(path.to_str().unwrap(), (5 * BLOCK_SIZE)as u64).unwrap();

        let data = pattern();
        store.write(3, &data).unwrap();

        let mut out_buff = [1u8; BLOCK_SIZE];

        store.read(2, &mut out_buff).unwrap();
        assert!(out_buff.iter().all(|&el| el == 0));
        store.read(4, &mut out_buff).unwrap();
        assert!(out_buff.iter().all(|&el| el == 0));
        store.read(3, &mut out_buff).unwrap();
        assert_eq!(out_buff, data);

        remove_file(path).unwrap();
    }

    #[test]
    fn total_blocks_return_correct_value(){
        let path = temp_image("total_correct");
        let mut store = FileDevice::create(path.to_str().unwrap(), (5 * BLOCK_SIZE)as u64).unwrap();

        let data = pattern();
        store.write(3, &data).unwrap();

        let size = store.total_blocks();

        assert_eq!(size, 5);

        remove_file(path).unwrap();
    }

    #[test]
    fn return_error_when_bigger_than_total(){
        let path = temp_image("error_when_bigger");
        let mut store = FileDevice::create(path.to_str().unwrap(), (3 * BLOCK_SIZE)as u64).unwrap();

        let mut out_buff = [0u8; BLOCK_SIZE];
        assert_eq!(store.read(3, &mut out_buff), Err(FsError::EINVAL));
        assert_eq!(store.write(3, &out_buff), Err(FsError::EINVAL));

        remove_file(path).unwrap();
    }

        #[test]
    fn first_last_block_readable(){
        let path = temp_image("first_last_readable");
        let mut store = FileDevice::create(path.to_str().unwrap(), (4 * BLOCK_SIZE)as u64).unwrap();

        let data = pattern();
        store.write(0, &data).unwrap();
        store.write(3, &data).unwrap();

        let mut out_buff = [0u8; BLOCK_SIZE];

        store.read(0, &mut out_buff).unwrap();
        assert_eq!(out_buff, data);
        store.read(3, &mut out_buff).unwrap();
        assert_eq!(out_buff, data);

        remove_file(path).unwrap();
    }

    #[test]
    fn err_when_wrong_path(){
        let path = temp_image("err_when_wrong_path");
        let str_path = path.to_str().unwrap();

        FileDevice::create(str_path, (4 * BLOCK_SIZE)as u64).unwrap();

        remove_file(str_path).unwrap();

        assert!(FileDevice::open(str_path).is_err());
    }

    #[test]
    fn read_after_close(){
        let path = temp_image("read_after_close");
        let str_path = path.to_str().unwrap();

    {        
        let mut store = FileDevice::create(str_path, (4 * BLOCK_SIZE)as u64).unwrap();
        let mut data = pattern();
        data[2] = 19;
        store.write(0,&data).unwrap();
        store.sync().unwrap();
    }

        let mut store = FileDevice::open(str_path).unwrap();

        let mut out_buff = [1u8; BLOCK_SIZE];
        store.read(0, &mut out_buff).unwrap();

        assert!(out_buff[2] == 19);

        remove_file(path).unwrap();
    }

    #[test]
    fn create_reset_data(){
        let path = temp_image("create_reset_data");
        let str_path = path.to_str().unwrap();

    {        
        let mut store = FileDevice::create(str_path, (4 * BLOCK_SIZE)as u64).unwrap();
        let data = pattern();

        store.write(0, &data).unwrap();
    }

        let mut store = FileDevice::create(str_path, (4 * BLOCK_SIZE)as u64).unwrap();
        let mut out_buff = [1u8; BLOCK_SIZE];
        store.read(0, &mut out_buff).unwrap();

        assert!(out_buff.iter().all(|&el| {el == 0}));
        remove_file(path).unwrap();
    }

    #[test]
    fn sync_is_ok(){
        let path = temp_image("sync_is_ok");
        let str_path = path.to_str().unwrap();
     
        let mut store = FileDevice::create(str_path, (4 * BLOCK_SIZE)as u64).unwrap();
        let mut data = pattern();
        data[2] = 19;
        store.write(0,&data).unwrap();
        assert!(store.sync().is_ok());
    }
}