use std::{cell::RefCell, ops::{Deref, DerefMut}};

use crate::{error::{FsError, FsResult}, raw_device::{BLOCK, BLOCK_SIZE}};

pub const POOLL_BUFFERS: usize = 8;

#[derive(Debug, PartialEq)]
pub struct BufferPooll {
   pub  free: RefCell<Vec<Box<BLOCK>>>,
   pub  allocated: usize,
}

#[derive(Debug, PartialEq)]
pub struct PoolledBuff<'a>{
    pool: &'a BufferPooll,
    buff: Option<Box<BLOCK>>

}

impl BufferPooll {
    pub fn new() -> Self {
        let mut vector = Vec::with_capacity(POOLL_BUFFERS);
        let buff = [0u8;  BLOCK_SIZE];

        for _ in 0..8{
            vector.push(Box::new(buff))
        }

        Self { free: RefCell::new(vector), allocated: POOLL_BUFFERS * BLOCK_SIZE }
    }

    pub fn take(&self) -> FsResult<PoolledBuff<'_>> {
        let block = self.free.borrow_mut().pop().ok_or(FsError::ENOMEM)?;
        Ok( PoolledBuff { pool: self, buff: Some(block) })
    }

    pub fn available(&self) -> usize {
        self.free.borrow().len()
    }

    pub fn in_use(&self) -> usize {
        POOLL_BUFFERS - self.available()
    }

    pub fn allocated_bytes(&self) -> usize {
        self.allocated
    }
}

impl Default for BufferPooll{
    fn default() -> Self{
        Self::new()
    }
}

impl Deref for PoolledBuff<'_>  {
    type Target = BLOCK;

    fn deref(&self) -> &Self::Target{
        self.buff.as_deref().unwrap()
    }
}

impl DerefMut for PoolledBuff<'_>{
    fn deref_mut(&mut self) -> &mut Self::Target {
       self.buff.as_deref_mut().unwrap()
    }
}

impl Drop for PoolledBuff<'_> {
    fn drop(&mut self) {
        if let Some(mut b) = self.buff.take() {
            b.fill(0);
            self.pool.free.borrow_mut().push(b);
        }
    }
}

#[cfg(test)]
mod tests{
    use crate::{error::FsError, mem::{BufferPooll, POOLL_BUFFERS, PoolledBuff}};

    #[test]
    fn pull_return_only_eight_buffers(){
        let pull = BufferPooll::default();
        let mut buffers = Vec::new();
        for _ in 0..8{
            buffers.push(pull.take().unwrap())
        }

        assert_eq!(pull.take(), Err(FsError::ENOMEM))
    }

    #[test]
    fn pull_return_to_buff(){
        let pull = BufferPooll::default();
        {
            let mut buffers = Vec::new();
                buffers.push(pull.take());
                buffers.push(pull.take());
                assert_eq!(pull.in_use(), 2)
        }
        assert_eq!(pull.available(), 8)
    }

    #[test]
    fn returned_buffer_is_zeroed(){
        let pull = BufferPooll::default();
        {
            let mut buff = pull.take().unwrap();
            buff.fill(0x88)
        }

        let all: Vec<PoolledBuff<'_>> = (0..POOLL_BUFFERS).map(|_| pull.take().unwrap()).collect();
        assert!(all.iter().all(|b| b.iter().all(|&x| x == 0)))

    }
}