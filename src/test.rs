#[cfg(test)]
mod tests {
    use crate::{
        bump::BumpAllocator,
        mem::{BlockVec, RawRef},
        stack::StackAllocator,
    };

    #[repr(C)]
    #[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct Point {
        x: f64,
        y: f64,
    }

    #[test]
    fn stack() -> anyhow::Result<()> {
        let mut sa = StackAllocator::<4096>::new();
        {
            let x = sa.alloc(4)?;
            let p = sa.alloc(Point { x: 56.0, y: 69. })?;
            let y = sa.alloc(56.)?;

            assert_eq!(4, *x);
            assert_eq!(56., *y);
            assert_eq!(p.x, 56.0);
            assert_eq!(p.y, 69.0);

            sa.clear();
        }

        const S: &'static str = "aye lmao";
        let x = sa.alloc(String::from(S))?;
        assert_eq!(*x, S);
        Ok(())
    }

    #[test]
    fn bump() -> anyhow::Result<()> {
        let mut ba = BumpAllocator::new(4096)?;

        {
            let x = ba.alloc(4)?;
            let p = ba.alloc(Point { x: 56.0, y: 69. })?;
            let y = ba.alloc(usize::MAX)?;

            assert_eq!(4, *x);
            assert_eq!(usize::MAX, *y);
            assert_eq!(p.x, 56.0);
            assert_eq!(p.y, 69.0);

            ba.clear();
        }

        const S: &'static str = "aye lmao";
        let x = ba.alloc(String::from(S))?;
        assert_eq!(*x, S);

        Ok(())
    }

    // TODO :: Debug this, something isnt writing/reading memory correctly...
    #[test]
    fn block_vec() -> anyhow::Result<()> {
        let bv = BlockVec::with_capacity(512, 40);

        let p = bv.alloc(Point { x: 56., y: 69. });
        let x = bv.alloc(4).into_scoped();
        let y = bv.alloc(usize::MAX).into_scoped();

        // let inner = RawRef::deref(p);

        assert_eq!(p.x, 56.);
        assert_eq!(p.y, 69.);
        assert_eq!(*x, 4);
        assert_eq!(*y, usize::MAX);

        Ok(())
    }
}
