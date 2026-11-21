// NOTE: pure here might mean something else than in other contexts. More like well-behaved.

use std::ops::Deref;

use crate::{gorc::GorcRef, pure::__hidden::Sealed};

pub(crate) mod __hidden {
    pub unsafe trait Sealed {}
}

/// # Safety
/// Method [`std::ops::Deref::deref`] must be pure in a sense if the same self is given, the same
/// value must be returned (whatever that might mean).
///
/// Safety clause ensures that as long as non-mutable alias to the object exists, it's
/// [`std::ops::Deref::deref`] returns the same reference everytime.
pub unsafe trait DerefPure: __hidden::Sealed + Deref {}

/// # Safety
/// Methods of [`std::cmp::Ord`] must be pure.
///
/// This trait also must imply that the following traits are pure:
/// - [`std::cmp::PartialOrd`]
/// - [`std::cmp::PartialEq`]
/// - [`std::cmp::Eq`]
pub unsafe trait OrdPure: __hidden::Sealed + Ord {}

/// # Safety
/// [std::clone::Clone] must be pure in the same sense as [`std::ops::Deref::deref`].
pub unsafe trait ClonePure: __hidden::Sealed + Clone {}

macro_rules! impl_for {
    ($trait: ident for $($ty:ty),*$(,)?) => {
        $(
            // SAFETY: The caller of the macro ensures safety.
            unsafe impl $trait for $ty {}
        )*
    }
}

impl_for!(Sealed for u8, i8, u16, i16, u32, i32, u64, i64, u128, i128, usize);
impl_for!(OrdPure for u8, i8, u16, i16, u32, i32, u64, i64, u128, i128, usize);
impl_for!(ClonePure for u8, i8, u16, i16, u32, i32, u64, i64, u128, i128, usize);

unsafe impl<'a, T> Sealed for &'a [T] {}
unsafe impl<'a, T> DerefPure for &'a [T] {}
unsafe impl<'a, T> ClonePure for &'a [T] {}

unsafe impl<'a, T> Sealed for [T] {}
// unsafe impl<'a, T> DerefPure for [T] {}

unsafe impl<'a, T, const SIZE: usize> Sealed for [T; SIZE] {}
unsafe impl<'a, T, const SIZE: usize> ClonePure for [T; SIZE] where T: ClonePure {}

unsafe impl<T> Sealed for Box<[T]> {}
unsafe impl<T> DerefPure for Box<[T]> {}
unsafe impl<T> ClonePure for Box<[T]> where T: ClonePure {}

unsafe impl<T> Sealed for Vec<T> {}
unsafe impl<T> DerefPure for Vec<T> {}
unsafe impl<T> ClonePure for Vec<T> where T: ClonePure {}

unsafe impl<'a, T: ?Sized> Sealed for GorcRef<'a, T> {}
unsafe impl<'a, T: ?Sized> DerefPure for GorcRef<'a, T> {}
// SAFETY: Points to the same object hence pure.
unsafe impl<'a, T: ?Sized> ClonePure for GorcRef<'a, T> {}
