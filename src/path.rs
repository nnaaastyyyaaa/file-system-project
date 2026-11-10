use crate::{directory::lookup, error::{FsError, FsResult}, inode::{Inode, Type}, inode_table, mem::BufferPooll, raw_device::RawDevice, superblock::{ROOT_INODE, Superblock}};

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

fn resolve_path< D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, start: Inode, path: &str) -> FsResult<(Inode, Vec<String>)>{
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

    for n in 0..len - 1{
        if curr_inode.inode_type != Type::Dir {
            return Err(FsError::ENOTDIR);
        }
        let dir = &components[n];
        let id  = lookup(device, superblock, &mut curr_inode, pool, dir)?;
        curr_inode = inode_table::read(id, device, superblock, pool)?;
    }
    if curr_inode.inode_type != Type::Dir {
        return Err(FsError::ENOTDIR);
    }
    Ok((curr_inode, components))
}

pub fn resolve_parent<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock,cwd: u32, path: &str) -> FsResult<(String, u32)>{

    let start_dir = resolve_start_dir(device, pool, superblock , cwd, path )?;
    let (current_inode, components) = resolve_path(device, pool, superblock, start_dir, path)?;

    if components.is_empty() {
        if path.starts_with('/') {
            return Ok((".".to_string(), ROOT_INODE));
        }
        return Err(FsError::EINVAL);
    }

    let file = String::from(components[components.len() - 1].clone());
    
    Ok((file, current_inode.id))
}

pub fn resolve_full<D: RawDevice>(device: &mut D, pool: &BufferPooll, superblock: &mut Superblock, cwd: u32, path: &str) -> FsResult<u32>{
    let start_dir = resolve_start_dir(device, pool, superblock , cwd, path )?;
    let (mut current_inode, components) = resolve_path(device, pool, superblock, start_dir, path)?;

    if components.is_empty() {
        if path.starts_with('/') {
            return Ok(ROOT_INODE);
        }
        return Err(FsError::EINVAL);
    }

    let file  = Some(components[components.len() - 1].clone()).unwrap();

    let id = lookup(device, superblock, &mut current_inode, pool, &file)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        alloc::alloc_inode,
        directory::add_entry,
        raw_device::FileDevice,
        test_utils::{MIB, full_image},
    };

    struct Tree {
        a: u32,
        b: u32,
        f: u32,
        g: u32,
        top: u32,
    }

    fn mk_node<D: RawDevice>(device: &mut D, pool: &BufferPooll, sb: &mut Superblock, parent: u32, name: &str, kind: Type,) -> u32 {
        let id = alloc_inode(sb, pool, device).unwrap();
        let mut inode = Inode::empty(id);
        inode.inode_type = kind;
        inode.nlink = 1;
        inode.mode = 0o755;
        inode_table::write(&inode, device, sb, pool).unwrap();

        if kind == Type::Dir {
            add_entry(device, sb, &mut inode, pool, id, ".").unwrap();
            add_entry(device, sb, &mut inode, pool, parent, "..").unwrap();
        }

        let mut p = inode_table::read(parent, device, sb, pool).unwrap();
        add_entry(device, sb, &mut p, pool, id, name).unwrap();
        id
    }

    fn setup(name: &str) -> (FileDevice, BufferPooll, Superblock, Tree) {
        let (mut device, pool, mut sb) = full_image(name, 4 * MIB);
        let top = mk_node(&mut device, &pool, &mut sb, ROOT_INODE, "top", Type::File);
        let a = mk_node(&mut device, &pool, &mut sb, ROOT_INODE, "a", Type::Dir);
        let g = mk_node(&mut device, &pool, &mut sb, a, "g", Type::File);
        let b = mk_node(&mut device, &pool, &mut sb, a, "b", Type::Dir);
        let f = mk_node(&mut device, &pool, &mut sb, b, "f", Type::File);
        assert_eq!(pool.in_use(), 0);
        (device, pool, sb, Tree { a, b, f, g, top })
    }

    #[test]
    fn parent_single_component_absolute() {
        let (mut d, pool, mut sb, _t) = setup("rp_single_abs");
        let r = resolve_parent(&mut d, &pool, &mut sb, ROOT_INODE, "/top");
        assert_eq!(r, Ok(("top".to_string(), ROOT_INODE)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_nested_absolute() {
        let (mut d, pool, mut sb, t) = setup("rp_nested_abs");
        let r = resolve_parent(&mut d, &pool, &mut sb, ROOT_INODE,  "/a/b/f");
        assert_eq!(r, Ok(("f".to_string(), t.b)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_nested_relative_from_cwd() {
        let (mut d, pool, mut sb, t) = setup("rp_nested_rel");
        let r = resolve_parent( &mut d, &pool, &mut sb, t.a, "b/f");
        assert_eq!(r, Ok(("f".to_string(), t.b)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_absolute_ignores_cwd() {
        let (mut d, pool, mut sb, t) = setup("rp_abs_ignores_cwd");
        let r = resolve_parent(&mut d, &pool, &mut sb, t.b,  "/top");
        assert_eq!(r, Ok(("top".to_string(), ROOT_INODE)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_extra_and_trailing_slashes() {
        let (mut d, pool, mut sb, t) = setup("rp_slashes");
        let r = resolve_parent( &mut d, &pool, &mut sb, ROOT_INODE, "//a///b//");
        assert_eq!(r, Ok(("b".to_string(), t.a)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_empty_and_root_works_correctly() {
        let (mut d, pool, mut sb, t) = setup("rp_empty_root");
        assert_eq!(resolve_parent(&mut d, &pool, &mut sb, t.a, ""), Err(FsError::EINVAL));
        assert_eq!(resolve_parent(&mut d, &pool, &mut sb, t.a, "/"), Ok((".".to_string(), ROOT_INODE)));
        assert_eq!(resolve_parent(&mut d, &pool, &mut sb, t.a, "///"), Ok((".".to_string(), ROOT_INODE)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_missing_intermediate_is_enoent() {
        let (mut d, pool, mut sb, _t) = setup("rp_missing_mid");
        assert_eq!(
            resolve_parent(&mut d, &pool, &mut sb, ROOT_INODE, "/nope/x"),
            Err(FsError::ENOENT)
        );
        assert_eq!(
            resolve_parent(&mut d, &pool, &mut sb, ROOT_INODE, "/a/nope/x/y"),
            Err(FsError::ENOENT)
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_cwd_is_file_is_enotdir() {
        let (mut d, pool, mut sb, t) = setup("rp_cwd_file");
        assert_eq!(
            resolve_parent(&mut d, &pool, &mut sb, t.top, "x"),
            Err(FsError::ENOTDIR)
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_follows_dotdot() {
        let (mut d, pool, mut sb, t) = setup("rp_dotdot");
        let r = resolve_parent( &mut d, &pool, &mut sb, ROOT_INODE, "/a/b/../g");
        assert_eq!(r, Ok(("g".to_string(), t.a)));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn parent_follows_dot() {
        let (mut d, pool, mut sb, t) = setup("rp_dot");
        let r = resolve_parent( &mut d, &pool, &mut sb, t.a, "./b/./f");
        assert_eq!(r, Ok(("f".to_string(), t.b)));
        assert_eq!(pool.in_use(), 0);
    }


    #[test]
    fn full_finds_file_absolute() {
        let (mut d, pool, mut sb, t) = setup("rf_abs");
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, "/a/b/f"), Ok(t.f));
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, "/top"), Ok(t.top));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_finds_file_relative() {
        let (mut d, pool, mut sb, t) = setup("rf_rel");
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, t.a, "g"), Ok(t.g));
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, t.a, "b/f"), Ok(t.f));
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_missing_intermediate_is_enoent() {
        let (mut d, pool, mut sb, _t) = setup("rf_missing_mid");
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, "/nope/f"),
            Err(FsError::ENOENT)
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_file_as_intermediate_is_enotdir() {
        let (mut d, pool, mut sb, _t) = setup("rf_file_mid");
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, "/top/x"),
            Err(FsError::ENOTDIR)
        );
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, "/a/b/f/x/y"),
            Err(FsError::ENOTDIR)
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_directory_return_id() {
        let (mut d, pool, mut sb, t) = setup("rf_dir");
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, "/a").unwrap(),
            t.a
        );
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, ROOT_INODE, "/a/b/").unwrap(),
            t.b
        );
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_empty_and_return_error_root_retun_root_id() {
        let (mut d, pool, mut sb, t) = setup("rf_empty_root");
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, t.a, ""), Err(FsError::EINVAL));
        assert_eq!(resolve_full(&mut d, &pool, &mut sb, t.a, "/").unwrap(), ROOT_INODE);
        assert_eq!(pool.in_use(), 0);
    }

    #[test]
    fn full_cwd_is_file_is_enotdir() {
        let (mut d, pool, mut sb, t) = setup("rf_cwd_file");
        assert_eq!(
            resolve_full(&mut d, &pool, &mut sb, t.top, "x"),
            Err(FsError::ENOTDIR)
        );
        assert_eq!(pool.in_use(), 0);
    }
}