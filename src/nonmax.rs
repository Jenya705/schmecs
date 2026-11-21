use crate::option::PureOption;
use std::num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128};
use std::ptr::NonNull;

macro_rules! impl_for {
    ($nonmax: ident, $optional_nonmax: ident, $raw_nonmax: ident, $nzty: ty, $ty: ty) => {
        #[repr(transparent)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        // TODO: use unstable pattern feature?
        pub struct $nonmax($nzty);

        impl PartialOrd for $nonmax {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                self.get().partial_cmp(&other.get())
            }
        }

        impl Ord for $nonmax {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.get().cmp(&other.get())
            }
        }

        impl std::hash::Hash for $nonmax {
            fn hash<H>(&self, state: &mut H)
                where H: std::hash::Hasher
            {
                self.get().hash(state)
            }
        }

        impl $nonmax {
            pub const ZERO: Self = Self::new(0).unwrap();
            pub const ONE: Self = Self::new(1).unwrap();

            pub const fn get(self) -> $ty {
                self.0.get() ^ <$ty>::MAX
            }

            pub const fn new(val: $ty) -> Option<Self> {
                match std::num::NonZero::new(val ^ <$ty>::MAX) {
                    Some(val) => Some(Self(val)),
                    None => None,
                }
            }

            pub const fn from_raw_value(val: $ty) -> Option<Self> {
                match <$nzty>::new(val) {
                    Some(val) => Some(Self(val)),
                    None => None,
                }
            }

            pub const fn from_inverse(val: $ty) -> Option<Self> {
                Self::from_raw_value(val)
            }

            pub const fn get_raw_value(self) -> $ty {
                self.0.get()
            }

            pub const fn get_inverse(self) -> $ty {
                self.get_raw_value()
            }

            pub const fn from_raw_nonmax(val: $raw_nonmax) -> Self {
                // SAFETY: $raw_nonmax never contains the maximum value.
                unsafe {
                    Self::new_unchecked(val.get())
                }
            }

            /// # Safety
            /// `val` must not be equal to the maximum value of the underlying data type.
            pub const unsafe fn new_unchecked(val: $ty) -> Self {
                // SAFETY: The caller's contract ensures that the value doesn't equal the maximum.
                // The maximum in the bit representation is all ones hence the only possibility when
                // the value is zero is when `val` is all ones which is the maximum value which it
                // cannot be as per the contract.
                Self(unsafe { std::num::NonZero::new_unchecked(val ^ <$ty>::MAX) })
            }
        }

        const _: () = {
            assert!(std::mem::size_of::<$nonmax>() == std::mem::size_of::<Option<$nonmax>>());
        };

        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct $raw_nonmax($ty);

        impl $raw_nonmax {
            pub const fn new(val: $ty) -> Option<Self> {
                if val == <$ty>::MAX {
                    None
                } else {
                    // SAFETY: `val` is not $ty::MAX.
                    Some(unsafe { Self::new_unchecked(val) })
                }
            }

            #[doc = concat!(
                "Safety: \n`val` may not be equal to [`",
                stringify!($ty),
                "::MAX`]."
            )]
            pub const unsafe fn new_unchecked(val: $ty) -> Self {
                Self(val)
            }

            pub const fn from_nonmax(val: $nonmax) -> Self {
                Self(val.get())
            }

            pub const fn get(self) -> $ty {
                self.0
            }

            #[doc = concat!(
                "Safety: \n`val` may not be equal to [`",
                stringify!($ty),
                "::MAX`]."
            )]
            pub(crate) const unsafe fn from_ref_unchecked(val: &$ty) -> &Self {
                // SAFETY: The caller's contract and ($ty and Self) are equivalent.
                unsafe { NonNull::from_ref(val).cast().as_ref() }
            }

            #[doc = concat!(
                "Safety: \n`val` may not be equal to [`",
                stringify!($ty),
                "::MAX`]."
            )]
            pub(crate) const unsafe fn from_mut_unchecked(val: &mut $ty) -> &mut Self {
                // SAFETY: The caller's contract and ($ty and Self) are equivalent.
                unsafe { NonNull::from_mut(val).cast().as_mut() }
            }
        }

        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct $optional_nonmax($ty);

        impl $optional_nonmax {
            pub const NONE: Self = Self(<$ty>::MAX);

            pub const fn get(self) -> Option<$nonmax> {
                $nonmax::new(self.0)
            }

            pub const fn get_raw(self) -> $ty {
                self.0
            }

            pub const fn get_raw_ref(&self) -> &$ty {
                &self.0
            }

            pub const fn get_raw_mut(&mut self) -> &mut $ty {
                &mut self.0
            }

            pub const fn new(val: $ty) -> Self {
                Self(val)
            }

            pub const fn from_nonmax(val: $nonmax) -> Self {
                Self::new(val.get())
            }

            pub const fn from_nonmax_option(val: Option<$nonmax>) -> Self {
                match val {
                    Some(val) => Self::from_nonmax(val),
                    None => Self::NONE,
                }
            }
        }

        unsafe impl PureOption for $optional_nonmax {
            type V = $raw_nonmax;

            fn some(val: Self::V) -> Self {
                Self(val.get())
            }

            fn none() -> Self {
                Self::NONE
            }

            fn from(val: Option<Self::V>) -> Self {
                match val {
                    Some(val) => Self::some(val),
                    None => Self::NONE,
                }
            }

            fn is_some(&self) -> bool {
                self.get_raw() != <$ty>::MAX
            }

            fn as_ref(&self) -> Option<&Self::V> {
                if self.is_some() {
                    // SAFETY: `self.0` doesn't equal the maximum value.
                    Some(unsafe { Self::V::from_ref_unchecked(&self.0) })
                } else {
                    None
                }
            }

            fn as_mut(&mut self) -> Option<&mut Self::V> {
                if self.is_some() {
                    // SAFETY: `self.0` doesn't equal the maximum value.
                    Some(unsafe { Self::V::from_mut_unchecked(&mut self.0) })
                } else {
                    None
                }
            }

            fn into(self) -> Option<Self::V> {
                Self::V::new(self.0)
            }

            fn replace(&mut self, value: Self::V) -> Self {
                std::mem::replace(self, Self(value.get()))
            }

            fn take(&mut self) -> Self {
                std::mem::replace(self, Self::NONE)
            }
        }

        impl From<$nonmax> for $optional_nonmax {
            fn from(val: $nonmax) -> Self {
                Self::new(val.get())
            }
        }

        impl Default for $optional_nonmax {
            fn default() -> Self {
                Self::NONE
            }
        }

        impl<'a> From<&'a $optional_nonmax> for Option<&'a $ty> {
            fn from(val: &'a $optional_nonmax) -> Self {
                if val.0 == <$ty>::MAX {
                    None
                } else {
                    Some(&val.0)
                }
            }
        }

        impl From<$optional_nonmax> for Option<$ty> {
            fn from(val: $optional_nonmax) -> Self {
                if val.0 == <$ty>::MAX {
                    None
                } else {
                    Some(val.0)
                }
            }
        }
    };
}

impl_for!(NonMaxU8, OptionalNonMaxU8, RawNonMaxU8, NonZeroU8, u8);
impl_for!(NonMaxU16, OptionalNonMaxU16, RawNonMaxU16, NonZeroU16, u16);
impl_for!(NonMaxU32, OptionalNonMaxU32, RawNonMaxU32, NonZeroU32, u32);
impl_for!(NonMaxU64, OptionalNonMaxU64, RawNonMaxU64, NonZeroU64, u64);
impl_for!(
    NonMaxU128,
    OptionalNonMaxU128,
    RawNonMaxU128,
    NonZeroU128,
    u128
);
