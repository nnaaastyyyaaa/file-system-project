use crate::{dirent::{ENTRY_SIZE, decode_entry, encode_entry}, error::{FsError, FsResult}, file_io::{read_file, write_file}, inode::Inode, mem::BufferPooll, raw_device::{RawDevice}, superblock::{Superblock}};

fn find_entry<D: RawDevice>(device: &mut D, superblock: &mut Superblock, inode: &mut Inode, pool: &BufferPooll, name: &str) -> FsResult<(u32, u64)> {
    let records = inode.size / ENTRY_SIZE as u64;

    for n in 0..records {
        let pos = n * ENTRY_SIZE as u64;
        let mut buff = pool.take()?;
        let read = read_file(device, pool, superblock, inode, pos, &mut buff[..ENTRY_SIZE])?;
        if read != ENTRY_SIZE as u64 {
            return Err(FsError::CORRUPT);
        }
        if let Some((id, file_name)) = decode_entry(&buff[..ENTRY_SIZE])? {
            if file_name == name {
                return Ok((id, pos));
            }
        }
    }
    Err(FsError::ENOENT)
}

pub fn lookup<D: RawDevice>(device: &mut D, superblock: &mut Superblock, inode: &mut Inode, pool: &BufferPooll, name: &str) -> FsResult<u32> {
    let (id, _pos) = find_entry(device, superblock, inode, pool, name)?;
    Ok(id)
}

pub fn add_entry<D: RawDevice>(device: &mut D, superblock: &mut Superblock, inode: &mut Inode, pool: &BufferPooll, id: u32, name: &str) -> FsResult<()> {
    match lookup(device, superblock, inode, pool, name) {
        Err(FsError::ENOENT) => {},
        Err(e) => return Err(e),
        _name => {
            return Err(FsError::EEXIST);
        }
    }

    let records = inode.size / ENTRY_SIZE as u64;
    let mut offset = inode.size;
    let mut chunk = [0; ENTRY_SIZE];

    for n in 0..records{
       let read_bytes = read_file(device, pool, superblock, inode, n * ENTRY_SIZE as u64, &mut chunk)?;
       if read_bytes != ENTRY_SIZE as u64 {
        return Err(FsError::CORRUPT);
       }
       if decode_entry(&chunk)?.is_none() {
        offset = n * ENTRY_SIZE as u64;
        break;
       }
    }

    let mut entry_buff = pool.take()?;
    encode_entry(&mut entry_buff[..ENTRY_SIZE], id, name)?;

    let written = write_file(device, pool, superblock, inode, offset, &entry_buff[..ENTRY_SIZE])?;
    if written != ENTRY_SIZE as u64 {
        return Err(FsError::CORRUPT);
    }

    Ok(())
}

pub fn remove_entry<D: RawDevice>(device: &mut D, superblock: &mut Superblock, inode: &mut Inode, pool: &BufferPooll, name: &str) -> FsResult<u32>{
    if name == "." || name == ".." {
        return Err(FsError::EINVAL)
    }

    let (id, pos) = find_entry(device, superblock, inode, pool, name)?; 

    let mut buff = pool.take()?;
    buff.fill(0);
    let written = write_file(device, pool, superblock, inode, pos, &buff[..ENTRY_SIZE])?;

    if written != ENTRY_SIZE as u64 {
        return Err(FsError::CORRUPT)
    }

    Ok(id)
}

pub fn list<D: RawDevice>(device: &mut D, superblock: &mut Superblock, inode: &mut Inode, pool: &BufferPooll) -> FsResult<Vec<(u32, String)>>{
    let num_records = inode.size / ENTRY_SIZE as u64;
    let mut v: Vec<(u32, String)> = Vec::new();

    for n in 0..num_records {
        let pos = n * ENTRY_SIZE as u64;
        let mut buff = pool.take()?;
        let read = read_file(device, pool, superblock, inode, pos, &mut buff[..ENTRY_SIZE])?;
        if read != ENTRY_SIZE as u64 {
            return Err(FsError::CORRUPT);
        }
        let Some((id, name)) = decode_entry(&buff[..ENTRY_SIZE])? else {
            continue;
        };
        v.push((id, name));
    }
     Ok(v)
}



#[cfg(test)]
mod tests{
    use super::*;
    use crate::{inode_table, raw_device::BLOCK_SIZE, test_utils::{MIB, full_image}};

    #[test]
    fn lookup_return_id_for_root_after_mkfs(){
        let (mut device, pool, mut superblock) = full_image("lookup_return_id_for_root_after_mkfs", 4 *MIB);
        let mut inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();
        assert_eq!(lookup(&mut device, &mut superblock, &mut inode, &pool, ".").unwrap(), 1);
        assert_eq!(lookup(&mut device, &mut superblock, &mut inode, &pool, "..").unwrap(), 1);
        assert_eq!(pool.in_use(), 0)
    }

    #[test]
    fn unknown_name_return_error(){
        let (mut device, pool, mut superblock) = full_image("unknown_name_return_error", 4 *MIB);
        let mut inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();
        assert_eq!(lookup(&mut device, &mut superblock, &mut inode, &pool, "abcde"), Err(FsError::ENOENT));
        assert_eq!(pool.in_use(), 0)
    }

    #[test]
    fn find_new_written_file(){
        let (mut device, pool, mut superblock) = full_image("find_new_written_file", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, "data").unwrap();
        assert_eq!(lookup(&mut device, &mut superblock, &mut root_inode, &pool, "data").unwrap(), 2);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn creating_entry_with_existing_name(){
        let (mut device, pool, mut superblock) = full_image("creating_entry_with_existing_name", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, "data").unwrap();
        assert_eq!(add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 3, "data"), Err(FsError::EEXIST));
        assert_eq!(lookup(&mut device, &mut superblock, &mut root_inode, &pool, "data").unwrap(), 2);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn new_entry_correctly_created_in_block() {
        let (mut device, pool, mut superblock) = full_image("new_entry_correctly_created", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, "data").unwrap();
        assert_eq!(root_inode.size, 3 * ENTRY_SIZE as u64);
        let mut buff = pool.take().unwrap();
        read_file(&mut device, &pool, &mut superblock, &mut root_inode, 128, &mut buff[..64]).unwrap();
        let (id, name) = decode_entry(&buff[..64]).unwrap().unwrap();
        assert_eq!(id, 2);
        assert_eq!(name, "data");
        drop(buff);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn directory_size_increases_correctly() {
        let (mut device, pool, mut superblock) = full_image("directory_size_increases_correctly", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        for n in 2..BLOCK_SIZE/ENTRY_SIZE{
            add_entry(&mut device, &mut superblock, &mut root_inode, &pool, n as u32, &format!("data{n}")).unwrap();
        }
        assert_eq!(root_inode.size, BLOCK_SIZE as u64);
        assert_ne!(root_inode.direct[0], 0);
        assert_eq!(root_inode.direct[1], 0);

        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 64, "data64").unwrap();
        assert_eq!(root_inode.size, BLOCK_SIZE as u64 + ENTRY_SIZE as u64);
        assert_ne!(root_inode.direct[0], 0);
        assert_ne!(root_inode.direct[1], 0);
    }

    #[test]
    fn incoreect_name_is_not_changes_directory() {
        let (mut device, pool, mut superblock) = full_image("incoreect_name_is_not_changes_directory", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        assert_eq!(add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, "a/b"), Err(FsError::EINVAL));
        assert_eq!(add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, &"a".repeat(65)), Err(FsError::ENAMETOOLONG));
        assert_eq!(lookup(&mut device, &mut superblock, &mut root_inode, &pool, "a/b"), Err(FsError::ENOENT));
        assert_eq!(root_inode.size, ENTRY_SIZE as u64 * 2);
    }

    #[test]
    fn add_entry_writes_changes_to_disk() {
        let (mut device, pool, mut superblock) = full_image("new_entry_correctly_created", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, "data").unwrap();
        let block = root_inode.direct[0];
        let mut buff = pool.take().unwrap();
        device.read(block, &mut buff).unwrap();

        assert_eq!(decode_entry(&buff[ENTRY_SIZE * 2 ..ENTRY_SIZE * 3]).unwrap().unwrap(), (2, "data".to_string()));
        assert_eq!(decode_entry(&buff[ENTRY_SIZE..ENTRY_SIZE * 2]).unwrap().unwrap(), (1, "..".to_string()));
        assert_eq!(decode_entry(&buff[ENTRY_SIZE * 3..ENTRY_SIZE * 4]).unwrap(), None);
        drop(buff);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn remove_entry_removes_entry_from_directory() {
        let (mut device, pool, mut superblock) = full_image("remove_entry_removes_entry_from_directory", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, "data").unwrap();
        assert_eq!(lookup(&mut device, &mut superblock, &mut root_inode, &pool, "data").unwrap(), 2);

        remove_entry(&mut device, &mut superblock, &mut root_inode, &pool, "data").unwrap();
        assert_eq!(lookup(&mut device, &mut superblock, &mut root_inode, &pool, "data"), Err(FsError::ENOENT));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn removing_root_entries_return_error() {
        let (mut device, pool, mut superblock) = full_image("remove_root_entries_return_error", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        assert_eq!(remove_entry(&mut device, &mut superblock, &mut root_inode, &pool, "."), Err(FsError::EINVAL));
        assert_eq!(remove_entry(&mut device, &mut superblock, &mut root_inode, &pool, ".."), Err(FsError::EINVAL));

        assert_eq!(lookup(&mut device, &mut superblock, &mut root_inode, &pool, ".").unwrap(), 1);
        assert_eq!(lookup(&mut device, &mut superblock, &mut root_inode, &pool, "..").unwrap(), 1);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn remove_entry_writes_changes_to_disk() {
        let (mut device, pool, mut superblock) = full_image("remove_entry_writes_changes_to_disk", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, "data").unwrap();
        let block = root_inode.direct[0];

        remove_entry(&mut device, &mut superblock, &mut root_inode, &pool, "data").unwrap();
        let mut buff = pool.take().unwrap();
        device.read(block, &mut buff).unwrap();
        assert_eq!(decode_entry(&buff[ENTRY_SIZE * 2 ..ENTRY_SIZE * 3]).unwrap(), None);
        drop(buff);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn adding_after_removing_writes_to_the_same_place() {
        let (mut device, pool, mut superblock) = full_image("adding_after_removing_writes_to_the_same_place", 4 *MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, "data").unwrap();
        let block = root_inode.direct[0];
        let mut buff = pool.take().unwrap();
        device.read(block, &mut buff).unwrap();
        assert_eq!(decode_entry(&buff[ENTRY_SIZE * 2 ..ENTRY_SIZE * 3]).unwrap().unwrap(), (2, "data".to_string()));

        remove_entry(&mut device, &mut superblock, &mut root_inode, &pool, "data").unwrap();
        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 3, "data").unwrap();
        device.read(block, &mut buff).unwrap();
        assert_eq!(decode_entry(&buff[ENTRY_SIZE * 2 ..ENTRY_SIZE * 3]).unwrap().unwrap(), (3, "data".to_string()));

        drop(buff);
        assert_eq!(lookup(&mut device, &mut superblock, &mut root_inode, &pool, "data").unwrap(), 3);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn remove_entry_from_second_block() {
        let (mut device, pool, mut superblock) = full_image("remove_entry_second_block", 4 * MIB);
        let mut root = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        for n in 2..(BLOCK_SIZE / ENTRY_SIZE) as u32 {
            add_entry(&mut device, &mut superblock, &mut root, &pool, n, &format!("data{n}")).unwrap();
        }
        add_entry(&mut device, &mut superblock, &mut root, &pool, 100, "extra").unwrap();
 
        let removed = remove_entry(&mut device, &mut superblock, &mut root, &pool, "extra").unwrap();
        assert_eq!(removed, 100);

        assert_eq!(lookup(&mut device, &mut superblock, &mut root, &pool, "extra"), Err(FsError::ENOENT));

        assert_eq!(lookup(&mut device, &mut superblock, &mut root, &pool, ".").unwrap(), 1);
        assert_eq!(lookup(&mut device, &mut superblock, &mut root, &pool, "..").unwrap(), 1);
        assert_eq!(lookup(&mut device, &mut superblock, &mut root, &pool, "data2").unwrap(), 2);
        assert_eq!(lookup(&mut device, &mut superblock, &mut root, &pool, "data63").unwrap(), 63);

        assert_eq!(root.size, (BLOCK_SIZE + ENTRY_SIZE) as u64);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn list_after_mkfs_returns_root_entries() {
        let (mut device, pool, mut superblock) = full_image("list_after_mkfs_returns_root_entries", 4 * MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        let entries = list(&mut device, &mut superblock, &mut root_inode, &pool).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], (1, ".".to_string()));
        assert_eq!(entries[1], (1, "..".to_string()));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn list_shows_added_and_hides_removed(){
        let (mut device, pool, mut superblock) = full_image("list_shows_added_and_hides_removed", 4 * MIB);
        let mut root_inode = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        add_entry(&mut device, &mut superblock, &mut root_inode, &pool, 2, "data").unwrap();
        let mut entries = list(&mut device, &mut superblock, &mut root_inode, &pool).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], (1, ".".to_string()));
        assert_eq!(entries[1], (1, "..".to_string()));
        assert_eq!(entries[2], (2, "data".to_string()));

        remove_entry(&mut device, &mut superblock, &mut root_inode, &pool, "data").unwrap();
        entries = list(&mut device, &mut superblock, &mut root_inode, &pool).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], (1, ".".to_string()));
        assert_eq!(entries[1], (1, "..".to_string()));

        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn list_includes_entries_from_second_block() {
        let (mut device, pool, mut superblock) = full_image("list_includes_entries_from_second_block", 4 * MIB);
        let mut root = inode_table::read(1, &mut device, &superblock, &pool).unwrap();

        for n in 2..(BLOCK_SIZE / ENTRY_SIZE) as u32 {
            add_entry(&mut device, &mut superblock, &mut root, &pool, n, &format!("data{n}")).unwrap();
        }
        add_entry(&mut device, &mut superblock, &mut root, &pool, 100, "extra").unwrap();

        let entries = list(&mut device, &mut superblock, &mut root, &pool).unwrap();
        assert_eq!(entries.len(), 65);
        assert_eq!(entries[64], (100, "extra".to_string()));
        assert_eq!(pool.in_use(), 0);
    }
}