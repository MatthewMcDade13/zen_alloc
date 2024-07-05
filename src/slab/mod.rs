use crate::mem::{BlockPtrRaw, BlockVec, MemCell};

pub mod const_sized;

pub struct Slab {
    blocks: Vec<BlockVec>,
}

impl Slab {
    const SIZES: [usize; 11] = [2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048];

    pub fn new() -> Self {
        let blocks = Self::SIZES.map(|size| BlockVec::new(size)).to_vec();
        Self { blocks }
    }

    pub fn alloc<T>(&self, val: T) -> anyhow::Result<BlockPtrRaw<T>>
    where
        T: MemCell,
    {
        let block = self.block_for::<T>().unwrap();
        block.alloc(val)
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
