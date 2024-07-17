use zen_alloc::{
    block::MemPool,
    mem::{BlockVec, Byteable},
};

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Point {
    x: f32,
    y: f32,
}

unsafe impl Byteable for Point {}

fn main() -> anyhow::Result<()> {
    let pool = MemPool::<512>::new(32);

    // let p = pool.alloc(Point { x: 56., y: 69. })?;
    // let x = pool.alloc(4)?;
    // let y = pool.alloc(usize::MAX)?;

    // let inner = BlockPtr::deref(p);

    // assert_eq!(p.x, 56.);
    // assert_eq!(p.y, 69.);
    // assert_eq!(*x, 4);
    // assert_eq!(*y, usize::MAX);
    Ok(())
}
