use std::{
    alloc::{alloc, dealloc, Layout},
    mem::align_of,
    ops::{Deref, DerefMut},
};

use crate::ptr::BasicPtr;

#[derive(Debug)]
struct PoolCell<T> {
    cell: T,
    slot: isize,
    next: isize,
    valid: bool,
}

#[derive(Debug)]
pub struct PoolPtr<T>(BasicPtr<PoolCell<T>>);

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
            PoolPtr(BasicPtr { ptr })
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
