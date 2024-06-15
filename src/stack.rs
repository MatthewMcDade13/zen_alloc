use std::mem::align_of;

use anyhow::bail;

use crate::ptr::BasicPtr;

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

    pub fn alloc<T>(&mut self, data: T) -> anyhow::Result<BasicPtr<T>>
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

            let sp = BasicPtr { ptr };
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
