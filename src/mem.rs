use core::slice;
use std::{
    alloc::{alloc_zeroed, dealloc, realloc, Layout},
    cell::Cell,
    marker::PhantomData,
    mem::{align_of, size_of},
    ops::{Deref, DerefMut, Index, IndexMut},
    ptr::NonNull,
};

use anyhow::{bail, ensure};
use bytemuck::{Pod, Zeroable, ZeroableInOption};
use thiserror::Error;

use crate::{
    array::{self, Array, Slice, SlicePtr},
    ptr::ZenPtr,
};

pub struct BlockRef<'alloc, T>
where
    T: Byteable,
{
    id: usize,
    parent: &'alloc BlockVec,
    _phantom: PhantomData<&'alloc [T]>,
}

/// Structs implementing this type MUST BE `[repr(C)]`
pub unsafe trait Byteable {
    fn as_bytes(&self) -> &[u8]
    where
        Self: Sized,
    {
        let ptr = self as *const Self;
        let ptr = ptr as *const u8;
        let size = size_of::<Self>();
        unsafe { slice::from_raw_parts(ptr, size) }
    }

    // fn from_bytes()
}

pub unsafe fn from_bytes<T>(bytes: &[u8]) -> anyhow::Result<&T>
where
    T: Byteable,
{
    let size = size_of::<T>();
    if size > bytes.len() {
        bail!(
            "Type must be at least size of given byte slice. T: {size}, bytes: {}",
            bytes.len()
        )
    }
    let ptr = NonNull::new(bytes.as_ptr().cast_mut()).expect("Given byte slice pointer is null!!!");
    let t = unsafe { ptr.cast::<T>().as_ref() };
    Ok(t)
    // todo!();
}

pub unsafe fn from_bytes_mut<T>(bytes: &mut [u8]) -> anyhow::Result<&mut T>
where
    T: Byteable,
{
    let size = size_of::<T>();
    if size > bytes.len() {
        bail!(
            "Type must be at least size of given byte slice. T: {size}, bytes: {}",
            bytes.len()
        )
    }
    let ptr = NonNull::new(bytes.as_mut_ptr()).expect("Given byte slice pointer is null!!!");
    let t = unsafe { ptr.cast::<T>().as_mut() };
    Ok(t)
}

unsafe impl<T> Byteable for Cell<T> where T: Byteable {}

unsafe impl<T> Byteable for Option<T> where T: Byteable {}
unsafe impl<T> Byteable for [T] where T: Byteable {}
unsafe impl<T> Byteable for &[T] where T: Byteable {}
unsafe impl<T> Byteable for &T {}
unsafe impl<T> Byteable for &mut T {}
unsafe impl<T> Byteable for *mut T {}
unsafe impl<T> Byteable for *const T {}
unsafe impl<T> Byteable for NonNull<T> {}

unsafe impl Byteable for () {}
unsafe impl Byteable for u8 {}
unsafe impl Byteable for i8 {}
unsafe impl Byteable for u16 {}
unsafe impl Byteable for i16 {}
unsafe impl Byteable for u32 {}
unsafe impl Byteable for i32 {}
unsafe impl Byteable for u64 {}
unsafe impl Byteable for i64 {}
unsafe impl Byteable for usize {}
unsafe impl Byteable for isize {}
unsafe impl Byteable for u128 {}
unsafe impl Byteable for i128 {}
unsafe impl Byteable for f32 {}
unsafe impl Byteable for f64 {}

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
        self::write_bytes(new.bytes_mut(), self.bytes());
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
        T: Byteable,
    {
        self.view(id).cast_into()
    }

    pub fn free<'a, T, Ptr>(&'a self, bh: Ptr)
    where
        T: Byteable + 'a,
        Ptr: Into<Unbounded<'a, T>>,
    {
        let bh = bh.into();
        let first = self.first_avail.get();
        // bh.delete();
        self.nactive.set(self.nactive.get() - 1);

        let next = {
            let mut view = self.view(bh.id());

            let next = view.header.next;
            // view_mut.header.alive = false;
            view.header.state = BlockState::Dead;

            view.header.next = first;

            view.clear_zero();
            next
        };

        self.first_avail.set(next as usize);
    }

    fn alloc_bytes(&self, nbytes: usize) -> anyhow::Result<Bytes> {
        ensure!(
            nbytes <= self.block_size(),
            AllocError::TypeTooLarge {
                type_name: std::any::type_name::<&[u8]>().into(),
                type_size: nbytes,
                block_size: self.block_size()
            }
        );

        let next_id = if self.first_avail.get() > self.len() {
            bail!(AllocError::Full);
        } else {
            self.first_avail.get()
        };

        let header = {
            let mut view = self.view(next_id);
            view.header.state = BlockState::Bytes(nbytes);

            let h = *view.header;

            h
        };
        self.first_avail.set(header.next as usize);

        self.nactive.set(self.nactive.get() + 1);

        let parent = self as *const Self;
        let parent = NonNull::new(parent as _).expect("BlockVec => Self is null!!!");
        let bs = Bytes {
            id: next_id,
            parent,
            _phantom: PhantomData,
        };
        Ok(bs)
    }

    // TODO :: Refactor this to return an Option/Result<RawRef<T>>
    // ideally, the owner of this struct will allocate manually with self::resize,
    // and if for some reason we can't allocate because we are full, then return None/Err
    pub fn alloc<T: Sized>(&self, v: T) -> anyhow::Result<Unbounded<T>>
    where
        T: Byteable,
    {
        let bytes = self.alloc_bytes(size_of::<T>())?;
        let typed = bytes.to_ptr(v);
        Ok(typed)
        // Ok(bytes)
        // ensure!(
        //     size_of::<T>() <= self.block_size(),
        //     AllocError::TypeTooLarge {
        //         type_name: std::any::type_name::<T>().into(),
        //         type_size: size_of::<T>(),
        //         block_size: self.block_size()
        //     }
        // );
        //
        // let next_id = if self.first_avail.get() > self.len() {
        //     bail!(AllocError::Full);
        // } else {
        //     self.first_avail.get()
        // };
        //
        // let header = {
        //     let mut view = self.view(next_id);
        //     view.header.state = BlockState::Single;
        //
        //     let h = *view.header;
        //
        //     // let v = v.init();
        //     unsafe {
        //         view.write_raw(v);
        //     };
        //
        //     h
        // };
        // self.first_avail.set(header.next as usize);
        //
        // self.nactive.set(self.nactive.get() + 1);
        //
        // let rr = Unbounded {
        //     id: next_id,
        //     parent: self,
        //     _phantom: PhantomData,
        // };
        // Ok(rr)
    }

    fn emplace_alloc<T>(&self, vec: Vec<T>) -> anyhow::Result<Unbounded<Array<T>>>
    where
        T: Byteable,
    {
        let bytes = self.alloc_bytes(Array::<T>::size_of(vec.len()))?;
        let mut view = self.view(bytes.id());
        let len = if let BlockState::Bytes(n) = view.header.state {
            n / std::mem::size_of::<T>()
        } else {
            unreachable!()
        };
        view.header.state = BlockState::Array(len);
        let arr = unsafe { Array::from_alloced(bytes.mem_ptr(), vec.len()) };

        let arr = unsafe { arr.as_ref() };
        view.write_array(arr, vec.as_slice());
        let ptr = Unbounded {
            id: bytes.id(),
            parent: self,
            _phantom: PhantomData,
        };
        Ok(ptr)
    }

    pub fn array_reserve<T>(&self, n: usize) -> anyhow::Result<array::Slice<T>>
    where
        T: Byteable,
    {
        let size = size_of::<T>();
        let total_size = (size * n) + size_of::<array::Array<T>>();
        let arr = if total_size < self.block_size_full() {
            let v = Vec::<T>::with_capacity(n);
            let arr = unsafe { self.emplace_alloc(v)? };
            Slice::Block(SlicePtr::from(arr))
        } else {
            let layout = Layout::array::<u8>(total_size)?;
            let arr = unsafe {
                let p = unsafe { std::alloc::alloc_zeroed(layout) };
                let head = NonNull::new(p).expect("Out of Memory!!!");
                Array::<T>::from_alloced(head, n)
            };
            Slice::Fallback(arr)

            //     unsafe {
            //         let arr = head.add(1).cast::<u8>();
            //         let offset = arr.align_offset(align_of::<T>());
            //         let arr = arr.add(offset).cast::<T>();
            //         let arr = Array::from_raw_parts(arr, 1, n);
            //         NonNull::write(head, arr);
            //         arr
            //     }
        };
        Ok(arr)
    }

    pub fn array<T>(&self) -> anyhow::Result<array::Slice<T>>
    where
        T: Byteable,
    {
        self.array_reserve(0)
    }

    pub(crate) fn view(&self, index: usize) -> BlockView {
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
                    // id: i,
                    next: i + 1,
                    state: BlockState::Dead,
                    ..Default::default()
                };
            }
        } else if new_size < self.len() {
            self.shrink(self.len() - new_size)
        }
    }

    pub fn shrink(&self, nblocks: usize) {
        let nlen = self.len() - nblocks;
        self.len.set(nlen);
    }

    // pub fn shrink_free(&self, nblocks: usize) {}

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

            assert!(layout_old.size() == nbytes_old);
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

impl<'a, T> Index<Unbounded<'a, T>> for BlockVec
where
    T: Byteable,
{
    type Output = T;

    fn index(&self, index: Unbounded<'a, T>) -> &Self::Output {
        self.view(index.id()).cast_into()
    }
}

impl<'a, T> IndexMut<Unbounded<'a, T>> for BlockVec
where
    T: Byteable,
{
    fn index_mut(&mut self, index: Unbounded<'a, T>) -> &mut Self::Output {
        self.view(index.id()).cast_into_mut()
    }
}

impl<'a, T> Index<&Unbounded<'a, T>> for BlockVec
where
    T: Byteable,
{
    type Output = T;

    fn index(&self, index: &Unbounded<'a, T>) -> &Self::Output {
        self.view(index.id()).cast_into()
    }
}

impl<'a, T> IndexMut<&Unbounded<'a, T>> for BlockVec
where
    T: Byteable,
{
    fn index_mut(&mut self, index: &Unbounded<'a, T>) -> &mut Self::Output {
        self.view(index.id()).cast_into_mut()
    }
}

impl<'a, T> Index<&mut Unbounded<'a, T>> for BlockVec
where
    T: Byteable,
{
    type Output = T;

    fn index(&self, index: &mut Unbounded<'a, T>) -> &Self::Output {
        self.view(index.id()).cast_into()
    }
}

impl<'a, T> IndexMut<&mut Unbounded<'a, T>> for BlockVec
where
    T: Byteable,
{
    fn index_mut(&mut self, index: &mut Unbounded<'a, T>) -> &mut Self::Output {
        self.view(index.id()).cast_into_mut()
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Scoped<'alloc, T>(ZenPtr<'alloc, T>)
where
    T: Byteable;

impl<'a, T> Clone for Scoped<'a, T>
where
    T: Byteable + Clone,
{
    fn clone(&self) -> Self {
        let s = self.0.deref();

        match self.0 {
            ZenPtr::Unbounded(ref ptr) => {
                let other = ptr
                    .parent
                    .alloc(s.clone())
                    .expect("Unable to deep clone Scoped Ptr.");
                other.into_scoped()
            }
            ZenPtr::Raw(_) => todo!(),
        }
    }
}

impl<'a, T> Scoped<'a, T>
where
    T: Byteable,
{
    pub fn leak(&self) -> &ZenPtr<'a, T> {
        &self.0
    }
}

impl<'a, T> Deref for Scoped<'a, T>
where
    T: Byteable,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<'a, T> DerefMut for Scoped<'a, T>
where
    T: Byteable,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl<'a, T> Drop for Scoped<'a, T>
where
    T: Byteable,
{
    fn drop(&mut self) {
        match self.0 {
            ZenPtr::Unbounded(ref ptr) => {
                let inner = Unbounded::clone(&ptr);
                ptr.parent.free(inner);
            }
            ZenPtr::Raw(ptr) => unsafe { deallocate(ptr) },
        }
    }
}

impl<'a, T> From<Unbounded<'a, T>> for Scoped<'a, T>
where
    T: Byteable,
{
    fn from(value: Unbounded<'a, T>) -> Self {
        Self(ZenPtr::Unbounded(value))
    }
}

#[repr(usize)]
#[derive(Debug, Default, Clone, Copy)]
enum BlockState {
    #[default]
    Dead = 0,
    Single = 1,
    Bytes(usize),
    Array(usize),
}

impl BlockState {
    pub const fn try_to_array(&self, array_type_size: usize) -> Option<BlockState> {
        match *self {
            BlockState::Bytes(n) => Some(BlockState::Array(n / array_type_size)),
            BlockState::Array(_) => Some(*self),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct BlockHeader {
    next: usize,
    state: BlockState,
}

unsafe impl Byteable for BlockHeader {}

impl BlockHeader {
    pub const ALIVE: usize = 1;

    // This block can be used for a new allocation.
    pub const DEAD: usize = 0;

    /// True if self.alive is anything besides 0
    pub const fn alive(&self) -> bool {
        if let BlockState::Dead = &self.state {
            false
        } else {
            true
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy)]
pub struct Unbounded<'alloc, T: Byteable> {
    id: usize,
    // header: BlockHeader,
    parent: &'alloc BlockVec,
    _phantom: PhantomData<&'alloc T>,
}

impl<'alloc, T> Clone for Unbounded<'alloc, T>
where
    T: Byteable,
{
    fn clone(&self) -> Self {
        Self {
            id: self.id,

            parent: self.parent,
            _phantom: PhantomData,
        }
    }
}

impl<'alloc, T> Unbounded<'alloc, Array<'alloc, T>>
where
    T: Byteable,
{
    pub fn write_array(&mut self, v: &[T]) {
        let ptr = self.clone();
        let arr = ptr.deref();
        let mut view = self.parent.view(self.id());
        view.write_array(arr, v);
    }
}

impl<'alloc, T> Unbounded<'alloc, T>
where
    T: Byteable,
{
    pub fn write(&mut self, v: T) {
        let mut view = self.parent.view(self.id());
        view.write_bytes(v);
    }

    pub fn read(&self) -> T {
        let view = self.parent.view(self.id());
        let ptr = view.to_ptr::<T>();
        unsafe { NonNull::read(ptr) }
    }

    pub const fn id(&self) -> usize {
        self.id as usize
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

impl<'alloc, T> Deref for Unbounded<'alloc, T>
where
    T: Byteable,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.parent[self]
    }
}

impl<'alloc, T> DerefMut for Unbounded<'alloc, T>
where
    T: Byteable,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.parent.view(self.id()).cast_into_mut::<T>()
    }
}

/// A view into an allocated block of memory.
#[derive(Debug)]
struct BlockView<'alloc> {
    header: &'alloc mut BlockHeader,
    mem: &'alloc mut [u8],
    _phantom: PhantomData<&'alloc BlockVec>,
}

macro_rules! type_slice {
    ($self: ident, $size_t: ident) => {{
        let size = std::mem::size_of::<$size_t>();
        &$self.mem[..size]
    }};
}

macro_rules! type_slice_mut {
    ($self: ident, $size_t: ident) => {{
        let size = std::mem::size_of::<$size_t>();
        &mut $self.mem[..size]
    }};
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
        T: Byteable,
    {
        let x: &'a [u8] = type_slice!(self, T);

        unsafe { self::from_bytes(x).unwrap() }
        // bytemuck::from_bytes::<T>(x)
    }

    pub fn cast_into_mut<T>(self) -> &'a mut T
    where
        T: Byteable,
    {
        let x: &'a mut [u8] = type_slice_mut!(self, T);
        unsafe { self::from_bytes_mut(x).unwrap() }
        // bytemuck::from_bytes_mut::<T>(x)
    }

    // NOTE :: Lifetimes make these below conversion functions 'safe', but
    // any allocations made on the owning BlockVec will make all these
    // references invalid.

    pub fn cast<T>(&'a self) -> &'a T
    where
        T: Byteable,
    {
        let x: &'a [u8] = type_slice!(self, T);
        unsafe { self::from_bytes(x).unwrap() }
        // bytemuck::from_bytes(x)
    }

    pub fn cast_mut<T>(&'a mut self) -> &'a mut T
    where
        T: Byteable,
    {
        let x: &'a mut [u8] = type_slice_mut!(self, T);
        unsafe { self::from_bytes_mut(x).unwrap() }
        // bytemuck::from_bytes_mut(x)
    }

    pub fn write_array<T>(&'a mut self, arr: &Array<'a, T>, vec: &[T])
    where
        T: Byteable,
    {
        let mut head = arr.as_bytes();
        let bs = vec
            .iter()
            .flat_map(|x| x.as_bytes())
            .map(|x| *x)
            .collect::<Vec<u8>>();

        let bs = [head, bs.as_slice()].concat();
        self::write_bytes(self.mem, bs.as_slice());
    }

    pub fn write_vec<T>(&'a mut self, vec: Vec<T>)
    where
        T: Byteable,
    {
        let bs = vec
            .iter()
            .flat_map(|x| x.as_bytes())
            .map(|x| *x)
            .collect::<Vec<u8>>();
        self::write_bytes(self.mem, bs.as_slice());
    }

    pub fn write_bytes<T>(&'a mut self, v: T)
    where
        T: Byteable,
    {
        let val = v.as_bytes();
        // let val = bytemuck::bytes_of(&v);
        self::write_bytes(self.mem, val);
        // let inner = self.cast_mut::<T>();
        // *inner = v;
    }

    pub fn write_raw<T>(&'a mut self, v: T)
    where
        T: Byteable,
    {
        let ptr = self.mem.as_mut_ptr();
        unsafe { self::write_from(ptr, v) };
    }

    pub fn clear_zero(&'a mut self) {
        bytemuck::fill_zeroes(self.mem);
    }

    pub fn to_ptr<T>(self) -> NonNull<T>
    where
        T: Byteable,
    {
        let ptr = self.mem.as_ptr();
        NonNull::new(ptr as _).expect("Memory ref is invalid!!!")
    }
}

pub fn copy_raw(dst_ptr: *mut u8, dst_len: usize, src_ptr: *const u8, src_len: usize) {
    let (dst, src) = unsafe {
        let dst = slice::from_raw_parts_mut(dst_ptr, dst_len);
        let src = slice::from_raw_parts(src_ptr, src_len);
        (dst, src)
    };

    self::write_bytes(dst, src);
}

pub fn write_bytes(dst: &mut [u8], src: &[u8]) {
    let n = std::cmp::min(dst.len(), src.len());
    dst[..n].copy_from_slice(&src[..n])
}

pub unsafe fn write_from<T>(ptr: *mut u8, v: T)
where
    T: Byteable,
{
    let offset = ptr.align_offset(align_of::<T>());
    let ptr = ptr.add(offset).cast::<T>();
    std::ptr::write(ptr, v);
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
    ptr
}

/// Fallback allocator uses standard lib to allocate/free each object.
pub unsafe fn allocate<T>(val: T) -> NonNull<T> {
    let layout = Layout::new::<T>();
    let ptr = std::alloc::alloc_zeroed(layout) as *mut T;
    NonNull::new(ptr).expect("Out of Memory!!!")
}

pub unsafe fn deallocate<T>(ptr: NonNull<T>) {
    let layout = Layout::new::<T>();
    std::alloc::dealloc(ptr.as_ptr() as _, layout);
}
// pub fn realloc_(ptr: *mut u8, old_layout: Layout, new_layout: Layout) -> *mut u8 {
//     let dst = unsafe { alloc_zeroed(new_layout) };
//     let dst = NonNull::new(dst).expect("Out of memory!!!");
//
//     copy_raw(dst.as_ptr(), new_layout.size(), ptr, old_layout.size());
//
//     unsafe { dealloc(ptr, old_layout) };
//     dst.as_ptr()
//     // self.mem.set(dst.as_ptr());
// }

#[derive(Error, Debug)]
pub enum AllocError {
    #[error("Given Type Size is greater than BlockVec block_size. type: {type_name}, type_size: {type_size} block_size: {block_size}")]
    TypeTooLarge {
        type_name: String,
        type_size: usize,
        block_size: usize,
    },

    #[error("Allocated buffer is full. Resize (grow) to allocate more.")]
    Full,
    #[error("Attempted to lookup a block that is dead or exceeds the length of the vec. index(id): {id}, vec_len: {vec_len}")]
    OutOfScope { id: usize, vec_len: usize },
    #[error("Operating System Out of Memory!!!")]
    OutOfMemory,
    #[error("Allocator is not initialized")]
    Uninit,
}

#[repr(C)]
#[derive(Debug, Copy)]
pub struct Bytes {
    id: usize,
    parent: NonNull<BlockVec>,
    _phantom: PhantomData<[u8]>,
}
unsafe impl Byteable for Bytes {}

impl Bytes {
    const fn id(&self) -> usize {
        self.id as usize
    }

    pub fn to_array<'a, T>(self, v: &[T]) -> Unbounded<'a, T>
    where
        T: Byteable,
    {
        let id = self.id;
        let parent: &'a BlockVec = unsafe { self.parent.as_ref() };
        let mut view = parent.view(id);
        view.header.state = view
            .header
            .state
            .try_to_array(size_of::<T>())
            .expect("Self is not Bytes!!!");
        let mem = NonNull::new(view.mem.as_ptr() as _).expect("Null ptr deref!!!");
        let arr = unsafe { Array::from_alloced(mem, v.len()).as_ref() };
        view.write_array(arr, v);
        Unbounded {
            id,
            parent,
            _phantom: PhantomData,
        }
    }

    pub fn to_ptr<'a, T>(self, v: T) -> Unbounded<'a, T>
    where
        T: Byteable,
    {
        let id = self.id;
        let parent: &'a BlockVec = unsafe { self.parent.as_ref() };
        let mut view = parent.view(id);
        view.header.state = BlockState::Single;
        view.write_bytes(v);
        Unbounded {
            id,
            parent,
            _phantom: PhantomData,
        }
    }

    pub(crate) fn mem_ptr(&self) -> NonNull<u8> {
        let view = unsafe { self.parent.as_ref().view(self.id) };
        let mem = view.mem.as_ptr();
        NonNull::new(mem as _).expect("mem is null!!")
    }

    // const fn into_many(self) ->
}

impl Clone for Bytes {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            parent: self.parent,
            _phantom: PhantomData,
        }
    }
}

// #[repr(transparent)]
// pub struct ZenHandle<'alloc, T>()

// #[repr(transparent)]
// #[derive(Debug, Pod, Zeroable, Copy, Clone)]
// pub struct ZenHandle<'alloc, T>
// where
//     T: MemCell,
// {
//     ptr: Bytes,
//     _phantom: PhantomData<&'alloc T>,
// }

impl<'a, T> From<Unbounded<'a, T>> for Bytes
where
    T: Byteable,
{
    fn from(value: Unbounded<'a, T>) -> Self {
        let id = value.id;
        let parent = value.parent.as_ptr().cast::<BlockVec>();
        Self {
            id,
            parent,
            _phantom: PhantomData,
        }
    }
}
