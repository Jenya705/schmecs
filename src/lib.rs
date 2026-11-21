pub mod access;
pub mod bitset;
pub mod bundle;
pub mod component;
pub mod debug;
pub mod dyn_key;
pub mod entity;
pub mod fragment;
pub mod gorc;
pub mod hash_map;
pub mod iter;
pub mod math;
pub mod nonmax;
pub mod option;
pub mod page;
pub mod pure;
pub mod slice;
pub mod smart;
pub mod sparse;
pub mod storage;
pub mod vec;
pub mod world;

#[derive(thiserror::Error, Debug)]
#[error("Allocation failure")]
pub struct AllocError;

#[macro_export]
#[doc(hidden)]
macro_rules! id_struct {
    ($vis: vis $name: ident = NonMaxU32) => {
        #[derive(Hash, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        $vis struct $name($crate::nonmax::NonMaxU32);

        impl $name {
            pub const fn new(id: $crate::nonmax::NonMaxU32) -> Self {
                Self(id)
            }

            pub const fn id(&self) -> $crate::nonmax::NonMaxU32 {
                self.0
            }

            pub const fn id_u32(&self) -> u32 {
                self.0.get()
            }

            pub const fn id_usize(&self) -> usize {
                self.0.get() as usize
            }
        }

        unsafe impl $crate::pure::__hidden::Sealed for $name {}
        unsafe impl $crate::pure::OrdPure for $name {}
        unsafe impl $crate::pure::ClonePure for $name {}

        impl From<$name> for usize {
            fn from(obj: $name) -> usize {
                obj.id_usize()
            }
        }
    };
    ($vis: vis $name: ident = u32) => {
        #[derive(Hash, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        $vis struct $name(u32);

        impl $name {
            pub const fn new(id: u32) -> Self {
                Self(id)
            }

            pub const fn id(&self) -> u32 {
                self.0
            }

            pub const fn id_u32(&self) -> u32 {
                self.0
            }

            pub const fn id_usize(&self) -> usize {
                self.0 as usize
            }
        }

        unsafe impl $crate::pure::__hidden::Sealed for $name {}
        unsafe impl $crate::pure::OrdPure for $name {}
        unsafe impl $crate::pure::ClonePure for $name {}

        impl From<$name> for usize {
            fn from(obj: $name) -> usize {
                obj.id_usize()
            }
        }
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! unsafe_cell {
    ($vis: vis $name: ident<$lt:lifetime$(,$generic:ident)*$(,)?> = $ty: ty, $access: ty $(,$additional: ty)?) => {
        $vis struct $name<$lt, $($generic,)* AC = $crate::access::checked::DebugAccessCheck<$access>, > {
            ptr: std::ptr::NonNull<$ty>,
            $(additional: $additional,)*
            access: AC,
            _marker: std::marker::PhantomData<(&$lt $ty, &$lt std::cell::UnsafeCell<$ty>)>,
        }

        // SAFETY: Both mutable and immutable references and access variable implement Sync.
        unsafe impl<$lt, AC, $($generic,)*> Sync for $name<$lt, $($generic,)* AC,>
        where
            &$lt $ty: Sync,
            &$lt mut $ty: Sync,
            $($additional: Sync,)*
            AC: Sync,
        {
        }

        // SAFETY: Both mutable and immutable references and access variable implement Send.
        unsafe impl<$lt, AC, $($generic,)*> Send for $name<$lt, $($generic,)* AC,>
        where
            &$lt $ty: Send,
            &$lt mut $ty: Send,
            $($additional: Send,)*
            AC: Send,
        {
        }

        impl<$lt, AC, $($generic,)*> $name<$lt, $($generic,)* AC>
        where
            AC: $crate::access::checked::AccessCheck<Inner = $access>,
        {
            /// # Safety
            /// `ptr` must be dereferencable.
            pub unsafe fn new(
                ptr: std::ptr::NonNull<$ty>,
                access: AC,
                $(additional: $additional,)*
            ) -> Self {
                Self {
                    ptr,
                    access,
                    $(additional: {
                        let additional: $additional = additional;
                        additional
                    },)*
                    _marker: std::marker::PhantomData,
                }
            }

            pub fn ptr(&self) -> std::ptr::NonNull<$ty> {
                self.ptr
            }

            $(
                pub fn additional(&self) -> &$additional {
                    &self.additional
                }
            )*

            pub fn access(&self) -> &AC {
                &self.access
            }
        }
    }
}

pub(crate) struct AbortOnDrop<'a>(&'a str);

impl<'a> Drop for AbortOnDrop<'a> {
    fn drop(&mut self) {
        panic!("{}", self.0);
    }
}
