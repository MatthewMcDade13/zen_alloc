
mod test;

use std::{
    alloc::{alloc, dealloc, Layout},
    mem::align_of,
    ops::{Deref, DerefMut},
};

use anyhow::bail;

#[derive(Debug)]
pub struct RadPtr<T>
where
    T: Sized,
{
    ptr: *mut T,
}

impl<T> Clone for RadPtr<T> {
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr.clone(),
        }
    }
}

impl<T> DerefMut for RadPtr<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            self.ptr
                .as_mut()
                .expect("Attempted to dereference null RadPtr")
        }
    }
}

impl<T> Deref for RadPtr<T> {
    fn deref(&self) -> &Self::Target {
        unsafe {
            self.ptr
                .as_ref()
                .expect("Attempted to dereference null RadPtr")
        }
    }

    type Target = T;
}

pub struct StackAllocator<const S: usize> {
    stack: [u8; S],
    top: usize,
}

impl<const S: usize> StackAllocator<S> {
    pub const fn new() -> Self {
        Self {
            stack: [0; S],
            top: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn alloc<T>(&mut self, data: T) -> anyhow::Result<RadPtr<T>>
    where
        T: Sized,
    {
        let data_size = std::mem::size_of::<T>();
        if self.top + data_size > self.len() {
            bail!("Stack allocator out of memory");
        }
        unsafe {
            // let offset = self.stack.as_mut_ptr().align_offset(align_of::<u8>());
            let ptr = self.stack.as_mut_ptr().add(self.top);
            let offset = ptr.align_offset(align_of::<T>());
            let ptr = ptr.add(offset).cast::<T>();
            std::ptr::write(ptr, data);
            self.top += data_size + offset;

            let sp = RadPtr { ptr };
            Ok(sp)
        }
    }

    pub fn clear(&mut self) {
        self.top = 0;
    }

    pub fn popn(&mut self, n: usize) {
        self.shrink(self.top - n);
    }

    pub fn shrink(&mut self, to: usize) {
        self.top = to;
    }
}

#[derive(Debug)]
struct PoolCell<T> {
    cell: T,
    slot: isize,
    next: isize,
    valid: bool,
}

#[derive(Debug)]
pub struct PoolPtr<T>(RadPtr<PoolCell<T>>);

impl<T> Clone for PoolPtr<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct PoolAllocator<T> {
    buf: *mut PoolCell<T>,
    layout: Layout,
    size: isize,
    next_available: isize,
}

impl<T> PoolAllocator<T> {
    pub fn new(size: isize) -> Self {
        unsafe {
            let layout = Layout::array::<T>(size as usize).expect("Error with memory layout size");
            let ptr = alloc(layout);
            let ptr = ptr as *mut PoolCell<T>;

            for i in 0..size {
                let cell = &mut *ptr.offset(i);
                cell.next = i + 1;
                cell.slot = i;
                cell.valid = false;
            }

            let back = &mut *ptr.offset(size - 1);
            back.next = -1;

            Self {
                buf: ptr,
                layout,
                size,
                next_available: 0,
            }
        }
    }

    pub fn alloc(&mut self, data: T) -> PoolPtr<T> {
        let next_avail = self.next_available;
        let c = self.at_mut(next_avail);
        c.cell = data;
        self.next_available = c.next;
        self.at_ptr(next_avail)
    }

    pub fn dealloc(&mut self, ptr: PoolPtr<T>) {
        let mut ptr = ptr.clone();
        let cell = ptr.pcell_mut();
        cell.next = self.next_available;
        self.next_available = cell.slot;
    }

    fn at(&self, slot: isize) -> &PoolCell<T> {
        unsafe {
            let ptr = self.buf.offset(slot);
            &*ptr
        }
    }

    fn at_mut(&mut self, slot: isize) -> &mut PoolCell<T> {
        unsafe {
            let ptr = self.buf.offset(slot);
            let offset = ptr.align_offset(align_of::<T>());
            let ptr = ptr.add(offset);
            &mut *ptr
        }
    }

    fn at_ptr(&self, slot: isize) -> PoolPtr<T> {
        unsafe {
            let ptr = self.buf.offset(slot);
            PoolPtr(RadPtr { ptr })
        }
    }
}

impl<T> PoolPtr<T> {
    fn pcell(&self) -> &PoolCell<T> {
        &self.0
    }

    fn pcell_mut(&mut self) -> &mut PoolCell<T> {
        &mut self.0
    }
}

impl<T> Deref for PoolPtr<T> {
    fn deref(&self) -> &Self::Target {
        &self.0.cell
    }

    type Target = T;
}

impl<T> DerefMut for PoolPtr<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.cell
    }
}

impl<T> Drop for PoolAllocator<T> {
    fn drop(&mut self) {
        unsafe { dealloc(self.buf as *mut u8, self.layout) }
    }
}

pub type BumpPtr<T> = RadPtr<T>;

pub struct BumpAllocator {
    buf: *mut u8,

    layout: Layout,
    capacity: usize,
    size: usize,
}

impl BumpAllocator {
    pub const DEFAULT_ALIGNMENT: usize = std::mem::align_of::<u8>();

    pub fn new(size_bytes: usize) -> anyhow::Result<Self> {
        Self::with_align(size_bytes, Self::DEFAULT_ALIGNMENT)
    }

    pub fn with_align(size_bytes: usize, align: usize) -> anyhow::Result<Self> {
        unsafe {
            let layout = Layout::from_size_align(size_bytes, align)?;
            let buf = alloc(layout);
            if buf.is_null() {
                bail!("BumpAllocator::with_align => Unable to allocate more memory from Global Allocator");
            }
            let top = buf;
            let capacity = size_bytes;

            let s = Self {
                buf,
                layout,
                capacity,

                size: 0,
            };
            Ok(s)
        }
    }

    pub fn alloc<T>(&mut self, data: T) -> anyhow::Result<BumpPtr<T>> {
        unsafe {
            let data_size = std::mem::size_of::<T>();
            if self.size + data_size > self.capacity {
                bail!(
                    "BumpAllocator::alloc => Cannot performa allocation: Allocator out of memory"
                );
            }

            let ptr = self.buf.add(self.size);
            let offset = ptr.align_offset(align_of::<T>());
            let ptr = ptr.add(offset).cast::<T>();
            std::ptr::write(ptr, data);
            self.size += data_size + offset;

            let sp = RadPtr { ptr };
            Ok(sp)
        }
    }

    pub fn clear(&mut self) {
        self.size = 0;
    }

    pub fn release(self) {
        drop(self)
    }
}

impl Drop for BumpAllocator {
    fn drop(&mut self) {
        unsafe { dealloc(self.buf as *mut u8, self.layout) }
    }
}

pub struct DoubleBumpAllocator {
    bufs: [BumpAllocator; 2],
    current: usize,
}

impl DoubleBumpAllocator {
    pub fn new(size_bytes: usize) -> anyhow::Result<Self> {
        Self::with_align(size_bytes, BumpAllocator::DEFAULT_ALIGNMENT)
    }

    pub fn with_align(size_bytes: usize, align: usize) -> anyhow::Result<Self> {
        let a = BumpAllocator::with_align(size_bytes, align)?;
        let b = BumpAllocator::with_align(size_bytes, align)?;

        let s = Self {
            bufs: [a, b],
            current: 0,
        };
        Ok(s)
    }

    pub fn swap(&mut self) {
        self.current = !self.current;
    }

    pub fn current(&self) -> &BumpAllocator {
        &self.bufs[self.current]
    }

    pub fn current_mut(&mut self) -> &mut BumpAllocator {
        &mut self.bufs[self.current]
    }

    pub fn clear(&mut self) {
        self.current_mut().clear()
    }
}
