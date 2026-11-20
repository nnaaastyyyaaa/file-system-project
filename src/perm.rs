use crate::{error::{FsError, FsResult}, inode::Inode};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurrentUser {
    pub uid: u32,
    pub gid: u32
}

pub const ROOT_USER: CurrentUser = CurrentUser { uid: 0, gid: 0};

pub const R: u16 = 4;
pub const W: u16 = 2;
pub const X: u16 = 1;

pub fn check_access(inode: &Inode, curr_user: &CurrentUser, need: u16) -> FsResult<()>{
    if curr_user.uid == ROOT_USER.uid {
        return Ok(());
    }

    let mut class_bits = inode.mode & 0o007;

    if curr_user.uid == inode.uid {
        class_bits = (inode.mode >> 6) & 0o007;
    } else if curr_user.gid == inode.gid {
        class_bits = (inode.mode >> 3) & 0o007;
    }

    if class_bits & need == need {
        return Ok(());
    }

    Err(FsError::EACCES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inode_with(mode: u16, uid: u32, gid: u32) -> Inode {
        let mut inode = Inode::empty(2);
        inode.mode = mode;
        inode.uid = uid;
        inode.gid = gid;
        inode
    }

    const OWNER: CurrentUser = CurrentUser { uid: 10, gid: 20 };
    const GROUP_MEMBER: CurrentUser = CurrentUser { uid: 11, gid: 20 };
    const STRANGER: CurrentUser = CurrentUser { uid: 11, gid: 99 };

    #[test]
    fn root_bypasses_everything() {
        let inode = inode_with(0o000, 10, 20);
        assert_eq!(check_access(&inode, &ROOT_USER, R | W | X), Ok(()));
    }

    #[test]
    fn class_is_chosen_by_uid_then_gid_then_other() {
        let inode = inode_with(0o437, 10, 20);

        assert_eq!(check_access(&inode, &OWNER, R), Ok(()));
        assert_eq!(check_access(&inode, &OWNER, W), Err(FsError::EACCES));

        assert_eq!(check_access(&inode, &GROUP_MEMBER, W | X), Ok(()));
        assert_eq!(check_access(&inode, &GROUP_MEMBER, R), Err(FsError::EACCES));

        assert_eq!(check_access(&inode, &STRANGER, R | W | X), Ok(()));
    }

    #[test]
    fn owner_is_denied_even_if_group_and_other_allow() {
        let inode = inode_with(0o044, 10, 20);

        assert_eq!(check_access(&inode, &OWNER, R), Err(FsError::EACCES));
        assert_eq!(check_access(&inode, &GROUP_MEMBER, R), Ok(()));
        assert_eq!(check_access(&inode, &STRANGER, R), Ok(()));
    }

    #[test]
    fn all_needed_bits_must_be_present() {
        let inode = inode_with(0o200, 10, 20);

        assert_eq!(check_access(&inode, &OWNER, W), Ok(()));
        assert_eq!(check_access(&inode, &OWNER, W | X), Err(FsError::EACCES));
    }

    #[test]
    fn other_class_bits_do_not_leak() {
        let inode = inode_with(0o070, 10, 20);

        assert_eq!(check_access(&inode, &GROUP_MEMBER, R | W | X), Ok(()));
        assert_eq!(check_access(&inode, &OWNER, R), Err(FsError::EACCES));
        assert_eq!(check_access(&inode, &STRANGER, R), Err(FsError::EACCES));
    }
}