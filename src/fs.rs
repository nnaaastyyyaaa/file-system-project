use std::{collections::HashMap};

use crate::{alloc::{alloc_inode, free_inode}, directory::{add_entry, lookup, remove_entry}, error::{FsError, FsResult}, file_io::truncate_file, inode::{Inode, Type, now_ms}, inode_table, mem::BufferPooll, mkfs::mkfs, raw_device::RawDevice, superblock::{ROOT_INODE, Superblock}};

const MAX_OPEN_FILES: usize = 64; 

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct OpenedFile {
    pub inode_id: u32,
    pub offset: u64,
}
pub struct Fs <D: RawDevice> {
    pub device: D,
    pub superblock: Superblock,
    pub pool: BufferPooll,
    pub opened_files: Vec<Option<OpenedFile>>,
    pub opened_inodes: HashMap<u32, u32>,
}

impl <D: RawDevice> Fs<D> {
    pub fn format_fs(mut device: D, pool: BufferPooll, inode_count: Option<u32>) -> FsResult<Self> {
       let superblock = mkfs(&pool, &mut device, inode_count)?;
        Ok(Self {device, superblock, pool, opened_files: Vec::new(), opened_inodes: HashMap::new() })
    }

    pub fn create_file(&mut self, name: &str) -> FsResult<u32> {

        let mut root_inode = inode_table::read(ROOT_INODE, &mut self.device, &self.superblock, &self.pool)?;
        let inode_id = alloc_inode(&mut self.superblock, &self.pool, &mut self.device)?;
        let now = now_ms();

        let mut inode = Inode::empty(inode_id);
        inode.nlink = 1;
        inode.inode_type = Type::File;
        inode.mode = 0o644;
        inode.mtime = now;
        inode.ctime = now;

        inode_table::write(&inode, &mut self.device, &self.superblock, &self.pool)?;
        let result = add_entry(&mut self.device, &mut self.superblock, &mut root_inode, &self.pool, inode_id, name);

        match result {
            Ok(_) => {},
            Err(e) => {
                let _ = free_inode(&mut self.superblock, inode_id, &self.pool, &mut self.device);
                inode = Inode::empty(inode_id);
                let _ = inode_table::write(&inode, &mut self.device, &self.superblock, &self.pool);
                return Err(e);
            }
        }
        Ok(inode_id)
    }

    pub fn stat_file(&mut self, name: &str) -> FsResult<Inode> {
        let mut root_inode = inode_table::read(ROOT_INODE, &mut self.device, &self.superblock, &self.pool)?;
        let id = match lookup(&mut self.device, &mut self.superblock, &mut root_inode, &self.pool, name) {
            Ok(id) => id,
            Err(e) => return Err(e),
            
        };
        let inode = inode_table::read(id, &mut self.device, &self.superblock, &self.pool)?;
        Ok(inode)
    }

    pub fn ls(&mut self) -> FsResult<Vec<(u32, String)>> {
        let mut root_inode = inode_table::read(ROOT_INODE, &mut self.device, &self.superblock, &self.pool)?;
        let entries = crate::directory::list(&mut self.device, &mut self.superblock, &mut root_inode, &self.pool)?;
        Ok(entries)
    }

    pub fn link(&mut self, name1: &str, name2: &str) -> FsResult<()> {
        let mut root_inode = inode_table::read(ROOT_INODE, &mut self.device, &self.superblock, &self.pool)?;

        let mut inode = match self.stat_file(name1){
            Ok(inode) => inode,
            Err(e) => return Err(e),
        };
        if inode.inode_type != Type::File {
            return Err(FsError::EINVAL);
        }

        let prev_mtime = inode.mtime;
        let prev_nlink = inode.nlink;
        inode.nlink = inode.nlink.checked_add(1).ok_or(FsError::CORRUPT)?;
        inode.mtime = now_ms();
        inode_table::write(&inode, &mut self.device, &self.superblock, &self.pool)?;
        let result = add_entry(&mut self.device, &mut self.superblock, &mut root_inode, &self.pool, inode.id, name2);

        match result {
            Ok(_) => {},
            Err(e) => {
                inode.nlink = prev_nlink;
                inode.mtime = prev_mtime;
                let _ = inode_table::write(&inode, &mut self.device, &self.superblock, &self.pool);
                return Err(e)
            },
        }

        Ok(())
    }

    fn maybe_free(&mut self, id: u32) -> FsResult<()>{
        let mut inode = inode_table::read(id, &mut self.device, &mut self.superblock, &self.pool)?;
        let opened = self.opened_inodes.contains_key(&inode.id);

        if inode.nlink == 0 && !opened {
            self.opened_inodes.remove(&id);
            truncate_file(&mut self.device, &self.pool, &mut self.superblock, &mut inode, 0)?;
            inode_table::write(&Inode::empty(inode.id), &mut self.device, &mut self.superblock,  &self.pool)?;
            free_inode(&mut self.superblock, inode.id, &self.pool, &mut self.device)?;
        }

        Ok(())
    }

    pub fn unlink(&mut self, name: &str) -> FsResult<()>{
        let mut root_inode = inode_table::read(ROOT_INODE, &mut self.device, &mut self.superblock, &self.pool)?;
        let mut inode = self.stat_file(name)?;
        let now = now_ms();

        inode.nlink = inode.nlink.checked_sub(1).ok_or(FsError::CORRUPT)?;
        inode.ctime = now;
        remove_entry(&mut self.device, &mut self.superblock, &mut root_inode, &self.pool, name)?;
        inode_table::write(&inode, &mut self.device, &mut self.superblock,  &self.pool)?;

        self.maybe_free(inode.id)?;

        Ok(())
    }

    pub fn open(&mut self, name: &str) -> FsResult<usize>{
        let inode = self.stat_file(name)?;
        if inode.inode_type == Type::Dir {
            return Err(FsError::EISDIR)
        }
        let fd = match self.opened_files.iter().position(|el| el.is_none()){
            Some(fd) => fd,
            None => {
                if self.opened_files.len() >= MAX_OPEN_FILES {
                    return Err(FsError::ENOMEM);            
                }
                self.opened_files.push(None);                
                self.opened_files.len() - 1                 
            }
        };
        let opened_file = OpenedFile{
            inode_id: inode.id,
            offset: 0
        };
        self.opened_files[fd] = Some(opened_file);
        *self.opened_inodes.entry(inode.id).or_insert(0) += 1;
        Ok(fd)
    }

    pub fn close(&mut self, fd: usize) -> FsResult<()> {
        let id = self.opened_files
            .get_mut(fd)
            .and_then(|slot| slot.take())
            .ok_or(FsError::EINVAL)? 
            .inode_id;

        match self.opened_inodes.get_mut(&id) {
            Some(c) if *c > 1 => *c -= 1,
            Some(_) => { self.opened_inodes.remove(&id); }
            None => return Err(FsError::CORRUPT),
        }

        self.maybe_free(id)
    }

}

#[cfg(test)]
mod tests{
    use super::*;
    use crate::{directory::{list, lookup}, dirent::ENTRY_SIZE, error::FsError, inode::Type, inode_table, mem::BufferPooll, raw_device::{BLOCK_SIZE, FileDevice}, superblock::ROOT_INODE, test_utils::{MIB, temp_image}};

    #[test]
    fn test_format(){
        let image = temp_image("test_format");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let fs = Fs::format_fs(device, pool, Some(128)).unwrap();

        assert_eq!(fs.superblock.total_inodes, 128);
        assert_eq!(fs.superblock.free_inodes, fs.superblock.total_inodes - 1);
        assert_eq!(fs.superblock.free_blocks, fs.superblock.data_table_blocks - 1);
        assert_eq!(fs.pool.in_use(), 0);
        assert!(fs.opened_files.is_empty());
        assert!(fs.opened_inodes.is_empty());
    }

    #[test]
    fn format_root_inode_is_described_on_disk(){
        let image = temp_image("format_root_inode_is_described_on_disk");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, Some(128)).unwrap();

        let root = inode_table::read(ROOT_INODE, &mut fs.device, &fs.superblock, &fs.pool).unwrap();

        assert_eq!(root.inode_type, Type::Dir);
        assert_eq!(root.nlink, 2);
        assert_eq!(root.size, 2 * ENTRY_SIZE as u64);
        assert_ne!(root.direct[0], 0);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn format_root_contains_dot_and_dotdot(){
        let image = temp_image("format_root_contains_dot_and_dotdot");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, Some(128)).unwrap();

        let mut root = inode_table::read(ROOT_INODE, &mut fs.device, &fs.superblock, &fs.pool).unwrap();
        let entries = list(&mut fs.device, &mut fs.superblock, &mut root, &fs.pool).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], (1, ".".to_string()));
        assert_eq!(entries[1], (1, "..".to_string()));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn format_too_small_image_returns_err(){
        let image = temp_image("format_too_small_image_returns_err");
        let device = FileDevice::create(image.to_str().unwrap(), MIB).unwrap();
        let pool = BufferPooll::new();
        assert_eq!(Fs::format_fs(device, pool, Some(100)).err(), Some(FsError::EINVAL));
    }

    #[test]
    fn format_rounds_inode_count_up_to_full_block(){
        let image = temp_image("format_rounds_inode_count_up_to_full_block");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let fs = Fs::format_fs(device, pool, Some(100)).unwrap();

        assert_eq!(fs.superblock.total_inodes, 128);
        assert_eq!(fs.superblock.free_inodes, 127);
    }

    #[test]
    fn format_zero_inodes_returns_err(){
        let image = temp_image("format_zero_inodes_returns_err");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();

        assert_eq!(Fs::format_fs(device, pool, Some(0)).err(), Some(FsError::EINVAL));
    }


    #[test]
    fn create_adds_name_to_root(){
        let image = temp_image("create_adds_name_to_root");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();
        let file_id = fs.create_file("my_file").unwrap();

        let mut root = inode_table::read(ROOT_INODE, &mut fs.device, &fs.superblock, &fs.pool).unwrap();
        let entries = list(&mut fs.device, &mut fs.superblock, &mut root, &fs.pool).unwrap();
        assert_eq!(entries[2], (file_id, "my_file".to_string()));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn create_writes_inode_to_disk(){
        let image = temp_image("create_writes_inode_to_disk");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();
        let file_id = fs.create_file("my_file").unwrap();

        let inode =  inode_table::read(file_id, &mut fs.device, &fs.superblock, &fs.pool).unwrap();
        assert_eq!(inode.inode_type, Type::File);
        assert_eq!(inode.nlink, 1);
        assert_eq!(inode.size, 0);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn create_correctly_updates_sb_values(){
        let image = temp_image("create_correctly_updates_sb_values");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();

        let free_blocks_before = fs.superblock.free_blocks;
        let free_inodes_before = fs.superblock.free_inodes;

        fs.create_file("my_file").unwrap();

        assert_eq!(fs.superblock.free_blocks, free_blocks_before);
        assert_eq!(fs.superblock.free_inodes, free_inodes_before - 1);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn create_correctly_handles_error(){
        let image = temp_image("create_correctly_handles_error");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();

        let free_blocks_before = fs.superblock.free_blocks;
        let free_inodes_before = fs.superblock.free_inodes;

        let first_id = fs.create_file("my_file").unwrap();
        assert_eq!(fs.create_file("my_file"), Err(FsError::EEXIST));

        let second_id = fs.create_file("my_file2").unwrap();
        assert_eq!(first_id, second_id - 1);
        assert_eq!(fs.superblock.free_blocks, free_blocks_before);
        assert_eq!(fs.superblock.free_inodes, free_inodes_before - 2);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn create_with_invalid_name_returns_err(){
        let image = temp_image("create_with_invalid_name_returns_err");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();

        assert_eq!(fs.create_file(""), Err(FsError::EINVAL));
        assert_eq!(fs.create_file(&"a".repeat(65)), Err(FsError::ENAMETOOLONG));
        assert_eq!(fs.create_file("a/b"), Err(FsError::EINVAL));
    }

    #[test]
    fn create_dot_names_return_eexist(){
        let image = temp_image("create_dot_names_return_eexist");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();

        assert_eq!(fs.create_file("."), Err(FsError::EEXIST));
        assert_eq!(fs.create_file(".."), Err(FsError::EEXIST));
    }

    #[test]
    fn create_too_many_files_returns_enospc(){
        let image = temp_image("create_too_many_files_returns_enospc");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(32)).unwrap(); 

        for i in 0..31 {
            let name = format!("file{}", i);
            fs.create_file(&name).unwrap();
        }
        assert_eq!(fs.create_file("file32"), Err(FsError::ENOSPC));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn creates_many_files_without_error(){
        let image = temp_image("creates_many_files_without_error");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap(); 

        for i in 0..72 {
            let name = format!("file{}", i);
            fs.create_file(&name).unwrap();
        }

        let mut root = inode_table::read(ROOT_INODE, &mut fs.device, &fs.superblock, &fs.pool).unwrap();
        let entries = list(&mut fs.device, &mut fs.superblock, &mut root, &fs.pool).unwrap();
        assert_eq!(entries.len(), 74);

        for i in 0..72 {
            let id = lookup(&mut fs.device, &mut fs.superblock, &mut root, &fs.pool, &format!("file{}", i)).unwrap();
            assert_eq!(id, i + 2);
        }

        assert_eq!(fs.superblock.free_inodes, fs.superblock.total_inodes - 73);
        assert_eq!(fs.pool.in_use(), 0);

    }

    #[test]
    fn stat_file_returns_correct_inode(){
        let image = temp_image("stat_file_returns_correct_inode");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();
    
        fs.create_file("data").unwrap();
        let inode = fs.stat_file("data").unwrap();
        assert_eq!(inode.id, 2);
        assert_eq!(inode.inode_type, Type::File);
        assert_eq!(inode.nlink, 1);
        assert_eq!(inode.size, 0);
        assert_eq!(inode.mode, 0o644);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn stat_file_returns_err_for_nonexistent_file(){
        let image = temp_image("stat_file_returns_err_for_nonexistent_file");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();  

        assert_eq!(fs.stat_file("data"), Err(FsError::ENOENT));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn stat_of_dot_and_dotdot(){
        let image = temp_image("stat_of_dot_and_dotdot");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();  

        let dot_inode = fs.stat_file(".").unwrap();
        let dotdot_inode = fs.stat_file("..").unwrap();
        assert_eq!(dot_inode.id, 1);
        assert_eq!(dotdot_inode.id, 1);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn stat_returns_the_same_data_for_same_inodes(){
        let image = temp_image("returns_the_same_data_for_same_inodes");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs =Fs::format_fs(device, pool, Some(128)).unwrap();  

        let dot_inode = fs.stat_file(".").unwrap();
        let dot_inode_clone = fs.stat_file(".").unwrap();
        assert_eq!(dot_inode, dot_inode_clone);
    }

    #[test]
    fn ls_after_format_returns_dot_and_dotdot(){
        let image = temp_image("ls_after_format_returns_dot_and_dotdot");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, Some(128)).unwrap();

        let entries = fs.ls().unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], (1, ".".to_string()));
        assert_eq!(entries[1], (1, "..".to_string()));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn ls_shows_created_files(){
        let image = temp_image("ls_shows_created_files");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, Some(128)).unwrap();

        let a = fs.create_file("a").unwrap();
        let b = fs.create_file("b").unwrap();
        let entries = fs.ls().unwrap();

        assert_eq!(entries.len(), 4);
        assert_eq!(entries[2], (a, "a".to_string()));
        assert_eq!(entries[3], (b, "b".to_string()));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn link_increases_nlink_and_adds_entry(){
        let image = temp_image("link_increases_nlink_and_adds_entry");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let file_id = fs.create_file("data").unwrap();
        fs.link("data", "new_data").unwrap();

        let mut root_inode = inode_table::read(ROOT_INODE, &mut fs.device, &mut fs.superblock, &fs.pool).unwrap();
        assert_eq!(lookup(&mut fs.device, &mut fs.superblock, &mut root_inode, &fs.pool, "data"), lookup(&mut fs.device, &mut fs.superblock, &mut root_inode, &fs.pool, "new_data"));
        let inode = inode_table::read(file_id, &mut fs.device, &mut fs.superblock, &fs.pool).unwrap();
        assert_eq!(inode.nlink, 2);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn link_with_unknown_name_return_error(){
        let image = temp_image("link_with_unknown_name_return_error");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        assert_eq!(fs.link("data", "new_data"), Err(FsError::ENOENT));
        let entries = fs.ls().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn link_with_already_existing_name2_return_error(){
        let image = temp_image("link_with_already_existing_name2_return_error");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        fs.create_file("data").unwrap();
        fs.create_file("new_data").unwrap();
        assert_eq!(fs.link("data", "new_data"), Err(FsError::EEXIST));

        let inode = fs.stat_file("data").unwrap();
        assert_eq!(inode.nlink, 1);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn link_to_dir_return_error(){
        let image = temp_image("link_to_dir_return_error");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();
        
        assert_eq!(fs.link(".", "new_data"), Err(FsError::EINVAL));
        assert_eq!(fs.link("..", "new_data"), Err(FsError::EINVAL));

        let root_inode = inode_table::read(ROOT_INODE, &mut fs.device, &mut fs.superblock, &fs.pool).unwrap();
        assert_eq!(root_inode.nlink, 2);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn link_with_invalid_name_return_error(){
        let image = temp_image("link_with_invalid_name_return_error");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        fs.create_file("data").unwrap();
        assert_eq!(fs.link("data", "new/data"), Err(FsError::EINVAL));
        assert_eq!(fs.link("data", ""), Err(FsError::EINVAL));

        let inode = fs.stat_file("data").unwrap();
        assert_eq!(inode.nlink, 1);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn two_links_in_a_row_works(){
        let image = temp_image("two_links_in_a_row_works");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        fs.create_file("data").unwrap();
        fs.link("data", "a").unwrap();
        fs.link("data", "b").unwrap();

        let inode = fs.stat_file("data").unwrap();
        assert_eq!(inode.nlink, 3);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn works_when_first_block_of_dir_full(){
        let image = temp_image("works_when_first_block_of_dir_full");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let entries_per_block = BLOCK_SIZE / ENTRY_SIZE;
        for n in 2..entries_per_block {
            fs.create_file(&format!("data{}",n)).unwrap();
        }
        fs.link("data3", "a").unwrap();

        let root_inode = inode_table::read(ROOT_INODE, &mut fs.device, &mut fs.superblock, &fs.pool).unwrap();
        assert_ne!(root_inode.direct[0], 0);
        assert_ne!(root_inode.direct[1], 0);
        assert_eq!(root_inode.size, (BLOCK_SIZE + ENTRY_SIZE) as u64);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn unlink_remove_name(){
        let image = temp_image("unlink_remove_name");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id = fs.create_file("data").unwrap();
        fs.link("data", "one_more_name").unwrap();

        let inodes_before = fs.superblock.free_inodes;
        let blocks_before = fs.superblock.free_blocks;

        let mut inode = fs.stat_file("data").unwrap();
        assert_eq!(inode.nlink, 2);
        assert_eq!(fs.stat_file("one_more_name").unwrap().id, id);

        fs.unlink("data").unwrap();
        assert_eq!(fs.stat_file("data"), Err(FsError::ENOENT));

        inode = fs.stat_file("one_more_name").unwrap();
        assert_eq!(inode.id, id);

        assert_eq!(fs.superblock.free_blocks, blocks_before);
        assert_eq!(fs.superblock.free_inodes, inodes_before);

        let entries = fs.ls().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[2], (id, "one_more_name".to_string()));
        assert_eq!(fs.pool.in_use(), 0);

    }

    #[test]
    fn unlink_remove_file(){
        let image = temp_image("unlink_remove_file");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id = fs.create_file("data").unwrap();

        let inodes_before = fs.superblock.free_inodes;
        let blocks_before = fs.superblock.free_blocks;

        fs.unlink("data").unwrap();

        assert_eq!(fs.stat_file("data"), Err(FsError::ENOENT));

        let inode = inode_table::read(id, &mut fs.device, &mut fs.superblock, &fs.pool).unwrap();
        assert_eq!(inode.inode_type, Type::Free);
        assert_eq!(inode.nlink, 0);

        assert_eq!(fs.superblock.free_blocks, blocks_before);
        assert_eq!(fs.superblock.free_inodes - 1, inodes_before);
        assert_eq!(fs.create_file("other").unwrap(), id);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn unlink_two_names_in_row(){
        let image = temp_image("unlink_two_names_in_row");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();
        let inodes_before = fs.superblock.free_inodes;
        let blocks_before = fs.superblock.free_blocks;

        let id =fs.create_file("data").unwrap();
        fs.link("data", "name2").unwrap();
        assert_eq!(fs.stat_file("name2").unwrap().id, id);

        fs.unlink("data").unwrap();
        fs.unlink("name2").unwrap();

        assert_eq!(fs.stat_file("data"), Err(FsError::ENOENT));
        assert_eq!(fs.stat_file("name2"), Err(FsError::ENOENT));

        assert_eq!(fs.superblock.free_blocks, blocks_before);
        assert_eq!(fs.superblock.free_inodes, inodes_before);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn unlink_with_unknown_name() {
        let image = temp_image("unlink_with_unknown_name");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        assert_eq!(fs.unlink("data"), Err(FsError::ENOENT));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn unlink_root_entries(){
        let image = temp_image("unlink_root_entries");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        assert_eq!(fs.unlink("."), Err(FsError::EINVAL));
        assert_eq!(fs.unlink(".."), Err(FsError::EINVAL));

        let inode = fs.stat_file(".").unwrap();
        assert_eq!(inode.nlink, 2);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn open_return_correct_fd(){
        let image = temp_image("open_return_correct_fd");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        fs.create_file("data").unwrap();
        assert_eq!(fs.open("data").unwrap(), 0);

        fs.create_file("new_data").unwrap();
        assert_eq!(fs.open("new_data").unwrap(), 1);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn open_save_correct_opened_file(){
        let image = temp_image("open_save_correct_opened_file");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id = fs.create_file("data").unwrap();
        let fd = fs.open("data").unwrap();

        assert_eq!(fs.opened_files[fd], Some(OpenedFile { inode_id: id, offset: 0}));
        assert_eq!(fs.pool.in_use(), 0);
    }
    
    #[test]
    fn open_unknown_name_return_error(){
        let image = temp_image("open_unknown_name_return_error");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        assert_eq!(fs.open("data"), Err(FsError::ENOENT));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn open_dir_name_return_eisdir(){
        let image = temp_image("open_dir_name_return_eisdir");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        assert_eq!(fs.open("."), Err(FsError::EISDIR));
        assert_eq!(fs.open(".."), Err(FsError::EISDIR));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn open_more_than_allowed_files_return_error(){
        let image = temp_image("open_more_than_allowed_files_return_error");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        for n in 0..MAX_OPEN_FILES{
            fs.create_file(&format!("data{}",n)).unwrap();
            fs.open(&format!("data{}",n)).unwrap();
        }
        fs.create_file("data64").unwrap();

        assert_eq!(fs.open("data64"), Err(FsError::ENOMEM));
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn open_two_files_have_separate_fds_and_counters(){
        let image = temp_image("open_two_files_have_separate_fds_and_counters");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id_a = fs.create_file("a").unwrap();
        let id_b = fs.create_file("b").unwrap();

        let fd_a = fs.open("a").unwrap();
        let fd_b = fs.open("b").unwrap();
        let fd_a2 = fs.open("a").unwrap();

        assert_eq!(fd_a, 0);
        assert_eq!(fd_b, 1);
        assert_eq!(fd_a2, 2);

        assert_eq!(fs.opened_files[fd_a], Some(OpenedFile { inode_id: id_a, offset: 0 }));
        assert_eq!(fs.opened_files[fd_b], Some(OpenedFile { inode_id: id_b, offset: 0 }));
        assert_eq!(fs.opened_files[fd_a2], Some(OpenedFile { inode_id: id_a, offset: 0 }));

        assert_eq!(fs.opened_inodes.get(&id_a), Some(&2));
        assert_eq!(fs.opened_inodes.get(&id_b), Some(&1));
        assert_eq!(fs.opened_inodes.len(), 2);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn close_correctly_updates_tables(){
        let image = temp_image("close_correctly_updates_tables");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id = fs.create_file("a").unwrap();
        let fd = fs.open("a").unwrap();

        fs.close(fd).unwrap();
        assert_eq!(fs.opened_files[fd], None);
        assert_eq!(fs.opened_inodes.get(&id), None);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn close_one_of_two_fds_keeps_counter(){
        let image = temp_image("close_one_of_two_fds_keeps_counter");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id = fs.create_file("a").unwrap();
        let fd1 = fs.open("a").unwrap();
        let fd2 = fs.open("a").unwrap();
        assert_eq!(fs.opened_inodes.get(&id), Some(&2));

        fs.close(fd1).unwrap();
        assert_eq!(fs.opened_files[fd1], None);
        assert_eq!(fs.opened_files[fd2], Some(OpenedFile { inode_id: id, offset: 0 }));
        assert_eq!(fs.opened_inodes.get(&id), Some(&1));

        fs.close(fd2).unwrap();
        assert_eq!(fs.opened_files[fd2], None);
        assert_eq!(fs.opened_inodes.get(&id), None);
        assert!(fs.opened_inodes.is_empty());
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn close_invalid_fd_returns_err_and_changes_nothing(){
        let image = temp_image("close_invalid_fd_returns_err_and_changes_nothing");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id = fs.create_file("a").unwrap();
        fs.open("a").unwrap();
        fs.close(0).unwrap();

        let files_before = fs.opened_files.clone();
        let inodes_before = fs.opened_inodes.clone();
        let free_inodes_before = fs.superblock.free_inodes;

        assert_eq!(fs.close(0), Err(FsError::EINVAL));
        assert_eq!(fs.close(100), Err(FsError::EINVAL));

        assert_eq!(fs.opened_files, files_before);
        assert_eq!(fs.opened_inodes, inodes_before);
        assert_eq!(fs.opened_inodes.get(&id), None);
        assert_eq!(fs.superblock.free_inodes, free_inodes_before);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn double_close_returns_err(){
        let image = temp_image("double_close_returns_err");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        fs.create_file("a").unwrap();
        let fd = fs.open("a").unwrap();

        assert_eq!(fs.close(fd), Ok(()));
        assert_eq!(fs.close(fd), Err(FsError::EINVAL));

        assert_eq!(fs.opened_files[fd], None);
        assert!(fs.opened_inodes.is_empty());
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn close_frees_slot_for_reuse(){
        let image = temp_image("close_frees_slot_for_reuse");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id_a = fs.create_file("a").unwrap();
        let id_b = fs.create_file("b").unwrap();
        let id_c = fs.create_file("c").unwrap();

        assert_eq!(fs.open("a").unwrap(), 0);
        assert_eq!(fs.open("b").unwrap(), 1);
        assert_eq!(fs.open("c").unwrap(), 2);

        fs.close(1).unwrap();
        assert_eq!(fs.opened_files.len(), 3);
        assert_eq!(fs.opened_files[0], Some(OpenedFile { inode_id: id_a, offset: 0 }));
        assert_eq!(fs.opened_files[1], None);
        assert_eq!(fs.opened_files[2], Some(OpenedFile { inode_id: id_c, offset: 0 }));
        assert_eq!(fs.opened_inodes.get(&id_b), None);

        assert_eq!(fs.open("c").unwrap(), 1);
        assert_eq!(fs.opened_files[1], Some(OpenedFile { inode_id: id_c, offset: 0 }));
        assert_eq!(fs.opened_inodes.get(&id_c), Some(&2));
        assert_eq!(fs.opened_files.len(), 3);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn unlink_of_opened_file_frees_it_only_on_close(){
        let image = temp_image("unlink_of_opened_file_frees_it_only_on_close");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id = fs.create_file("a").unwrap();
        let inodes_before = fs.superblock.free_inodes;
        let blocks_before = fs.superblock.free_blocks;

        let fd = fs.open("a").unwrap();
        fs.unlink("a").unwrap();

        assert_eq!(fs.stat_file("a"), Err(FsError::ENOENT));
        let mut inode = inode_table::read(id, &mut fs.device, &fs.superblock, &fs.pool).unwrap();
        assert_eq!(inode.nlink, 0);
        assert_eq!(inode.inode_type, Type::File);
        assert_eq!(fs.superblock.free_inodes, inodes_before);
        assert_eq!(fs.opened_inodes.get(&id), Some(&1));

        fs.close(fd).unwrap();

        inode = inode_table::read(id, &mut fs.device, &fs.superblock, &fs.pool).unwrap();
        assert_eq!(inode.inode_type, Type::Free);
        assert_eq!(fs.opened_files[fd], None);
        assert_eq!(fs.opened_inodes.get(&id), None);
        assert_eq!(fs.superblock.free_inodes, inodes_before + 1);
        assert_eq!(fs.superblock.free_blocks, blocks_before);
        assert_eq!(fs.pool.in_use(), 0);
    }

    #[test]
    fn opened_file_with_two_names_is_freed_only_after_close(){
        let image = temp_image("opened_file_with_two_names_is_freed_only_after_close");
        let device = FileDevice::create(image.to_str().unwrap(), 4 * MIB).unwrap();
        let pool = BufferPooll::new();
        let mut fs = Fs::format_fs(device, pool, None).unwrap();

        let id = fs.create_file("a").unwrap();
        fs.link("a", "b").unwrap();
        let inodes_before = fs.superblock.free_inodes;
        let blocks_before = fs.superblock.free_blocks;

        let fd = fs.open("a").unwrap();

        fs.unlink("a").unwrap();
        assert_eq!(fs.stat_file("b").unwrap().nlink, 1);
        assert_eq!(fs.superblock.free_inodes, inodes_before);

        fs.unlink("b").unwrap();
        assert_eq!(fs.stat_file("b"), Err(FsError::ENOENT));
        let inode = inode_table::read(id, &mut fs.device, &fs.superblock, &fs.pool).unwrap();
        assert_eq!(inode.nlink, 0);
        assert_eq!(fs.superblock.free_inodes, inodes_before);

        fs.close(fd).unwrap();

        assert!(fs.opened_inodes.is_empty());
        assert_eq!(fs.superblock.free_inodes, inodes_before + 1);
        assert_eq!(fs.superblock.free_blocks, blocks_before);
        assert_eq!(fs.ls().unwrap().len(), 2);
        assert_eq!(fs.pool.in_use(), 0);
    }
}