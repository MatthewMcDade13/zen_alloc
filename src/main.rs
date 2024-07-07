use zen_alloc::mem::BlockVec;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Point {
    x: f32,
    y: f32,
}

fn main() -> anyhow::Result<()> {
    let bv = BlockVec::with_capacity(512, 40);

    // let p = bv.alloc(Point { x: 56., y: 69. });
    let x = bv.alloc(4)?;
    let y = bv.alloc(usize::MAX)?;

    // let inner = RawRef::deref(p);

    // assert_eq!(p.x, 56.);
    // assert_eq!(p.y, 69.);
    assert_eq!(*x, 4);
    assert_eq!(*y, usize::MAX);
    Ok(())
}
