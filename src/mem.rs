use core::slice;
use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    cell::Cell,
    marker::PhantomData,
    ops::{Deref, DerefMut, Index, IndexMut},
    ptr::NonNull,
};

#[derive(Debug)]
pub struct BlockVec {
    first_avail: Cell<usize>,
    block_size: usize,
    len: Cell<usize>,
    isinit: Cell<bool>,

    buf: Cell<*mut u8>,
    _phantom: PhantomData<[u8]>,
}

impl Clone for BlockVec {
    fn clone(&self) -> Self {
        let mut new = Self::with_capacity(self.block_size, self.len());
        self::copy(new.bytes_mut(), self.bytes());
        new
    }
}

impl BlockVec {
    pub fn new(block_size: usize) -> Self {
        Self {
            block_size,
            isinit: Cell::new(false),
            len: Cell::new(0),
            first_avail: Cell::new(0),
            buf: Cell::new(std::ptr::null_mut()),
            _phantom: PhantomData,
        }
    }

    pub fn with_capacity(block_size: usize, capacity: usize) -> Self {
        let mut s = Self::new(block_size);
        *s.isinit.get_mut() = true;
        s.resize(capacity);
        s
    }

    pub fn get<T>(&self, id: usize) -> &T {
        self.view(id).to_ref()
    }

    pub fn free<T>(&self, bh: RawRef<T>) {
        let first = self.first_avail.get();

        let next = {
            let view_mut = self.view_mut_header(bh.header.id);

            let next = view_mut.header.next;
            view_mut.header.alive = false;

            view_mut.header.next = first;

            let view = view_mut.done();
            view.drop_inner::<T>();
            view.clear_zero();
            next
        };

        self.first_avail.set(next);
    }

    // TODO :: Refactor this to return an Option/Result<RawRef<T>>
    // ideally, the owner of this struct will allocate manually with self::resize,
    // and if for some reason we can't allocate because we are full, then return None/Err
    pub fn alloc<T>(&self, v: T) -> RawRef<T> {
        let first_avail = if self.first_avail.get() > self.len() {
            let next = self.len();
            self.resize(self.len() * 2);

            self.first_avail.set(next);
            next
        } else {
            self.first_avail.get()
        };

        let header = {
            let view_mut = self.view_mut_header(first_avail);
            view_mut.header.alive = true;

            let view = view_mut.done();
            view.write(v);
            view.header
        };

        RawRef {
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
            self.len.get()
        );

        unsafe {
            let ptr = self.as_ptr().add(byte_offset);

            let ptr = ptr.cast::<BlockHeader>();
            let header = std::ptr::read(ptr);
            let ptr = ptr.add(1).cast::<u8>();

            let block_size = self.block_size;
            BlockView {
                ptr: NonNull::new(ptr).unwrap(),
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
            self.nbytes()
        );

        unsafe {
            let ptr = self.as_ptr().add(byte_offset);

            let ptr = ptr.cast::<BlockHeader>();
            let mut header = ptr;
            let ptr = ptr.add(1).cast::<u8>();

            let block_size = self.block_size;

            BlockViewMut {
                header: header.as_mut().expect("Null ptr deref!!!"),
                block_size,
                ptr: NonNull::new(ptr).unwrap(),
                _phantom: PhantomData,
            }
        }
    }

    fn as_ptr(&self) -> *mut u8 {
        self.buf.get()
    }

    pub fn bytes(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.as_ptr(), self.nbytes()) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.as_ptr(), self.nbytes()) }
    }

    pub const fn block_size(&self) -> usize {
        self.block_size
    }

    /// Full size of block in bytes including header. block_size + size_of::<BlockHeader>()
    pub const fn block_size_full(&self) -> usize {
        self.block_size + std::mem::size_of::<BlockHeader>()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len.get()
    }

    #[inline]
    pub fn nbytes(&self) -> usize {
        self.len() * self.block_size_full()
    }

    #[inline]
    fn layout(&self) -> Layout {
        Layout::array::<u8>(self.nbytes()).expect("Layout::array => nbytes too big")
    }

    // fn realloc_inner(&mut self, new_size: usize) {
    //     self.len = new_size;
    //     unsafe {
    //         let ptr = std::alloc::realloc(self.buf.as_ptr(), self.layout(), self.len);
    //         self.buf = NonNull::new(ptr).expect("Out of Memory!!");
    //     }
    // }

    /// Resizes inner buffer into a new allocated one with new size and
    /// memcpys the old vals into new allocated buffer, then frees old buffer.
    /// Allocates if new_size > self.len, otherwise self.len is decreased. (no need to
    /// allocated/deallocate if we dont need to on shrink )
    pub fn resize(&self, new_size: usize) {
        if new_size == 0 {
            self.len.set(0);
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

    fn shrink(&self, nblocks: usize) {
        let nlen = self.len() - nblocks;
        self.len.set(nlen);
    }

    pub fn is_init(&self) -> bool {
        self.isinit.get() && self.len() > 0
    }

    pub fn is_uninit(&self) -> bool {
        !self.is_init()
    }

    fn grow(&self, nblocks: usize) {
        if self.is_uninit() {
            self.len.set(nblocks);
            let ptr = unsafe { alloc_zeroed(self.layout()) };
            if ptr.is_null() {
                panic!("Out of memory!!!")
            }
            // let ptr = NonNull::new(ptr).expect("Out of memory!!!");
            self.buf.set(ptr);
            self.isinit.set(true);
        } else {
            let layout_old = self.layout();
            let nbytes_old = self.nbytes();

            {
                let n = self.len() + nblocks;
                self.len.set(n);
            }

            let layout_new = self.layout();
            let nbytes_new = self.nbytes();
            let dst = unsafe { alloc_zeroed(layout_new) };
            let dst = NonNull::new(dst).expect("Out of memory!!!");

            let bptr = self.as_ptr();
            copy_raw(dst.as_ptr(), nbytes_new, bptr, nbytes_old);

            unsafe { dealloc(bptr, layout_old) };
            self.buf.set(dst.as_ptr());
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

impl<'a, T> Index<RawRef<'a, T>> for BlockVec {
    type Output = T;

    fn index(&self, index: RawRef<'a, T>) -> &Self::Output {
        self.view(index.header.id).to_ref()
    }
}

impl<'a, T> IndexMut<RawRef<'a, T>> for BlockVec {
    fn index_mut(&mut self, index: RawRef<'a, T>) -> &mut Self::Output {
        self.view(index.header.id).to_mut()
    }
}

impl<'a, T> Index<&RawRef<'a, T>> for BlockVec {
    type Output = T;

    fn index(&self, index: &RawRef<'a, T>) -> &Self::Output {
        self.view(index.header.id).to_ref()
    }
}

impl<'a, T> IndexMut<&RawRef<'a, T>> for BlockVec {
    fn index_mut(&mut self, index: &RawRef<'a, T>) -> &mut Self::Output {
        self.view(index.header.id).to_mut()
    }
}

impl<'a, T> Index<&mut RawRef<'a, T>> for BlockVec {
    type Output = T;

    fn index(&self, index: &mut RawRef<'a, T>) -> &Self::Output {
        self.view(index.header.id).to_ref()
    }
}

impl<'a, T> IndexMut<&mut RawRef<'a, T>> for BlockVec {
    fn index_mut(&mut self, index: &mut RawRef<'a, T>) -> &mut Self::Output {
        self.view(index.header.id).to_mut()
    }
}

#[derive(Debug)]
pub struct Scoped<'alloc, T>(pub RawRef<'alloc, T>);

impl<'a, T> Deref for Scoped<'a, T> {
    type Target = RawRef<'a, T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, T> DerefMut for Scoped<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a, T> Drop for Scoped<'a, T> {
    fn drop(&mut self) {
        let inner = RawRef::clone(&self.0);
        self.0.parent.free(inner);
    }
}

impl<'a, T> From<RawRef<'a, T>> for Scoped<'a, T> {
    fn from(value: RawRef<'a, T>) -> Self {
        Self(value)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct BlockHeader {
    id: usize,
    next: usize,
    alive: bool,
}

#[derive(Debug, Copy)]
pub struct RawRef<'alloc, T> {
    header: BlockHeader,
    parent: &'alloc BlockVec,
    _phantom: PhantomData<T>,
}

impl<'alloc, T> Clone for RawRef<'alloc, T> {
    fn clone(&self) -> Self {
        Self {
            header: self.header,
            parent: self.parent,
            _phantom: PhantomData,
        }
    }
}

impl<'alloc, T> RawRef<'alloc, T> {
    pub const fn id(&self) -> usize {
        self.header.id
    }

    pub fn is_alive(&self) -> bool {
        let view = self.parent.view(self.id());
        view.header.alive
    }

    pub fn free(self) {
        self.parent.free(self);
    }

    pub fn into_scoped(self) -> Scoped<'alloc, T> {
        Scoped::from(self)
    }
}

impl<'alloc, T> Deref for RawRef<'alloc, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.parent[self]
    }
}

impl<'alloc, T> DerefMut for RawRef<'alloc, T> {
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

    pub fn clear_zero(&self) {
        self.slice_mut().fill(0);
    }

    pub fn drop_inner<T>(&self) {
        let inner = self.to_ref::<T>();
        let _ = *inner;
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
        unsafe { dealloc(self.as_ptr(), self.layout()) }
    }
}
