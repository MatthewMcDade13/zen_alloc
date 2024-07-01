use core::slice;
use std::{
    alloc::{alloc_zeroed, dealloc, realloc, Layout},
    cell::Cell,
    marker::PhantomData,
    mem::align_of,
    ops::{Deref, DerefMut, Index, IndexMut},
    ptr::NonNull,
};

pub trait MemCell: bytemuck::Pod + bytemuck::Zeroable {
    fn init(self) -> Self {
        self
    }

    fn delete(self) {}
}

// impl MemCell for () {}
// impl MemCell for u8 {}
// impl MemCell for i8 {}
// impl MemCell for u16 {}
// impl MemCell for i16 {}
// impl MemCell for u32 {}
// impl MemCell for i32 {}
// impl MemCell for u64 {}
// impl MemCell for i64 {}
// impl MemCell for usize {}
// impl MemCell for isize {}
// impl MemCell for u128 {}
// impl MemCell for i128 {}
// impl MemCell for f32 {}
// impl MemCell for f64 {}
impl<T> MemCell for T where T: bytemuck::Pod + bytemuck::Zeroable {}

#[derive(Debug)]
pub struct BlockVec {
    first_avail: Cell<usize>,
    block_size: usize,
    /// Number of Blocks currently allocated.
    len: Cell<usize>,
    /// Number of blocks that have been 'allocated' for use.
    nactive: Cell<i32>,
    mem: Cell<*mut u8>,
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
    pub const fn new(block_size: usize) -> Self {
        Self {
            block_size,
            len: Cell::new(0),
            nactive: Cell::new(0),
            first_avail: Cell::new(0),
            mem: Cell::new(std::ptr::null_mut()),
            _phantom: PhantomData,
        }
    }

    pub fn with_capacity(block_size: usize, capacity: usize) -> Self {
        let s = Self::new(block_size);
        // s.isinit.set(true);
        s.resize(capacity);
        s
    }

    pub fn nactive(&self) -> i32 {
        self.nactive.get()
    }

    pub fn is_full(&self) -> bool {
        self.nactive() >= self.len() as i32
    }

    pub fn get<T>(&self, id: usize) -> &T
    where
        T: bytemuck::Pod + bytemuck::Zeroable,
    {
        self.view(id).cast_into()
    }

    pub fn free<T>(&self, bh: RawRef<T>)
    where
        T: MemCell,
    {
        let first = self.first_avail.get();
        bh.delete();
        self.nactive.set(self.nactive.get() - 1);

        let next = {
            let mut view = self.view(bh.header.id);

            let next = view.header.next;
            // view_mut.header.alive = false;
            view.header.alive = BlockHeader::DEAD;

            view.header.next = first;

            view.clear_zero();
            next
        };

        self.first_avail.set(next);
    }

    // TODO :: Refactor this to return an Option/Result<RawRef<T>>
    // ideally, the owner of this struct will allocate manually with self::resize,
    // and if for some reason we can't allocate because we are full, then return None/Err
    pub fn alloc<T>(&self, v: T) -> RawRef<T>
    where
        T: MemCell,
    {
        let first_avail = if self.first_avail.get() > self.len() {
            let next = self.len();
            self.resize(self.len() * 2);

            self.first_avail.set(next);
            next
        } else {
            self.first_avail.get()
        };

        let header = {
            let mut view = self.view(first_avail);
            // view_mut.header.alive = true;
            view.header.alive = BlockHeader::ALIVE;

            let h = *view.header;

            let v = v.init();
            // view.write_bytes(v);
            unsafe { view.write_raw(v) };

            h
        };

        self.nactive.set(self.nactive.get() + 1);

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
            self.len()
        );

        unsafe {
            let ptr = self.as_ptr().add(byte_offset);

            // let offset = ptr.align_offset(align_of::<BlockHeader>());
            let mut ptr = ptr.cast::<BlockHeader>();

            let header = ptr.as_mut();
            let ptr = ptr.add(1).cast::<u8>();
            let offset = ptr.align_offset(align_of::<u8>());
            let ptr = ptr.add(offset);

            let mem = slice::from_raw_parts_mut(ptr.as_ptr(), self.block_size);
            BlockView {
                mem,
                header,
                _phantom: PhantomData,
            }
        }
    }
    //
    // fn view_mut_header(&self, index: usize) -> BlockViewMut {
    //     let byte_offset = index * self.block_size_full();
    //
    //     assert!(
    //         byte_offset < self.nbytes(),
    //         "Index out of range: BlockVec[{}]. len: {}",
    //         index,
    //         self.nbytes()
    //     );
    //
    //     unsafe {
    //         let ptr = self.as_ptr().add(byte_offset);
    //
    //         let ptr = ptr.cast::<BlockHeader>();
    //         let header = ptr;
    //         let ptr = ptr.add(1).cast::<u8>();
    //         let offset = ptr.align_offset(align_of::<usize>());
    //         let ptr = ptr.add(offset);
    //
    //         let block_size = self.block_size;
    //         let mem = unsafe { slice::from_raw_parts_mut(ptr, block_size) };
    //
    //         BlockViewMut {
    //             header: header.as_mut().expect("Null ptr deref!!!"),
    //             block_size,
    //             mem,
    //             _phantom: PhantomData,
    //         }
    //     }
    // }

    fn as_ptr(&self) -> NonNull<u8> {
        let p = self.mem.get();
        NonNull::new(p).expect("Memory pointed to by BlockView is null!!!")
    }

    fn raw(&self) -> *mut u8 {
        self.mem.get()
    }

    pub fn bytes(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.raw(), self.nbytes()) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.raw(), self.nbytes()) }
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
        if new_size == self.len() {
            return;
        }
        if new_size == 0 {
            self.len.set(0);
        } else if new_size > self.len() {
            let old_len = self.len();
            let delta = new_size - old_len;
            // let new_len = new_size + self.len();
            self.grow(delta);

            for i in old_len..new_size {
                let v = self.view(i);
                *v.header = BlockHeader {
                    id: i,
                    next: i + 1,
                    alive: BlockHeader::DEAD,
                    ..Default::default()
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
        self.len() > 0
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
            self.mem.set(ptr);
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

            let bptr = self.raw();
            copy_raw(dst.as_ptr(), nbytes_new, bptr, nbytes_old);

            unsafe { dealloc(bptr, layout_old) };
            self.mem.set(dst.as_ptr());
        }
    }
}

impl<'a, T> Index<RawRef<'a, T>> for BlockVec
where
    T: bytemuck::Pod + bytemuck::Zeroable,
{
    type Output = T;

    fn index(&self, index: RawRef<'a, T>) -> &Self::Output {
        self.view(index.header.id).cast_into()
    }
}

impl<'a, T> IndexMut<RawRef<'a, T>> for BlockVec
where
    T: bytemuck::Pod + bytemuck::Zeroable,
{
    fn index_mut(&mut self, index: RawRef<'a, T>) -> &mut Self::Output {
        self.view(index.header.id).cast_into_mut()
    }
}

impl<'a, T> Index<&RawRef<'a, T>> for BlockVec
where
    T: bytemuck::Pod + bytemuck::Zeroable,
{
    type Output = T;

    fn index(&self, index: &RawRef<'a, T>) -> &Self::Output {
        self.view(index.header.id).cast_into()
    }
}

impl<'a, T> IndexMut<&RawRef<'a, T>> for BlockVec
where
    T: bytemuck::Pod + bytemuck::Zeroable,
{
    fn index_mut(&mut self, index: &RawRef<'a, T>) -> &mut Self::Output {
        self.view(index.header.id).cast_into_mut()
    }
}

impl<'a, T> Index<&mut RawRef<'a, T>> for BlockVec
where
    T: bytemuck::Pod + bytemuck::Zeroable,
{
    type Output = T;

    fn index(&self, index: &mut RawRef<'a, T>) -> &Self::Output {
        self.view(index.header.id).cast_into()
    }
}

impl<'a, T> IndexMut<&mut RawRef<'a, T>> for BlockVec
where
    T: bytemuck::Pod + bytemuck::Zeroable,
{
    fn index_mut(&mut self, index: &mut RawRef<'a, T>) -> &mut Self::Output {
        self.view(index.header.id).cast_into_mut()
    }
}

#[derive(Debug)]
pub struct Scoped<'alloc, T>(RawRef<'alloc, T>)
where
    T: MemCell;

impl<'a, T> Scoped<'a, T>
where
    T: MemCell,
{
    pub fn as_ref(&self) -> &RawRef<'a, T> {
        &self.0
    }

    pub fn leak(&self) -> RawRef<'a, T> {
        self.0
    }
}

impl<'a, T> Deref for Scoped<'a, T>
where
    T: MemCell,
{
    type Target = T; // RawRef<'a, T>;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<'a, T> DerefMut for Scoped<'a, T>
where
    T: MemCell,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl<'a, T> Drop for Scoped<'a, T>
where
    T: MemCell,
{
    fn drop(&mut self) {
        let inner = RawRef::clone(&self.0);
        self.0.parent.free(inner);
    }
}

impl<'a, T> From<RawRef<'a, T>> for Scoped<'a, T>
where
    T: MemCell,
{
    fn from(value: RawRef<'a, T>) -> Self {
        Self(value)
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlockHeader {
    id: usize,
    next: usize,
    // cant use bool with bytemuck, so we use u16 to also provide the padding we want.
    alive: u16,
    _padding: u16,
    _padding2: u32,
}

impl BlockHeader {
    pub const ALIVE: u16 = 1;

    // This blcok can be used for a new allocation.
    pub const DEAD: u16 = 0;

    /// True if self.alive is anything besides 0
    pub const fn alive(&self) -> bool {
        self.alive > 0
    }
}

#[derive(Debug, Copy)]
pub struct RawRef<'alloc, T: MemCell> {
    header: BlockHeader,
    parent: &'alloc BlockVec,
    _phantom: PhantomData<T>,
}

impl<'alloc, T> Clone for RawRef<'alloc, T>
where
    T: bytemuck::Pod + bytemuck::Zeroable,
{
    fn clone(&self) -> Self {
        Self {
            header: self.header,
            parent: self.parent,
            _phantom: PhantomData,
        }
    }
}

impl<'alloc, T> RawRef<'alloc, T>
where
    T: MemCell,
{
    pub const fn id(&self) -> usize {
        self.header.id
    }

    pub fn is_alive(&self) -> bool {
        let view = self.parent.view(self.id());
        view.header.alive()
    }

    pub fn free(self) {
        self.parent.free(self);
    }

    pub fn into_scoped(self) -> Scoped<'alloc, T> {
        Scoped::from(self)
    }
}

impl<'alloc, T> Deref for RawRef<'alloc, T>
where
    T: bytemuck::Pod + bytemuck::Zeroable,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.parent[self]
    }
}

impl<'alloc, T> DerefMut for RawRef<'alloc, T>
where
    T: bytemuck::Pod + bytemuck::Zeroable,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.parent.view(self.header.id).cast_into_mut::<T>()
    }
}

/// A view into an allocated block of memory.
#[derive(Debug)]
struct BlockView<'alloc> {
    header: &'alloc mut BlockHeader,
    mem: &'alloc mut [u8],
    _phantom: PhantomData<&'alloc BlockVec>,
}

impl<'a> BlockView<'a> {
    pub const fn alloc_size(&self) -> usize {
        std::mem::size_of::<BlockHeader>() + self.block_size()
    }

    pub const fn block_size(&self) -> usize {
        self.mem.len()
    }

    pub fn cast_into<T>(self) -> &'a T
    where
        T: bytemuck::Pod + bytemuck::Zeroable,
    {
        let size = std::mem::size_of::<T>();
        let x: &'a [u8] = &self.mem[..size];
        bytemuck::from_bytes::<T>(x)
    }

    pub fn cast_into_mut<T>(self) -> &'a mut T
    where
        T: bytemuck::Pod + bytemuck::Zeroable,
    {
        let size = std::mem::size_of::<T>();
        let x: &'a mut [u8] = &mut self.mem[..size];
        bytemuck::from_bytes_mut::<T>(x)
    }

    // pub fn cast_into<T>(self) -> BlockRef<'a, T>
    // where
    //     T: bytemuck::Pod + bytemuck::Zeroable,
    // {
    //     let size = std::mem::size_of::<T>();
    //     let x: &'a [u8] = &self.mem[..size];
    //     let val = bytemuck::from_bytes::<T>(x);
    //     BlockRef {
    //         header: *self.header,
    //         block_size: self.block_size,
    //         mem: val,
    //         _phantom: PhantomData,
    //     }
    // }

    // pub const fn slice(&'a self) -> &'a [u8] {
    // self.mem
    // unsafe { slice::from_raw_parts(self.mem.as_ptr(), self.block_size) }
    // }

    // pub fn slice_mut(&self) -> &'a mut [u8] {
    // self.mem
    // unsafe { slice::from_raw_parts_mut(self.mem.as_ptr(), self.block_size) }
    // }

    // NOTE :: Lifetimes make these below conversion functions 'safe', but
    // any allocations made on the owning BlockVec will make all these
    // references invalid.

    // pub const fn to_ref<T>(self) -> &'a T
    // where
    //     T: bytemuck::Pod + bytemuck::Zeroable,
    // {
    //     self.cast::<T>()
    //     // let size = std::mem::size_of::<T>();
    //     // bytemuck::from_bytes(&self.mem[..size])
    //     // unsafe { self.mem.cast::<T>().as_ref() }
    // }
    //
    // pub fn to_mut<T>(self) -> &'a mut T
    // where
    //     T: bytemuck::Pod + bytemuck::Zeroable,
    // {
    //     let s = { self.cast_mut::<T>() };
    //     s
    //     // let size = std::mem::size_of::<T>();
    //     // bytemuck::from_bytes_mut(&mut self.mem[..size])
    //     // unsafe { self.mem.cast::<T>().as_mut() }
    // }

    pub fn cast<T>(&'a self) -> &'a T
    where
        T: bytemuck::Pod + bytemuck::Zeroable,
    {
        let size = std::mem::size_of::<T>();
        let x: &'a [u8] = &self.mem[..size];
        bytemuck::from_bytes(x)
        // unsafe { self.mem.cast::<T>().as_ref() }
    }

    pub fn cast_mut<T>(&'a mut self) -> &'a mut T
    where
        T: bytemuck::Pod + bytemuck::Zeroable,
    {
        let size = std::mem::size_of::<T>();
        bytemuck::from_bytes_mut(&mut self.mem[..size])

        // unsafe { self.mem.cast::<T>().as_mut() }
    }

    pub fn write_bytes<T>(&'a mut self, v: T)
    where
        T: bytemuck::Pod,
    {
        let val = bytemuck::bytes_of(&v);
        self::copy(self.mem, val);
        // let inner = self.cast_mut::<T>();
        // *inner = v;
    }

    pub unsafe fn write_raw<T>(&'a mut self, v: T)
    where
        T: MemCell,
    {
        let ptr = self.mem.as_mut_ptr();
        let offset = ptr.align_offset(align_of::<T>());
        let ptr = ptr.add(offset).cast::<T>();
        std::ptr::write(ptr, v);

        // let inner = self.cast_mut::<T>();
        // *inner = v;
    }

    pub fn clear_zero(&'a mut self) {
        bytemuck::fill_zeroes(self.mem);
        // self.mem.fill(0);
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
        unsafe { dealloc(self.raw(), self.layout()) }
    }
}

#[inline]
pub fn index<T>(ptr: *const u8, mem_len: usize, index: usize, stride: usize) -> Option<*const T> {
    let byte_offset = index * stride;
    if byte_offset >= mem_len {
        None
    } else {
        let ptr = unsafe {
            let ptr = ptr.add(byte_offset);
            let offset = ptr.align_offset(align_of::<T>());
            let ptr = if offset < mem_len - 1 {
                ptr.add(offset).cast::<T>()
            } else {
                return None;
            };
            ptr
        };

        Some(ptr)
    }
}

#[inline]
pub fn index_read<T>(ptr: *const u8, mem_len: usize, index: usize, stride: usize) -> Option<T> {
    let ptr = self::index::<T>(ptr, mem_len, index, stride)?;
    unsafe { Some(std::ptr::read(ptr)) }
}

#[inline]
pub fn index_write<T>(ptr: *const u8, mem_len: usize, index: usize, stride: usize, val: T) {
    let ptr = self::index::<T>(ptr, mem_len, index, stride).unwrap() as *mut T;
    unsafe { std::ptr::write(ptr, val) }
}

pub const unsafe fn offset_by<T>(mem: *const u8) -> *const u8 {
    let byte_offset = std::mem::size_of::<T>();
    let ptr = mem.add(byte_offset);
    // let offset = mem.align_offset(align_of::<T>());
    // let ptr = mem.add(offset).cast::<T>();
    // let ptr = ptr.add(1);
    //
    // let offset = ptr.align_offset(align_of::<u8>());
    // let ptr = ptr.add(offset).cast::<u8>();
    ptr
}
