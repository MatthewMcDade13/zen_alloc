use core::slice;
use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    marker::PhantomData,
    ops::Deref,
    ptr::NonNull,
};

#[derive(Debug)]
pub struct BlockVec {
    block_size: usize,
    len: usize,
    buf: NonNull<u8>,

    _phantom: PhantomData<[u8]>,
}

impl BlockVec {
    pub fn new(block_size: usize) -> Self {
        Self {
            block_size,
            len: 0,
            buf: NonNull::dangling(),
            _phantom: PhantomData,
        }
    }

    pub fn index<T>(&self, index: usize) -> &T {
        self.view(index).to_ref()
    }

    fn view(&self, index: usize) -> BlockView {
        let byte_offset = index * self.block_size();

        assert!(
            byte_offset < self.nbytes(),
            "Index out of range: BlockVec[{}]. len: {}",
            index,
            self.len
        );

        unsafe {
            let ptr = self.buf.as_ptr().add(byte_offset);
            // let ptr = ptr.add(std::mem::size_of::<BlockHeader>());
            // let offset = ptr.align_offset(std::mem::align_of::<BlockView>());
            // let ptr = ptr.add(offset).cast::<BlockView>();

            let ptr = ptr.cast::<BlockHeader>();
            let header = std::ptr::read(ptr);
            let ptr = ptr.add(1).cast::<u8>();
            BlockView { ptr, header }
        }
    }

    pub const fn bytes(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.buf.as_ptr(), self.nbytes()) }
    }

    pub const fn block_size(&self) -> usize {
        self.block_size + std::mem::size_of::<BlockHeader>()
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn nbytes(&self) -> usize {
        self.len * self.block_size()
    }

    #[inline]
    fn layout(&self) -> Layout {
        Layout::array::<u8>(self.nbytes()).expect("Layout::array => nbytes too big")
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

            let new_buf = copy_raw(dst, nbytes_new, self.buf.as_ptr(), nbytes_old);

            dealloc(self.buf.as_ptr(), layout_old);
            self.buf = NonNull::new_unchecked(new_buf);
            // let dst = slice::from_raw_parts_mut(dst, nbytes_new);
            // let src = slice::from_raw_parts(self.buf, nbytes_old);
            // super::mem::copy(dst, src);
        }
    }

    fn deref_handle<T>(&self, handle: &BlockHandle<T>) -> &T {
        unsafe {
            let ptr = self.buf.as_ptr().add(handle.header.id * self.block_size());
            let ptr = ptr.add(std::mem::size_of::<BlockHeader>());
            let offset = ptr.align_offset(std::mem::align_of::<T>());
            let ptr = ptr.add(offset).cast::<T>();
            ptr.as_ref().expect("Attempt to deref null pointer")
        }

        // let ptr = self.buf[handle.id * self.block_size()].as_ptr();
    }
}

impl<'alloc, T> BlockHandle<'alloc, T> {
    pub fn get(&self) -> &T {
        self.parent.deref_handle(self)
    }
}

// pub struct BlockHandleMut<'alloc> {
//     id: usize,
//     parent: &'alloc mut BlockVec,
// }

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct BlockHeader {
    id: usize,
    next: usize,
    inner_size: usize,
}

impl BlockHeader {
    pub const fn alloc_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.inner_size
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BlockHandle<'alloc, T> {
    header: BlockHeader,
    parent: &'alloc BlockVec,
    _phantom: PhantomData<T>,
}

impl<'alloc, T> Deref for BlockHandle<'alloc, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.parent.deref_handle(self)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct BlockView {
    header: BlockHeader,
    ptr: *mut u8,
}

impl BlockView {
    pub const fn alloc_size(&self) -> usize {
        std::mem::size_of::<BlockHeader>() + self.header.inner_size
    }
    pub const fn slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.ptr, self.header.inner_size) }
    }

    pub const fn inner_size(&self) -> usize {
        self.header.inner_size
    }

    pub fn to_ref<'a, T>(self) -> &'a T {
        unsafe { self.ptr.cast::<T>().as_ref().expect("Null pointer deref.") }
    }

    pub fn to_mut<'a, T>(self) -> &'a mut T {
        unsafe { self.ptr.cast::<T>().as_mut().expect("Null pointer deref.") }
    }

    pub fn cast<T>(&self) -> &T {
        unsafe { self.ptr.cast::<T>().as_ref().expect("Null pointer deref.") }
    }

    pub fn cast_mut<T>(&self) -> &mut T {
        unsafe { self.ptr.cast::<T>().as_mut().expect("Null pointer deref.") }
    }
}

pub fn block_alloc_size(meta: &BlockHeader) -> usize {
    std::mem::size_of::<BlockHeader>() + meta.inner_size
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
        unsafe { dealloc(self.buf.as_ptr(), self.layout()) }
    }
}
