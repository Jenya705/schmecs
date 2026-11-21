use std::ops::{BitAnd, BitAndAssign, BitOrAssign, Not, Shl};

#[cfg_attr(kani, derive(kani::Arbitrary))]
pub struct PrimitiveBitSet<T>(T);

impl<T> PrimitiveBitSet<T> {
    pub fn empty() -> Self
    where
        T: From<u8>,
    {
        Self(0u8.into())
    }

    pub fn set(&mut self, index: u32)
    where
        T: From<u8>,
        T: BitOrAssign,
        T: Shl<u32, Output = T>,
    {
        let one: T = 1u8.into();
        self.0 |= one << index;
    }

    pub fn unset(&mut self, index: u32)
    where
        T: From<u8>,
        T: Not<Output = T>,
        T: BitAndAssign,
        T: Shl<u32, Output = T>,
    {
        let one: T = 1u8.into();
        self.0 &= !(one << index);
    }

    pub fn get(&self, index: u32) -> bool
    where
        T: From<u8>,
        T: PartialEq,
        T: BitAnd<Output = T>,
        T: Shl<u32, Output = T>,
        T: Clone,
    {
        let one: T = 1u8.into();
        self.0.clone() & (one << index) != (0u8.into())
    }

    pub fn clear(&mut self)
    where
        T: From<u8>,
    {
        *self = Self::empty();
    }
}

#[cfg(kani)]
mod proofs {
    use crate::bitset::PrimitiveBitSet;

    #[kani::proof]
    fn primitive_bit_set_u32_set_proof() {
        let mut bs: PrimitiveBitSet<u32> = kani::any();
        let index: u32 = kani::any_where(|v| *v < 32);
        bs.set(index);
        assert!(bs.get(index));
    }

    #[kani::proof]
    fn primitive_bit_set_u32_unset_proof() {
        let mut bs: PrimitiveBitSet<u32> = kani::any();
        let index: u32 = kani::any_where(|v| *v < 32);
        bs.unset(index);
        assert!(!bs.get(index));
    }
}
