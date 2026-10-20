use std::io;

#[derive(Debug, PartialEq)]
pub enum FsError{
    EEXIST,
    ENOENT,
    EACCESS,
    EISDIR,
    ENOSPC, 
    EFBIG,
    ENOTDIR,
    ENOTEMPTY,
    ENOMEM,
    EPERM,
    EINVAL,
    ELOOP,
    ENAMETOOLONG,
    FAULTYCRASH,
    CORRUPT,
    Io(io::ErrorKind),
}

pub type FsResult<T> = Result<T, FsError>;

impl From<io::Error> for FsError {
    fn from(e: io::Error) -> Self{
        FsError::Io(e.kind())
    }
}