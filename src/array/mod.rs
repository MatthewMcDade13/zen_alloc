use core::slice;
use std::{
    alloc::{dealloc, Layout},
    cell::Cell,
    marker::PhantomData,
    mem::{align_of, size_of},
    ops::{Deref, DerefMut, Index},
    ptr::NonNull,
};

use crate::mem::{BlockVec, Byteable, Bytes, Unbounded};

pub mod htable;
pub mod str;

#[repr(transparent)]
#[derive(Debug)]
pub struct ArrayPtr<'alloc, T: Byteable> {
    ptr: NonNull<Array<'alloc, T>>,
}

impl<'a, T> Copy for ArrayPtr<'a, T> where T: Byteable {}
impl<'a, T> Clone for ArrayPtr<'a, T>
where
    T: Byteable,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T> ArrayPtr<'a, T>
where
    T: Byteable,
{
    pub const fn as_ptr(&self) -> *const Array<'a, T> {
        self.ptr.as_ptr()
    }

    pub const fn as_ref(&self) -> &'a Array<'a, T> {
        unsafe { self.ptr.as_ref() }
    }

    pub fn as_mut(&mut self) -> &mut Array<'a, T> {
        unsafe { self.ptr.as_mut() }
    }
}

impl<'a, T> Deref for ArrayPtr<'a, T>
where
    T: Byteable,
{
    type Target = Array<'a, T>;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a, T> DerefMut for ArrayPtr<'a, T>
where
    T: Byteable,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

impl<'a, T> From<NonNull<Array<'a, T>>> for ArrayPtr<'a, T>
where
    T: Byteable,
{
    fn from(value: NonNull<Array<'a, T>>) -> Self {
        Self { ptr: value }
    }
}

// TODO:: Add capacity for Array/Slice

/// Actual allocated block of memory
#[repr(C)]
#[derive(Debug)]
pub struct Array<'alloc, T: Byteable> {
    rc: usize,

    ptr: &'alloc [T],
    // _phantom: PhantomData<&'alloc [T]>,
}

impl<'a, T> Index<usize> for Array<'a, T>
where
    T: Byteable,
{
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.ptr[index]
    }
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

    pub fn index_ptr(&self, index: usize) -> NonNull<T> {
        let elem = self.index(index) as *const T;
        NonNull::new(elem as _).expect("attempt to read invalid pointer!!!")
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
    pub fn index_ptr(&self, index: usize) -> NonNull<T> {
        self.ptr.index_ptr(index)
    }

    pub(crate) fn inner_ptr(&self) -> ArrayPtr<'a, T> {
        let p = self.ptr.deref();
        let ptr = std::ptr::from_ref(p);
        NonNull::new(ptr as _)
            .expect("inner pointer is null!!!")
            .into()
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
    Fallback(ArrayPtr<'alloc, T>),
}

impl<'a, T> Slice<'a, T>
where
    T: Byteable,
{
    pub fn len(&self) -> usize {
        match self {
            Slice::Block(slice) => slice.inner_ptr().len(),
            Slice::Fallback(arr) => arr.len(),
        }
    }

    pub(crate) fn inner_ptr(&self) -> ArrayPtr<'a, T> {
        match self {
            Slice::Block(ref slice) => slice.inner_ptr(),
            Slice::Fallback(arr) => *arr,
        }
    }

    pub fn index_ptr(&self, index: usize) -> NonNull<T> {
        match *self {
            Slice::Block(ref slice) => slice.index_ptr(index),
            Slice::Fallback(arr) => unsafe { arr.as_ref().index_ptr(index) },
        }
    }
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

#[derive(Debug, Clone, Copy)]
pub struct SliceIter<'a, T: Byteable> {
    curr_index: usize,
    len: usize,
    parent: ArrayPtr<'a, T>,
    _phantom: PhantomData<&'a T>,
}

impl<'a, T> Index<usize> for SlicePtr<'a, T>
where
    T: Byteable,
{
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        let ptr = self.inner_ptr();
        let arr: &'a Array<'a, T> = ptr.as_ref();
        arr.index(index)
    }
}

impl<'a, T> Index<usize> for Slice<'a, T>
where
    T: Byteable,
{
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        match *self {
            Slice::Block(ref slice) => slice.index(index),
            Slice::Fallback(arr) => unsafe { arr.as_ref().index(index) },
        }
    }
}

impl<'a, T> SliceIter<'a, T>
where
    T: Byteable,
{
    pub fn inner_ref(&self) -> &Array<'a, T> {
        unsafe { self.parent.as_ref() }
    }
}

impl<'a, T> std::iter::Iterator for SliceIter<'a, T>
where
    T: Byteable,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.curr_index >= self.len {
            None
        } else {
            let i = self.curr_index;
            self.curr_index += 1;
            let arr = self.inner_ref();
            let elem = arr.index_ptr(i);
            let elem = unsafe { NonNull::read(elem) };
            Some(elem)
        }
    }
}

impl<'a, T> std::iter::DoubleEndedIterator for SliceIter<'a, T>
where
    T: Byteable,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.curr_index <= 0 {
            None
        } else {
            let i = self.curr_index;
            self.curr_index -= 1;
            let arr = self.inner_ref();
            let elem = arr.index_ptr(i);
            let elem = unsafe { NonNull::read(elem) };
            Some(elem)
        }
    }
}

impl<'a, T> std::iter::IntoIterator for Slice<'a, T>
where
    T: Byteable,
{
    type Item = T;

    type IntoIter = SliceIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        Self::IntoIter {
            curr_index: 0,
            len: self.len(),
            parent: self.inner_ptr(),
            _phantom: PhantomData,
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
