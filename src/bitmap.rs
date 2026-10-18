use crate::raw_device::{BLOCK, BLOCK_SIZE};

pub const BITS_PER_BLOCK: usize= BLOCK_SIZE * 8;

pub fn get_bit(block: &BLOCK, bit: usize) -> bool{
    let b = block[bit / 8];
    let mask = 1u8 <<(bit % 8);
    b & mask != 0
}

pub fn set_bit(block: &mut BLOCK, bit: usize, value: bool){
    let b = block[bit / 8];
    let shift = bit % 8;
    let mask = 1u8 << shift;
    let new_byte =  if value  { b | mask } else {  b & !mask };
    block[bit / 8] = new_byte;
}

pub fn count_zero_bits(block: &BLOCK) -> usize{
    block.iter().map(| &b | b.count_zeros() as usize ).sum()
}

pub fn mark_tail_used(block: &mut BLOCK, valid_bits: usize){
    for n in valid_bits..BITS_PER_BLOCK{
        set_bit(block, n, true);
    }
}

pub fn find_zero_bit(block: &BLOCK, start:usize) -> Option<usize>{
    let normalized_start = start % BITS_PER_BLOCK;
    for n in normalized_start..BITS_PER_BLOCK{
        let bit = get_bit(block, n);
        if !bit { return Some(n)}
    }
    for n in 0..normalized_start{
        let bit = get_bit(block, n);
        if !bit { return Some(n)}
    }
    None
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn set_bit_correct(){
        let mut buff = [0u8; BLOCK_SIZE];

        set_bit(&mut buff, 0, true);
        assert_eq!(get_bit(&buff, 0), true);
        set_bit(&mut buff, 0, false);
        assert_eq!(get_bit(&buff, 0), false);

        set_bit(&mut buff, 7, true);
        assert_eq!(get_bit(&buff, 7), true);
        set_bit(&mut buff, 7, false);
        assert_eq!(get_bit(&buff, 7), false);

        set_bit(&mut buff, 8, true);
        assert_eq!(get_bit(&buff, 8), true);
        set_bit(&mut buff, 8, false);
        assert_eq!(get_bit(&buff, 8), false);

        set_bit(&mut buff, 100, true);
        assert_eq!(get_bit(&buff, 100), true);
        set_bit(&mut buff, 100, false);
        assert_eq!(get_bit(&buff, 100), false);

        set_bit(&mut buff, BITS_PER_BLOCK - 1, true);
        assert_eq!(get_bit(&buff, BITS_PER_BLOCK - 1), true);
        set_bit(&mut buff, BITS_PER_BLOCK - 1, false);
        assert_eq!(get_bit(&buff, BITS_PER_BLOCK - 1), false);
    }

    #[test]
    fn find_bit_correct(){
        let mut buff = [0u8; BLOCK_SIZE];

        set_bit(&mut buff, 0, true);
        set_bit(&mut buff, 1, true);
        set_bit(&mut buff, 5, true);

        assert_eq!(find_zero_bit(&buff, 2), Some(2));
        assert_eq!(find_zero_bit(&buff, 0), Some(2));
        assert_eq!(find_zero_bit(&buff, 5), Some(6));
        assert_eq!(find_zero_bit(&buff, BITS_PER_BLOCK), Some(2));
    }

    #[test]
    fn transition_across_end(){
        let mut buff = [0xffu8; BLOCK_SIZE];
        set_bit(&mut buff, 3, false);
        assert_eq!(find_zero_bit(&buff, 4), Some(3));
    }

    #[test]
    fn block_full(){
        let buff = [0xffu8; BLOCK_SIZE];

        assert_eq!(find_zero_bit(&buff, 4), None);
        assert_eq!(count_zero_bits(&buff), 0);
    }

    #[test]
    fn tail_is_never_used(){
        let mut buff = [0u8; BLOCK_SIZE];

        mark_tail_used(&mut buff, 10);
        assert_eq!(count_zero_bits(&buff), 10);

        for n in 0..10 {
            assert_eq!(find_zero_bit(&buff, 0), Some(n));
            set_bit(&mut buff, n, true);
        }

        assert_eq!(find_zero_bit(&buff, 0), None);
    }
}