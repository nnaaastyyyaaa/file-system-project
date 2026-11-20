use crate::{directory::lookup, error::{FsError, FsResult}, file_io::read_file, inode::{Inode, Type}, inode_table, mem::BufferPooll, perm::{CurrentUser, X, check_access}, raw_device::{BLOCK_SIZE, RawDevice}, superblock::{ROOT_INODE, Superblock}};

fn resolve_start_dir<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, cwd: u32, path: &str) -> FsResult<Inode> {
    let start_dir =
    if path.starts_with("/"){
         ROOT_INODE
    } else {
         cwd
    };

    let inode = inode_table::read(start_dir, device, superblock, pool)?;

    Ok(inode)
}

fn resolve_path< D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, start: Inode, user: &CurrentUser, path: &str, follow_last: bool) -> FsResult<(Inode, Vec<String>)>{
    let mut components:Vec<String> = path.split('/').map(|el|String::from(el)).collect();
    components = components.iter().cloned().filter(|el| !el.is_empty()).collect();

    let len = components.len();
    if len == 0 {
        return Ok((start, components));
    }

    let mut curr_inode = start;
    if curr_inode.inode_type != Type::Dir { 
        return Err(FsError::ENOTDIR);
    }

    let mut links = 0;
    let mut n = 0;

    while components.len() > n {
        if components.len() == n + 1 && !follow_last {
            break;
        }

        if curr_inode.inode_type != Type::Dir {
            return Err(FsError::ENOTDIR);
        }
        check_access(&curr_inode, user, X)?;

        let dir = &components[n];
        let id  = lookup(device, superblock, &mut curr_inode, pool, dir)?;
        let mut next = inode_table::read(id, device, superblock, pool)?;
        if next.inode_type == Type::Symlink {
            links += 1;
            if links > 40 {
                return Err(FsError::ELOOP);
            }

            let mut buff = [0u8; BLOCK_SIZE];
            let size = next.size as usize;
            if size > BLOCK_SIZE {
                return Err(FsError::CORRUPT);
            }
            let read = read_file(device, pool, superblock, &mut next, 0, &mut buff[0..size])?;
            if read < size as u64 {
                return Err(FsError::CORRUPT);
            }

            let link = String::from_utf8(buff[..size].to_vec()).map_err(|_| FsError::EINVAL)?;
            if link.is_empty() {
                return Err(FsError::EINVAL);
            }

            let link_components:Vec<String> = link.split('/').map(|el|String::from(el)).filter(|el| !el.is_empty()).collect();
            if link.starts_with('/') {
                curr_inode = inode_table::read(ROOT_INODE, device, superblock, pool)?;
            }
            components.splice(0..=n, link_components);
            n = 0;
        }else{
            curr_inode = next;
            n += 1;
        }
    }

    if !follow_last && curr_inode.inode_type != Type::Dir {
        return Err(FsError::ENOTDIR);
    }
    Ok((curr_inode, components))
}

pub fn resolve_parent<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock,cwd: u32, user: &CurrentUser, path: &str) -> FsResult<(String, u32)>{

    let start_dir = resolve_start_dir(device, pool, superblock , cwd, path )?;
    let (current_inode, components) = resolve_path(device, pool, superblock, start_dir, user, path, false)?;

    if components.is_empty() {
        if path.starts_with('/') {
            return Ok((".".to_string(), ROOT_INODE));
        }
        return Err(FsError::EINVAL);
    }

    check_access(&current_inode, user, X)?;
    let file = String::from(components[components.len() - 1].clone());
    
    Ok((file, current_inode.id))
}

pub fn resolve_full<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, cwd: u32, user: &CurrentUser, path: &str) -> FsResult<u32>{
    if path.split('/').all(|c| c.is_empty()) {
        if path.starts_with('/') { 
            return Ok(ROOT_INODE) 
        } else { 
            return Err(FsError::EINVAL) 
        };
    }
    let start_dir = resolve_start_dir(device, pool, superblock , cwd, path )?;
    let (current_inode, _) = resolve_path(device, pool, superblock, start_dir, user, path, true)?;
    Ok(current_inode.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        perm::ROOT_USER, test_utils::setup,
    };


    #[test]
    fn parent_single_component_absolute() {
        let (mut d, pool, mut sb, _t) = setup("rp_single_abs");
        let r = resolve_parent(&mut d, &pool, &mut sb, ROOT_INODE, &&ROOT_USER, "/top");
        assert_eq!(r, Ok(("top".to_string(), ROOT_INODE)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_nested_absolute() {
        let (mut d, pool, mut sb, t) = setup("rp_nested_abs");
        let r = resolve_parent(&mut d, &pool, &mut sb, ROOT_INODE, &&ROOT_USER,  "/a/b/f");
        assert_eq!(r, Ok(("f".to_string(), t.b)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_nested_relative_from_cwd() {
        let (mut d, pool, mut sb, t) = setup("rp_nested_rel");
        let r = resolve_parent( &mut d, &pool, &mut sb, t.a, &ROOT_USER, "b/f");
        assert_eq!(r, Ok(("f".to_string(), t.b)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_absolute_ignores_cwd() {
        let (mut d, pool, mut sb, t) = setup("rp_abs_ignores_cwd");
        let r = resolve_parent(&mut d, &pool, &mut sb, t.b, &ROOT_USER,  "/top");
        assert_eq!(r, Ok(("top".to_string(), ROOT_INODE)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_extra_and_trailing_slashes() {
        let (mut d, pool, mut sb, t) = setup("rp_slashes");
        let r = resolve_parent( &mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "//a///b//");
        assert_eq!(r, Ok(("b".to_string(), t.a)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_empty_and_root_works_correctly() {
        let (mut d, pool, mut sb, t) = setup("rp_empty_root");
        assert_eq!(resolve_parent(&mut d, &pool, &mut sb, t.a, &ROOT_USER, ""), Err(FsError::EINVAL));
        assert_eq!(resolve_parent(&mut d, &pool, &mut sb, t.a, &ROOT_USER, "/"), Ok((".".to_string(), ROOT_INODE)));
        assert_eq!(resolve_parent(&mut d, &pool, &mut sb, t.a, &ROOT_USER, "///"), Ok((".".to_string(), ROOT_INODE)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_missing_intermediate_is_enoent() {
        let (mut d, pool, mut sb, _t) = setup("rp_missing_mid");
        assert_eq!(
            resolve_parent(&mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/nope/x"),
            Err(FsError::ENOENT)
        );
        assert_eq!(
            resolve_parent(&mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/a/nope/x/y"),
            Err(FsError::ENOENT)
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_cwd_is_file_is_enotdir() {
        let (mut d, pool, mut sb, t) = setup("rp_cwd_file");
        assert_eq!(
            resolve_parent(&mut d, &pool, &mut sb, t.top, &ROOT_USER, "x"),
            Err(FsError::ENOTDIR)
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_follows_dotdot() {
        let (mut d, pool, mut sb, t) = setup("rp_dotdot");
        let r = resolve_parent( &mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/a/b/../g");
        assert_eq!(r, Ok(("g".to_string(), t.a)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_follows_dot() {
        let (mut d, pool, mut sb, t) = setup("rp_dot");
        let r = resolve_parent( &mut d, &pool, &mut sb, t.a, &ROOT_USER,  "./b/./f");
        assert_eq!(r, Ok(("f".to_string(), t.b)));
        assert_eq!(pool.in_use(), 0);
    }


    #[test]
    fn full_finds_file_absolute() {
        let (mut d, pool, mut sb, t) = setup("rf_abs");
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/a/b/f"), Ok(t.f));
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/top"), Ok(t.top));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_finds_file_relative() {
        let (mut d, pool, mut sb, t) = setup("rf_rel");
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, t.a, &ROOT_USER, "g"), Ok(t.g));
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, t.a, &ROOT_USER, "b/f"), Ok(t.f));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_missing_intermediate_is_enoent() {
        let (mut d, pool, mut sb, _t) = setup("rf_missing_mid");
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/nope/f"),
            Err(FsError::ENOENT)
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_file_as_intermediate_is_enotdir() {
        let (mut d, pool, mut sb, _t) = setup("rf_file_mid");
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/top/x"),
            Err(FsError::ENOTDIR)
        );
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/a/b/f/x/y"),
            Err(FsError::ENOTDIR)
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_directory_return_id() {
        let (mut d, pool, mut sb, t) = setup("rf_dir");
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/a").unwrap(),
            t.a
        );
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, &ROOT_USER, "/a/b/").unwrap(),
            t.b
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_empty_and_return_error_root_retun_root_id() {
        let (mut d, pool, mut sb, t) = setup("rf_empty_root");
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, t.a, &ROOT_USER, ""), Err(FsError::EINVAL));
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, t.a, &ROOT_USER, "/").unwrap(), ROOT_INODE);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_cwd_is_file_is_enotdir() {
        let (mut d, pool, mut sb, t) = setup("rf_cwd_file");
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, t.top, &ROOT_USER, "x"),
            Err(FsError::ENOTDIR)
        );
        assert_eq!(pool.in_use(), 0);
    }
}