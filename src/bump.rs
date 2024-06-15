use std::{
    alloc::{alloc, dealloc, Layout},
    mem::align_of,
};

use anyhow::bail;

use crate::ptr::{BasicPtr, BumpPtr};

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

            let sp = BasicPtr { ptr };
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
