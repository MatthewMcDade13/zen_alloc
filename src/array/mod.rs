use core::slice;
use std::{
    alloc::{dealloc, Layout},
    cell::Cell,
    marker::PhantomData,
    mem::{align_of, size_of},
    ops::Deref,
    ptr::NonNull,
};

use crate::mem::{Byteable, Bytes, Unbounded};

pub mod htable;
pub mod str;

// TODO:: Add capacity for Array/Slice

/// Actual allocated block of memory
#[repr(C)]
#[derive(Debug)]
pub(crate) struct Array<'alloc, T: Byteable> {
    rc: usize,
    ptr: &'alloc [T],
    // _phantom: PhantomData<&'alloc [T]>,
}

impl<'a, T> Array<'a, T>
where
    T: Byteable,
{
    pub unsafe fn from_alloced(ptr: NonNull<u8>, len: usize) -> NonNull<Self> {
        unsafe {
            let head = ptr.cast::<Array<T>>();
            let arr = head.add(1).cast::<u8>();
            let offset = arr.align_offset(align_of::<T>());
            let arr = arr.add(offset).cast::<T>();

            let s = {
                let ptr = slice::from_raw_parts(arr.as_ptr(), len);

                Self { rc: 1, ptr }
            };
            // let arr = Array::from_raw_parts(arr, 1, len);

            NonNull::write(head, s);
            head
        }
    }

    pub const fn len(&self) -> usize {
        self.ptr.len()
    }

    pub const fn size_bytes(&self) -> usize {
        Self::size_of(self.len())
    }

    pub const fn size_of(len: usize) -> usize {
        (size_of::<T>() * len) + size_of::<Self>()
    }
}

unsafe impl<'a, T> Byteable for Array<'a, T> where T: Byteable {}

/// Points to array (actual block of memory)
#[repr(transparent)]
#[derive(Debug)]
pub struct SlicePtr<'alloc, T: Byteable> {
    ptr: Unbounded<'alloc, Array<'alloc, T>>,
    _phantom: PhantomData<&'alloc [T]>,
}
impl<'a, T> Clone for SlicePtr<'a, T>
where
    T: Byteable,
{
    fn clone(&self) -> Self {
        let inner = Unbounded::clone(&self.ptr);

        Self {
            ptr: inner,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T> SlicePtr<'a, T>
where
    T: Byteable,
{
    pub fn inner_ptr(&self) -> NonNull<Array<'a, T>> {
        let p = self.ptr.deref();
        let ptr = std::ptr::from_ref(p);
        NonNull::new(ptr as _).expect("inner pointer is null!!!")
    }
    // pub unsafe fn from_raw_parts(ptr: NonNull<T>, len: usize) -> Self {
    //     let ptr = Unbounded::from(ptr);
    //     Self {
    //         ptr,
    //         ..Default::default()
    //     }
    // }
}

impl<'a, T> From<Unbounded<'a, Array<'a, T>>> for SlicePtr<'a, T>
where
    T: Byteable,
{
    fn from(value: Unbounded<'a, Array<'a, T>>) -> Self {
        let ptr = value;
        Self {
            ptr,
            _phantom: PhantomData,
        }
    }
}

unsafe impl<'a, T> Byteable for SlicePtr<'a, T> where T: Byteable {}

#[derive(Debug)]
pub enum Slice<'alloc, T: Byteable> {
    Block(SlicePtr<'alloc, T>),
    Fallback(NonNull<Array<'alloc, T>>),
}

impl<'a, T> Clone for Slice<'a, T>
where
    T: Byteable,
{
    fn clone(&self) -> Self {
        match self {
            Slice::Block(slice) => {
                let mut ptr = slice.inner_ptr();

                let arr = unsafe { ptr.as_mut() };
                arr.rc += 1;
                let ptr = SlicePtr::clone(slice);
                Slice::Block(ptr)
            }
            Slice::Fallback(mut arr) => {
                let arr_ref = unsafe { arr.as_mut() };
                arr_ref.rc += 1;
                Slice::Fallback(arr)
            }
        }
    }
}

impl<'a, T> Drop for Slice<'a, T>
where
    T: Byteable,
{
    fn drop(&mut self) {
        match self {
            Slice::Block(slice) => {
                let mut ptr = slice.inner_ptr();

                let arr = unsafe { ptr.as_mut() };
                arr.rc -= 1;
                if arr.rc == 0 {
                    let slice_ptr = Unbounded::clone(&slice.ptr);
                    slice_ptr.free();
                }
            }
            Slice::Fallback(mut arr) => {
                let arr_ref = unsafe { arr.as_mut() };

                arr_ref.rc -= 1;
                if arr_ref.rc == 0 {
                    let layout = Layout::array::<T>(arr_ref.size_bytes())
                        .expect("attempt to deallocate invalid pointer!!!!");
                    unsafe { dealloc(arr.as_ptr() as _, layout) };
                }
            }
        }
    }
}
