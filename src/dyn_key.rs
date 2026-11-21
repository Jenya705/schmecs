use std::{
    any::TypeId,
    borrow::Borrow,
    hash::{Hash, Hasher},
    ops::Deref,
};

use crate::gorc::GorcRef;

pub trait DynHash {
    fn dyn_hash(&self, state: &mut dyn Hasher);
}

impl<T: ?Sized> DynHash for T
where
    T: Hash,
{
    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        self.hash(&mut state);
    }
}

/// # Safety
/// `other` in [`DynEq::eq`] must be interpreted as [`Self`] type.
pub unsafe trait DynUnsafeEq {
    /// # Safety
    /// `other` must be an aligned pointer to the underlying non-dynamic object.
    unsafe fn eq(&self, other: *const u8) -> bool;
}

// SAFETY: `other` is indeed interpreted as `T`.
unsafe impl<T> DynUnsafeEq for T
where
    T: Eq,
{
    unsafe fn eq(&self, other: *const u8) -> bool {
        // SAFETY: The caller's contract.
        unsafe {
            let other = other.cast::<T>().as_ref().unwrap_unchecked();
            self.eq(other)
        }
    }
}

pub trait DynUnsafeKey: DynHash + DynUnsafeEq {}

impl<T> DynUnsafeKey for T where T: DynHash + DynUnsafeEq {}

#[repr(C)]
pub struct DynKey<K> {
    // TODO: ffi way?
    tid: TypeId,
    key: K,
}

impl DynKey<Box<dyn DynUnsafeKey>> {
    pub fn new_boxed<T>(key: Box<T>) -> Self
    where
        T: 'static,
        T: DynHash + DynUnsafeEq,
    {
        Self {
            tid: std::any::TypeId::of::<T>(),
            key,
        }
    }
}

impl<'a, K: ?Sized> DynKey<GorcRef<'a, K>> {
    pub fn from_gorc_ref(key: GorcRef<'a, K>) -> Self
    where
        K: 'static,
        K: DynUnsafeKey,
        &'a K: Deref<Target = K>,
    {
        Self {
            tid: std::any::TypeId::of::<K>(),
            key,
        }
    }
}

impl<'a, K: ?Sized> DynKey<&'a K> {
    pub fn from_ref(key: &'a K) -> Self
    where
        K: 'static,
        K: DynUnsafeKey,
        &'a K: Deref<Target = K>,
    {
        Self {
            tid: std::any::TypeId::of::<K>(),
            key,
        }
    }
}

impl<'a, K> From<DynKey<&'a K>> for DynKey<Box<dyn DynUnsafeKey>>
where
    K: 'static,
    K: DynUnsafeKey,
    K: Clone,
{
    fn from(value: DynKey<&'a K>) -> Self {
        Self::new_boxed(Box::new(value.key.clone()))
    }
}

impl<'a, K> From<DynKey<&'a K>> for DynKey<GorcRef<'static, dyn DynUnsafeKey>>
where
    K: 'static,
    K: DynUnsafeKey,
    K: Clone,
{
    fn from(value: DynKey<&'a K>) -> Self {
        Self::from_gorc_ref(GorcRef::from_box(Box::new(value.key.clone())))
    }
}

impl<'a, 'b, K> From<&'b DynKey<&'a K>> for DynKey<&'b dyn DynUnsafeKey>
where
    K: DynUnsafeKey,
{
    fn from(value: &'b DynKey<&'a K>) -> Self {
        Self {
            tid: value.tid,
            key: value.key,
        }
    }
}

impl<'a> Borrow<DynKey<&'a dyn DynUnsafeKey>> for DynKey<Box<dyn DynUnsafeKey>> {
    fn borrow(&self) -> &DynKey<&'a dyn DynUnsafeKey> {
        // SAFETY: Box<T> is the same as &T.
        unsafe { std::mem::transmute(self) }
    }
}

impl<'a> Borrow<DynKey<&'a dyn DynUnsafeKey>> for DynKey<GorcRef<'a, dyn DynUnsafeKey>> {
    fn borrow(&self) -> &DynKey<&'a dyn DynUnsafeKey> {
        // SAFETY: GorcRef<T> is the same as &T.
        unsafe { std::mem::transmute(self) }
    }
}

impl<K, U: ?Sized> PartialEq for DynKey<K>
where
    K: Deref<Target = U>,
    U: DynUnsafeEq,
{
    fn eq(&self, other: &Self) -> bool {
        if self.tid != other.tid {
            false
        } else {
            let other_ptr = std::ptr::from_ref(&other.key);
            // SAFETY: It is ensured that `tid` corresponds to the type if of the underlying
            // object hence when tids are equal so are the types.
            unsafe { self.key.eq(other_ptr.cast()) }
        }
    }
}

impl<K, U: ?Sized> Eq for DynKey<K>
where
    K: Deref<Target = U>,
    U: DynUnsafeEq,
{
}

impl<K, U: ?Sized> Hash for DynKey<K>
where
    K: Deref<Target = U>,
    U: DynHash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tid.hash(state);
        self.key.dyn_hash(state);
    }
}
