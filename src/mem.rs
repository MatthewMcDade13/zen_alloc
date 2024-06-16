pub struct BlockVec {
    block_size: usize,
    cap: usize,
    nblocks: usize,
    buf: *mut u8,
}
