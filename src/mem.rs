use core::slice;
use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    marker::PhantomData,
    ops::{Deref, DerefMut, Index, IndexMut},
    ptr::NonNull,
};

#[derive(Debug)]
pub struct BlockVec {
    first_avail: Option<usize>,
    block_size: usize,
    len: usize,
    isinit: bool,

    buf: NonNull<u8>,
    _phantom: PhantomData<[u8]>,
}

impl Clone for BlockVec {
    fn clone(&self) -> Self {
        todo!()
    }
}

impl BlockVec {
    pub fn new(block_size: usize) -> Self {
        Self {
            block_size,
            isinit: false,
            len: 0,
            first_avail: Some(0),
            buf: NonNull::dangling(),
            _phantom: PhantomData,
        }
    }

    pub fn get<T>(&self, index: usize) -> &T {
        self.view(index).to_ref()
    }

    pub fn alloc<T>(&mut self, v: T) -> BlockHandle<T> {
        let first_avail = if let Some(i) = self.first_avail {
            i
        } else {
            let next = self.len();
            self.resize(self.len() * 2);
            self.first_avail = Some(next);
            next
        };

        let header = {
            let view_mut = self.view_mut_header(first_avail);
            view_mut.header.alive = true;

            let view = view_mut.done();
            view.write(v);
            view.header
        };

        if header.next >= self.len() {
            // we will need to reallocate if Self::alloc is called again.
            self.first_avail = None;
        }

        BlockHandle {
            header,
            parent: self,
            _phantom: PhantomData,
        }
    }

    fn view(&self, index: usize) -> BlockView {
        let byte_offset = index * self.block_size_full();

        assert!(
            byte_offset < self.nbytes(),
            "Index out of range: BlockVec[{}]. len: {}",
            index,
            self.len
        );

        unsafe {
            let ptr = self.buf.add(byte_offset);

            let ptr = ptr.cast::<BlockHeader>();
            let header = NonNull::read(ptr);
            let ptr = ptr.add(1).cast::<u8>();

            let block_size = self.block_size;
            BlockView {
                ptr,
                header,
                block_size,
                _phantom: PhantomData,
            }
        }
    }

    fn view_mut_header(&self, index: usize) -> BlockViewMut {
        let byte_offset = index * self.block_size_full();

        assert!(
            byte_offset < self.nbytes(),
            "Index out of range: BlockVec[{}]. len: {}",
            index,
            self.len
        );

        unsafe {
            let ptr = self.buf.add(byte_offset);

            let ptr = ptr.cast::<BlockHeader>();
            let mut header = ptr;
            let ptr = ptr.add(1).cast::<u8>();

            let block_size = self.block_size;

            BlockViewMut {
                header: header.as_mut(),
                block_size,
                ptr,
                _phantom: PhantomData,
            }
        }
    }

    pub const fn bytes(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.buf.as_ptr(), self.nbytes()) }
    }

    /// Full size of block in bytes including header. block_size + size_of::<BlockHeader>()
    pub const fn block_size_full(&self) -> usize {
        self.block_size + std::mem::size_of::<BlockHeader>()
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn nbytes(&self) -> usize {
        self.len * self.block_size_full()
    }

    #[inline]
    fn layout(&self) -> Layout {
        Layout::array::<u8>(self.nbytes()).expect("Layout::array => nbytes too big")
    }

    pub fn realloc_inner(&mut self, new_size: usize) {
        self.len = new_size;
        unsafe {
            let ptr = std::alloc::realloc(self.buf.as_ptr(), self.layout(), self.len);
            self.buf = NonNull::new(ptr).expect("Out of Memory!!");
        }
    }

    /// Resizes inner buffer into a new allocated one with new size and
    /// memcpys the old vals into new allocated buffer, then frees old buffer.
    /// Allocates if new_size > self.len, otherwise self.len is decreased. (no need to
    /// allocated/deallocate if we dont need to on shrink )
    pub fn resize(&mut self, new_size: usize) {
        if new_size == 0 {
            self.len = 0;
        } else if new_size > self.len() {
            let old_len = self.len();
            let new_len = new_size + self.len();
            self.grow(new_len);

            for i in old_len..new_len {
                let v = self.view_mut_header(i);
                *v.header = BlockHeader {
                    id: i,
                    next: i + 1,
                    alive: false,
                };
            }
        } else if new_size < self.len() {
            self.shrink(self.len() - new_size)
        }
    }

    fn shrink(&mut self, nblocks: usize) {
        self.len -= nblocks;
    }

    pub const fn is_init(&self) -> bool {
        self.isinit && self.len > 0
    }

    pub const fn is_uninit(&self) -> bool {
        !self.is_init()
    }

    fn grow(&mut self, nblocks: usize) {
        if self.is_uninit() {
            self.len = nblocks;
            let ptr = unsafe { alloc_zeroed(self.layout()) };
            self.buf = NonNull::new(ptr).expect("Out of memory!!!");
        } else {
            let layout_old = self.layout();
            let nbytes_old = self.nbytes();

            self.len += nblocks;

            let layout_new = self.layout();
            let nbytes_new = self.nbytes();
            let dst = unsafe { alloc_zeroed(layout_new) };
            let dst = NonNull::new(dst).expect("Out of memory!!!");

            copy_raw(dst.as_ptr(), nbytes_new, self.buf.as_ptr(), nbytes_old);

            unsafe { dealloc(self.buf.as_ptr(), layout_old) };
            self.buf = dst;
        }
    }
}

impl Index<usize> for BlockVec {
    type Output = [u8];

    fn index(&self, index: usize) -> &Self::Output {
        self.view(index).slice()
    }
}

impl IndexMut<usize> for BlockVec {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.view(index).slice_mut()
    }
}

impl<'a, T> Index<BlockHandle<'a, T>> for BlockVec {
    type Output = T;

    fn index(&self, index: BlockHandle<'a, T>) -> &Self::Output {
        self.view(index.header.id).to_ref()
    }
}

impl<'a, T> IndexMut<BlockHandle<'a, T>> for BlockVec {
    fn index_mut(&mut self, index: BlockHandle<'a, T>) -> &mut Self::Output {
        self.view(index.header.id).to_mut()
    }
}

impl<'a, T> Index<&BlockHandle<'a, T>> for BlockVec {
    type Output = T;

    fn index(&self, index: &BlockHandle<'a, T>) -> &Self::Output {
        self.view(index.header.id).to_ref()
    }
}

impl<'a, T> IndexMut<&BlockHandle<'a, T>> for BlockVec {
    fn index_mut(&mut self, index: &BlockHandle<'a, T>) -> &mut Self::Output {
        self.view(index.header.id).to_mut()
    }
}

impl<'a, T> Index<&mut BlockHandle<'a, T>> for BlockVec {
    type Output = T;

    fn index(&self, index: &mut BlockHandle<'a, T>) -> &Self::Output {
        self.view(index.header.id).to_ref()
    }
}

impl<'a, T> IndexMut<&mut BlockHandle<'a, T>> for BlockVec {
    fn index_mut(&mut self, index: &mut BlockHandle<'a, T>) -> &mut Self::Output {
        self.view(index.header.id).to_mut()
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct BlockHeader {
    id: usize,
    next: usize,
    alive: bool,
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
        &self.parent[self]
    }
}

impl<'alloc, T> DerefMut for BlockHandle<'alloc, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.parent.view(self.header.id).to_mut()
    }
}

/// A view into an allocated block of memory.
#[derive(Debug, Clone, Copy)]
struct BlockView<'alloc> {
    header: BlockHeader,
    block_size: usize,
    ptr: NonNull<u8>,
    _phantom: PhantomData<&'alloc BlockVec>,
}

#[derive(Debug)]
struct BlockViewMut<'alloc> {
    header: &'alloc mut BlockHeader,
    block_size: usize,
    ptr: NonNull<u8>,
    _phantom: PhantomData<&'alloc BlockVec>,
}

impl<'a> BlockViewMut<'a> {
    pub fn done(self) -> BlockView<'a> {
        let Self {
            header,
            block_size,
            ptr,
            _phantom,
        } = self;
        let header = *header;
        BlockView {
            header,
            block_size,
            ptr,
            _phantom,
        }
    }
}

impl<'a> BlockView<'a> {
    pub const fn alloc_size(&self) -> usize {
        std::mem::size_of::<BlockHeader>() + self.block_size
    }
    pub const fn slice(&self) -> &'a [u8] {
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.block_size) }
    }

    pub fn slice_mut(&self) -> &'a mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.block_size) }
    }

    pub const fn inner_size(&self) -> usize {
        self.block_size
    }

    // NOTE :: Lifetimes make these below conversion functions 'safe', but
    // any allocations made on the owning BlockVec will make all these
    // references invalid.

    pub const fn to_ref<T>(self) -> &'a T {
        unsafe { self.ptr.cast::<T>().as_ref() }
    }

    pub fn to_mut<T>(self) -> &'a mut T {
        unsafe { self.ptr.cast::<T>().as_mut() }
    }

    pub const fn cast<T>(&self) -> &'a T {
        unsafe { self.ptr.cast::<T>().as_ref() }
    }

    pub fn cast_mut<T>(&self) -> &'a mut T {
        unsafe { self.ptr.cast::<T>().as_mut() }
    }

    pub fn write<T>(&self, v: T) {
        let inner = self.cast_mut::<T>();
        *inner = v;
    }
}

pub fn copy_raw(dst_ptr: *mut u8, dst_len: usize, src_ptr: *const u8, src_len: usize) {
    let (dst, src) = unsafe {
        let dst = slice::from_raw_parts_mut(dst_ptr, dst_len);
        let src = slice::from_raw_parts(src_ptr, src_len);
        (dst, src)
    };

    self::copy(dst, src);
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
