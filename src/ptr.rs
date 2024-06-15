use std::ops::{Deref, DerefMut};

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

