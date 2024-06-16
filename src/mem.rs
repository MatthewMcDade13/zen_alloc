use core::slice;
use std::{
    alloc::{alloc, alloc_zeroed, dealloc, Layout},
    marker::PhantomData,
    ops::Index,
};

#[derive(Debug)]
pub struct BlockVec {
    block_size: usize,
    len: usize,
    buf: *mut u8,

    _phantom: PhantomData<[u8]>,
}

impl BlockVec {
    pub fn new(block_size: usize) -> Self {
        Self {
            block_size,
            len: 0,
            buf: std::ptr::null_mut(),
            _phantom: PhantomData,
        }
    }

    pub const fn block_size(&self) -> usize {
        self.block_size + std::mem::size_of::<BlockMeta>()
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn nbytes(&self) -> usize {
        self.len * self.block_size()
    }

    #[inline]
    fn layout(&self) -> Layout {
        Self::new_layout(self.nbytes())
    }

    #[inline]
    fn new_layout(nbytes: usize) -> Layout {
        Layout::array::<u8>(nbytes).expect("Layout::array => nbytes too big")
    }

    pub fn with_capacity(block_size: usize, capacity: usize) -> Self {
        todo!()
    }

    /// Resizes inner buffer into a new allocated one with new size and
    /// memcpys the old vals into new allocated buffer, then frees old buffer.
    /// Allocates if new_size > self.len, otherwise self.len is decreased. (no need to
    /// allocated/deallocate if we dont need to on shrink )
    pub fn resize(&mut self, new_size: usize) {
        if new_size == 0 {
            self.len = 0;
        } else if new_size > self.len() {
            self.grow(new_size + self.len());
        } else if new_size < self.len() {
            self.shrink(self.len() - new_size)
        }
    }

    fn shrink(&mut self, nblocks: usize) {
        self.len -= nblocks;
    }
    //     unsafe {
    //         let layout_old = self.layout();
    //         let nbytes_old = self.nbytes();
    //
    //         self.len -= nblocks;
    //
    //         let layout_new = self.layout();
    //         let nbytes_new = self.nbytes();
    //         let dst = alloc_zeroed(layout_new);
    //
    //         if dst.is_null() {
    //             panic!("Out of memory!!");
    //         }
    //
    //         let new_buf = copy_raw(dst, nbytes_new, self.buf, nbytes_old);
    //
    //         dealloc(self.buf, layout_old);
    //         self.buf = new_buf;
    //         // let dst = slice::from_raw_parts_mut(dst, nbytes_new);
    //         // let src = slice::from_raw_parts(self.buf, nbytes_old);
    //         // super::mem::copy(dst, src);
    //     }
    // }

    fn grow(&mut self, nblocks: usize) {
        unsafe {
            let layout_old = self.layout();
            let nbytes_old = self.nbytes();

            self.len += nblocks;

            let layout_new = self.layout();
            let nbytes_new = self.nbytes();
            let dst = alloc_zeroed(layout_new);

            if dst.is_null() {
                panic!("Out of memeory!!");
            }

            let new_buf = copy_raw(dst, nbytes_new, self.buf, nbytes_old);

            dealloc(self.buf, layout_old);
            self.buf = new_buf;
            // let dst = slice::from_raw_parts_mut(dst, nbytes_new);
            // let src = slice::from_raw_parts(self.buf, nbytes_old);
            // super::mem::copy(dst, src);
        }
    }

    fn deref_handle<T>(&self, handle: &BlockHandle<T>) -> &T {
        unsafe {
            let ptr = self.buf.add(handle.id * self.block_size());
            let ptr = ptr.add(std::mem::size_of::<BlockMeta>());
            let offset = ptr.align_offset(std::mem::align_of::<T>());
            let ptr = ptr.add(offset).cast::<T>();
            ptr.as_ref().expect("Attempt to deref null pointer")
        }

        // let ptr = self.buf[handle.id * self.block_size()].as_ptr();
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BlockHandle<'alloc, T> {
    id: usize,
    parent: &'alloc BlockVec,
    _phantom: PhantomData<&'alloc T>,
}

impl<'alloc, T> BlockHandle<'alloc, T> {
    pub fn get(&self) -> &T {
        self.parent.deref_handle(self)
    }
}

impl Index<usize> for BlockVec {
    type Output = BlockView;

    fn index(&self, index: usize) -> &Self::Output {
        todo!()
    }
}

// pub struct BlockHandleMut<'alloc> {
//     id: usize,
//     parent: &'alloc mut BlockVec,
// }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BlockMeta {
    id: usize,
    next: usize,
    inner_size: usize,
}

impl BlockMeta {
    pub const fn alloc_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.inner_size
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BlockView {
    meta: BlockMeta,
    ptr: *mut u8,
}

impl BlockView {
    pub fn slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.ptr, self.meta.inner_size) }
    }

    pub const fn inner_size(&self) -> usize {
        self.meta.inner_size
    }
}

pub fn block_alloc_size(meta: &BlockMeta) -> usize {
    std::mem::size_of::<BlockMeta>() + meta.inner_size
}

pub fn copy_raw(dst_ptr: *mut u8, dst_len: usize, src_ptr: *const u8, src_len: usize) -> *mut u8 {
    let (dst, src) = unsafe {
        let dst = slice::from_raw_parts_mut(dst_ptr, dst_len);
        let src = slice::from_raw_parts(src_ptr, src_len);
        (dst, src)
    };

    super::mem::copy(dst, src);
    dst.as_mut_ptr()
}

pub fn copy(dst: &mut [u8], src: &[u8]) {
    let n = std::cmp::min(dst.len(), src.len());
    dst[..n].copy_from_slice(&src[..n])
}

impl Drop for BlockVec {
    fn drop(&mut self) {
        unsafe { dealloc(self.buf, self.layout()) }
    }
}
