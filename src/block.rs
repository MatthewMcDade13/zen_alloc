use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    cell::{Cell, RefCell},
    marker::PhantomData,
    mem::size_of,
    ops::{Deref, DerefMut, Index, IndexMut},
    ptr::NonNull,
};

use anyhow::{bail, ensure};

use crate::mem::{allocate_array, from_bytes, from_bytes_mut, index_write, AllocError, Byteable};

#[repr(C)]
#[derive(Debug, Clone)]
pub struct MemBytes<const SIZE: usize>([u8; SIZE]);

impl<const SIZE: usize> Deref for MemBytes<SIZE> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.slice()
    }
}

impl<const SIZE: usize> DerefMut for MemBytes<SIZE> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl<const SIZE: usize> MemBytes<SIZE> {
    pub fn try_write<T>(&mut self, val: T) -> anyhow::Result<()>
    where
        T: Byteable,
    {
        ensure!(std::mem::size_of::<T>() <= SIZE);
        let other = val.as_bytes();
        self.write_bytes(other);
        Ok(())
    }

    pub fn try_write_bytes(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        ensure!(bytes.len() <= SIZE);
        crate::mem::write_bytes(&mut self.0, bytes);
        Ok(())
    }

    pub fn try_write_slice<T>(&mut self, slice: &[T]) -> anyhow::Result<()>
    where
        T: Byteable,
    {
        ensure!(std::mem::size_of::<T>() * slice.len() <= SIZE);
        let bytes: Vec<u8> = slice
            .iter()
            .flat_map(|x| x.as_bytes())
            .map(|x| *x)
            .collect();
        self.write_bytes(bytes.as_slice());
        Ok(())
    }
    pub fn write<T>(&mut self, val: T)
    where
        T: Byteable,
    {
        self.try_write(val).expect("Type too large!!!");
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.try_write_bytes(bytes).expect("Too many bytes!!!");
    }

    pub fn write_slice<T>(&mut self, slice: &[T])
    where
        T: Byteable,
    {
        self.try_write_slice(slice)
            .expect("Typed slice too large!!");
    }

    pub fn cast<T>(&self) -> anyhow::Result<&T>
    where
        T: Byteable,
    {
        let t = unsafe { from_bytes(self.slice())? };
        Ok(t)
    }

    pub fn cast_mut<T>(&mut self) -> anyhow::Result<&mut T>
    where
        T: Byteable,
    {
        let t = unsafe { from_bytes_mut(self.slice_mut())? };
        Ok(t)
    }

    pub fn cast_clone<T>(&self) -> anyhow::Result<T>
    where
        T: Byteable + Clone,
    {
        let t = unsafe { from_bytes::<T>(self.slice())? };
        Ok(t.clone())
    }

    pub fn cast_copy<T>(&self) -> anyhow::Result<T>
    where
        T: Byteable + Copy,
    {
        let t = unsafe { from_bytes(self.slice())? };
        Ok(*t)
    }

    pub const fn slice(&self) -> &[u8] {
        &self.0
    }
    pub fn slice_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }

    pub const fn zeroed() -> Self {
        Self([0u8; SIZE])
    }
}

#[repr(usize)]
#[derive(Debug, Default, Clone, Copy)]
enum MemState {
    #[default]
    Dead = 0,
    Single = 1,
    Bytes(usize),
    Array(usize),
}

impl MemState {
    pub const fn try_to_array(&self, array_type_size: usize) -> Option<MemState> {
        match *self {
            MemState::Bytes(n) => Some(MemState::Array(n / array_type_size)),
            MemState::Array(_) => Some(*self),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone)]
pub(crate) struct MemHeader {
    next: usize,
    state: MemState,
}

unsafe impl Byteable for MemHeader {}

#[derive(Debug)]
pub struct MemPool<const SIZE: usize> {
    first_avail: Cell<usize>,

    len: Cell<usize>,
    mem: Cell<NonNull<MemBlock<SIZE>>>,
}

impl<const S: usize> Index<usize> for MemPool<S> {
    type Output = MemBlock<S>;

    fn index(&self, index: usize) -> &Self::Output {
        let val = self.index_raw(index).unwrap();
        unsafe { val.as_ref() }
    }
}

// impl<const S: usize> IndexMut<usize> for MemPool<S> {
//     fn index_mut(&mut self, index: usize) -> &mut Self::Output {
//         let mut val = self.index_raw(index).unwrap();
//         unsafe { val.as_mut() }
//     }
// }

impl<const S: usize> MemPool<S> {
    pub(crate) fn mut_view<'a>(&'a self, index: usize) -> MemBlockMut<'a, S> {
        let block = self.index_raw(index).expect("index out of range!");
        MemBlockMut::new(block)
    }

    fn index_raw(&self, index: usize) -> anyhow::Result<NonNull<MemBlock<S>>> {
        let block_size = size_of::<MemBlock<S>>();
        let mem_len = self.len() * block_size;
        let val =
            crate::mem::index::<MemBlock<S>>(self.mem.as_ptr() as _, mem_len, index, block_size)
                .expect("Out of range!!!");
        if let Some(ptr) = NonNull::new(val as _) {
            Ok(ptr)
        } else {
            bail!("Null pointer deref!")
        }
    }
    pub fn len(&self) -> usize {
        self.len.get()
    }
    pub fn empty() -> Self {
        let first_avail = Cell::new(0);
        let mem = Cell::new(NonNull::dangling());
        Self {
            first_avail,
            mem,
            len: Cell::new(0),
        }
    }

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
                let v = self.mut_view(i);
                *v.head = MemHeader {
                    // id: i,
                    next: i + 1,
                    state: MemState::Dead,
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

    fn layout(&self) -> Layout {
        Layout::array::<MemBlock<S>>(self.len()).expect("Layout size error")
    }

    fn grow(&self, nblocks: usize) {
        if self.is_uninit() {
            self.len.set(nblocks);
            let ptr = unsafe { alloc_zeroed(self.layout()) };
            if ptr.is_null() {
                panic!("Out of memory!!!")
            }
            let ptr = NonNull::new(ptr).expect("Out of memory!!!");
            self.mem.set(ptr.cast::<MemBlock<S>>());
        } else {
            let layout_old = self.layout();
            let nbytes_old = self.size_bytes();

            assert!(layout_old.size() == nbytes_old);
            {
                let n = self.len() + nblocks;
                self.len.set(n);
            }

            let layout_new = self.layout();
            let nbytes_new = self.size_bytes();
            let dst = unsafe { alloc_zeroed(layout_new) };
            let dst = NonNull::new(dst).expect("Out of memory!!!");

            let bptr = self.mem.get().cast::<u8>();
            super::mem::copy_raw(dst.as_ptr(), nbytes_new, bptr.as_ptr(), nbytes_old);

            unsafe { dealloc(bptr.as_ptr(), layout_old) };
            self.mem.set(dst.cast::<MemBlock<S>>());
        }
    }

    pub fn new(len: usize) -> Self {
        let first_avail = Cell::new(0);
        // let mut mem = Vec::with_capacity(len);
        let (mem, nbytes) = unsafe { allocate_array::<MemBlock<S>>(len) };

        for i in 0..len {
            let h = MemHeader {
                next: i + 1,
                state: MemState::Dead,
            };
            let block = MemBlock::<S>::new(h);
            index_write(
                mem.as_ptr() as *mut u8,
                nbytes,
                i,
                size_of::<MemBlock<S>>(),
                block,
            )
            .expect("index write fail in MemPool::new");
        }
        let len = Cell::new(len);
        let mem = Cell::new(mem);
        Self {
            first_avail,
            mem,
            len,
        }
    }

    /// Size in bytes
    pub const fn block_size(&self) -> usize {
        size_of::<MemBlock<S>>()
    }

    pub fn size_bytes(&self) -> usize {
        self.len() * size_of::<MemBlock<S>>()
    }

    fn write_at<T>(&self, id: usize, v: T) -> anyhow::Result<()>
    where
        T: Byteable,
    {
        ensure!(id < self.len());

        let block = self.mut_view(id);
        block.bytes.try_write(v)?;
        Ok(())
    }

    pub fn alloc<T>(&self, v: T) -> anyhow::Result<Ptr<S, T>>
    where
        T: Byteable,
    {
        let bytes = self.alloc_bytes(std::mem::size_of::<T>())?;
        let mut typed = bytes.cast::<T>();

        let t = typed.inner_mut();

        Ok(typed)
    }

    pub fn alloc_bytes(&self, nbytes: usize) -> anyhow::Result<Ptr<S, [u8]>> {
        ensure!(
            nbytes <= S,
            AllocError::TypeTooLarge {
                type_name: std::any::type_name::<&[u8]>().into(),
                type_size: nbytes,
                block_size: S
            }
        );

        let next_id = if self.first_avail.get() > self.len() {
            bail!(AllocError::Full);
        } else {
            self.first_avail.get()
        };
        let v = self.mut_view(next_id);

        v.head.state = MemState::Bytes(nbytes);
        self.first_avail.set(v.head.next);

        let p: Ptr<S, [u8]> = Ptr {
            id: next_id,
            parent: self,
            _phantom: PhantomData,
        };
        Ok(p)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Ptr<'alloc, const SIZE: usize, T: Byteable + ?Sized> {
    id: usize,
    parent: &'alloc MemPool<SIZE>,
    _phantom: PhantomData<&'alloc T>,
}

impl<'a, const S: usize, T> Deref for Ptr<'a, S, T>
where
    T: Byteable,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner_ref()
    }
}

impl<'a, const S: usize, T> DerefMut for Ptr<'a, S, T>
where
    T: Byteable,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner_mut()
    }
}

impl<'a, const S: usize, T> Ptr<'a, S, T>
where
    T: Byteable,
{
    pub fn write(&self, v: T) {}
    pub fn inner_ref(&self) -> &T {
        self.parent[self.id]
            .mem
            .cast::<T>()
            .expect("Type too large!!!")
    }

    pub fn inner_mut(&mut self) -> &mut T {
        let v = self.parent.mut_view(self.id);
        v.cast_mut::<T>().expect("Type too large!!!")
    }
}

impl<'a, const S: usize> Ptr<'a, S, [u8]> {
    pub const fn cast<T>(&self) -> Ptr<'a, S, T>
    where
        T: Byteable + ?Sized,
    {
        Ptr {
            id: self.id,
            parent: self.parent,
            _phantom: PhantomData,
        }
    }
}

#[derive(Debug)]
pub(crate) struct MemBlockMut<'alloc, const SIZE: usize> {
    head: &'alloc mut MemHeader,
    bytes: &'alloc mut MemBytes<SIZE>,
}

impl<'a, const S: usize> MemBlockMut<'a, S> {
    pub fn new(mut ptr: NonNull<MemBlock<S>>) -> Self {
        let block = unsafe { ptr.as_mut() };
        let head = &mut block.head;
        let bytes = &mut block.mem;
        Self { head, bytes }
    }

    pub fn cast_mut<T>(mut self) -> anyhow::Result<&'a mut T>
    where
        T: Byteable,
    {
        // Hack/Workaround? We are erasing lifetime right here since we have it enforced already
        let t = unsafe {
            let tref = from_bytes_mut(&mut self.bytes)?;
            let mut ptr = NonNull::new(tref).expect("Reference is null!");
            ptr.as_mut()
        };
        Ok(t)
    }
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct MemBlock<const SIZE: usize> {
    head: MemHeader,
    mem: MemBytes<SIZE>,
}

impl<const S: usize> MemBlock<S> {
    const fn new(head: MemHeader) -> Self {
        let mem = MemBytes::zeroed();
        Self { head, mem }
    }
}

unsafe impl<const S: usize> Byteable for MemBlock<S> {}

impl<const SIZE: usize> MemBlock<SIZE> {}

// #[repr(C)]
// #[derive(Debug)]
// pub struct RcMem<const SIZE: usize, T: Byteable + ?Sized> {
//     strong: Cell<usize>,
//     weak: Cell<usize>,
// }
//
// impl<T> RcMem<T> where T: Byteable + ?Sized {}
