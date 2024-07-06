use crate::{
    mem::{BlockVec, MemCell, Unbounded},
    ptr::ZenPtr,
};

pub mod const_sized;

pub struct Slab {
    blocks: [BlockVec; 11],
}

impl Slab {
    const SIZES: [usize; 11] = [2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048];

    pub fn new() -> Self {
        let blocks = Self::SIZES.map(|size| BlockVec::new(size));
        Self { blocks }
    }

    pub fn alloc<T>(&self, val: T) -> anyhow::Result<ZenPtr<T>>
    where
        T: MemCell,
    {
        if let Some(block) = self.block_for::<T>() {
            if block.is_uninit() {
                block.resize(2);
            } else if block.is_full() {
                block.resize(block.len() * 2);
            }

            let ptr = block.alloc(val)?;
            let ptr = ZenPtr::Unbounded(ptr);
            Ok(ptr)
        } else {
            let ptr = unsafe { crate::mem::allocate(val) };
            let ptr = ZenPtr::Raw(ptr);
            Ok(ptr)
        }
    }

    pub fn free<T>(&self, ptr: Unbounded<T>)
    where
        T: MemCell,
    {
        if let Some(block) = self.block_for::<T>() {
            block.free(ptr)
        } else {
            unreachable!("We dont allocate Unbounded Pointers if the type size exceeds Max Slab BlockVec size")
        }
    }

    fn block_for<T>(&self) -> Option<&BlockVec>
    where
        T: MemCell,
    {
        let size = std::mem::size_of::<T>();
        for (i, s) in Self::SIZES.iter().enumerate() {
            if size <= *s {
                return Some(&self.blocks[i]);
            }
        }
        None
    }
}
