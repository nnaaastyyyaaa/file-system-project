use std::{collections::HashMap};

use crate::{alloc::{alloc_inode, free_inode}, directory::{add_entry, lookup}, error::{FsError, FsResult}, inode::{Inode, Type, now_ms}, inode_table, mem::BufferPooll, mkfs::mkfs, raw_device::RawDevice, superblock::{ROOT_INODE, Superblock}};

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
    }
}