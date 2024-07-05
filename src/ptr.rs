use core::slice;
use std::{
    alloc::{alloc, dealloc, Layout},
    marker::PhantomData,
    mem::align_of,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};

use crate::slab;

// pub type Nil<T> = &'static T;
const GLOBAL_NIL: usize = 80085101;
#[derive(Debug, Clone, Copy)]
pub struct Nil(&'static usize);
impl Nil {
    pub const fn get() -> NonNull<Nil> {
        let r = NIL.0;
        let r = std::ptr::from_ref(r);
        unsafe { NonNull::new_unchecked(r as *mut _) }
    }

    #[inline]
    pub fn addr() -> usize {
        Self::get().as_ptr() as usize
    }

    #[inline]
    pub fn is_nil<T>(v: *const T) -> bool {
        let a = v as usize;
        let b = Self::addr();
        a == b
    }
}
pub const NIL: Nil = Nil(&GLOBAL_NIL);

#[derive(Debug)]
pub struct Owned<T> {
    ptr: *mut T,
    _phantom: PhantomData<T>,
}

impl<T> Owned<T> {
    const LAYOUT: Layout = Layout::new::<T>();

    pub fn new(v: T) -> Self {
        unsafe {
            let ptr = alloc(Self::LAYOUT).cast::<T>();
            std::ptr::write(ptr, v);

            Self {
                ptr,
                _phantom: PhantomData,
            }
        }
    }

    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        Self {
            ptr,
            _phantom: PhantomData,
        }
    }
}

impl<T> Drop for Owned<T> {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr.cast::<u8>(), Self::LAYOUT) }
    }
}

#[derive(Debug)]
pub struct BasicPtr<T>
where
    T: Sized,
{
    pub ptr: *mut T,
}

impl<T> Clone for BasicPtr<T> {
    fn clone(&self) -> Self {
        Self { ptr: self.ptr }
    }
}

impl<T> DerefMut for BasicPtr<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            self.ptr
                .as_mut()
                .expect("Attempted to dereference null RadPtr")
        }
    }
}

impl<T> Deref for BasicPtr<T> {
    fn deref(&self) -> &Self::Target {
        unsafe {
            self.ptr
                .as_ref()
                .expect("Attempted to dereference null RadPtr")
        }
    }

    type Target = T;
}

pub type BumpPtr<T> = BasicPtr<T>;
