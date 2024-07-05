use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

use anyhow::bail;

pub trait ZenAlloc: Sized {
    fn alloc<T>(&self, data: T) -> Handle<T, Self>;
    fn dealloc<T>(&self, handle: Handle<T, Self>);

    fn get_ptr<T>(&self, handle: Handle<T, Self>) -> *mut T;
}

#[derive(Debug, Clone)]
pub struct SizedBlock<const SIZE: usize>([u8; SIZE]);

impl<const SIZE: usize> SizedBlock<SIZE> {
    pub const fn zeroed() -> Self {
        Self([0; SIZE])
    }

    pub fn write<T>(&mut self, data: T) -> anyhow::Result<()>
    where
        T: Sized,
    {
        let type_size = std::mem::size_of::<T>();
        if type_size > SIZE {
            bail!(
                "Type of size: {} is greater than size of Block: {}",
                type_size,
                SIZE
            );
        }
        let ptr = self.0.as_mut_ptr();
        let offset = ptr.align_offset(std::mem::align_of::<T>());
        let ptr = unsafe { ptr.add(offset) };
        let ptr = ptr.cast::<T>();
        unsafe { std::ptr::write(ptr, data) };
        Ok(())

        // let bytes = bytemuck::bytes_of(&data);
        // if bytes.len() > SIZE {
        //     panic!("Given value must be smaller than or equal to {}", SIZE);
        // }
        // self.0.clone_from_slice(bytes);
    }

    pub fn cast_mut<T>(&mut self) -> &mut T {
        let ptr = self.0.as_mut_ptr();
        let ptr = unsafe { NonNull::new_unchecked(ptr) };
        unsafe { ptr.cast::<T>().as_mut() }
    }

    pub fn cast<T>(&self) -> &T {
        let ptr = self.0.as_ptr();
        let ptr = unsafe { NonNull::new_unchecked(ptr as *mut u8) };
        unsafe { ptr.cast::<T>().as_ref() }
    }
}

impl<const SIZE: usize> Default for SizedBlock<SIZE> {
    fn default() -> Self {
        Self([0; SIZE])
    }
}

#[derive(Debug, Default, Clone)]
pub struct PoolBlock<const SIZE: usize> {
    id: usize,
    next: usize,
    alive: bool,
    block: SizedBlock<SIZE>,
}

impl<const SIZE: usize> PoolBlock<SIZE> {
    pub const fn new(index: usize, next: usize) -> Self {
        Self {
            id: index,
            next,
            alive: false,
            block: SizedBlock::zeroed(),
        }
    }
}

#[derive(Debug, Copy)]
pub struct Handle<'alloc, T, A>
where
    A: ZenAlloc,
{
    id: usize,
    alloc: *mut A,
    _phantom: PhantomData<&'alloc T>,
}

impl<'alloc, T, A> Clone for Handle<'alloc, T, A>
where
    A: ZenAlloc,
{
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            alloc: self.alloc,
            _phantom: self._phantom,
        }
    }
}

impl<'alloc, T, A> Deref for Handle<'alloc, T, A>
where
    A: ZenAlloc,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let alloc = unsafe {
            self.alloc
                .as_ref()
                .expect("Handle pointer to Allocator is null and was attempted to be dereferenced.")
        };

        unsafe {
            let ptr = alloc.get_ptr::<T>(self.clone());
            ptr.as_ref()
                .expect("Pointer to allocation id: {} is null and was attempted to be derefed.")
        }
    }
}

// pub struct PoolHandle<'pool, T, const SIZE: usize> {
//     index: usize,
//     pool: NonNull<FixedBlockPool<SIZE>>,
//     _phantom: PhantomData<&'pool FixedBlockPool<SIZE>>,
// }

#[derive(Debug)]
pub struct FixedBlockPool<const SIZE: usize> {
    first_avail: usize,
    pool: Vec<PoolBlock<SIZE>>,
}

impl<const SIZE: usize> FixedBlockPool<SIZE> {
    pub fn new(size: usize) -> Self {
        let mut pool = vec![PoolBlock::default(); size];
        for i in 0..pool.len() {
            let b = &mut pool[i];
            b.next = i + 1;
            b.id = i;
            b.alive = false;
        }

        Self {
            first_avail: 0,
            pool,
        }
    }
}

pub struct BlockPool {
    first_avail: usize,
    block_size: usize,
    blocks: Vec<u8>,
}

impl BlockPool {
    pub fn new(block_size: usize) -> Self {
        // let blocks = vec![0; block_size * 2];
        let blocks = Vec::new();
        Self {
            first_avail: 0,
            block_size,
            blocks,
        }
    }

    fn t<T: Sized>(&mut self, mut data: T) {
        let s = self.index_slice_mut(0);
        let ptr = s.as_mut_ptr();
        let ptr = ptr.cast::<T>();
        unsafe { std::ptr::write(ptr, data) }
    }

    // pub fn alloc(&)

    pub fn index_slice(&self, index: usize) -> &[u8] {
        let stride = index * self.block_size;
        let end = stride + self.block_size;
        &self.blocks[stride..end]
    }

    pub fn index_slice_mut(&mut self, index: usize) -> &mut [u8] {
        let stride = index * self.block_size;
        let end = stride + self.block_size;
        &mut self.blocks[stride..end]
    }

    // pub fn alloc<T>(&mut self, data: T) {}
}

#[derive(Debug, Clone, Copy)]
pub struct BlockDesc {
    size_bytes: usize,
    id: usize,
}
