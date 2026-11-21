use std::marker::PhantomData;

use crate::{option::PureOption, smart::BoxPtr};

macro_rules! sparse_set_impl {
    (immutable) => {
        pub fn get(&self, key: K) -> Option<&V> {
            self.values
                .get(key.into())
                .map(PureOption::as_ref)
                .flatten()
        }

        pub fn get_inner(&self, key: K) -> Option<&C> {
            self.values.get(key.into())
        }

        pub fn last(&self) -> Option<&V> {
            self.values
                .iter()
                .rev()
                .find(|o| o.is_some())
                .map(PureOption::as_ref)
                .map(Option::unwrap)
        }
    };
    (mutable) => {
        pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
            self.values
                .get_mut(key.into())
                .map(PureOption::as_mut)
                .flatten()
        }

        pub fn get_inner_mut(&mut self, key: K) -> Option<&mut C> {
            self.values.get_mut(key.into())
        }

        pub fn insert(&mut self, key: K, value: V) -> C {
            let key = key.into();
            match self.values.get_mut(key) {
                Some(stored) => stored.replace(value),
                None => {
                    self.values.resize_with(key + 1, || C::none());
                    self.values[key] = C::some(value);
                    C::none()
                }
            }
        }

        /// # Safety
        /// `key` must be within the bounds of current capacity.
        pub unsafe fn insert_within_capacity(&mut self, key: K, value: V) -> Option<V> {
            let key = key.into();
            std::mem::replace(
                // SAFETY: The caller's contract.
                unsafe { self.values.get_unchecked_mut(key) },
                C::some(value),
            )
            .into()
        }

        pub fn get_or_insert_mut<'a>(
            &'a mut self,
            key: K,
            or_insert: impl FnOnce() -> V,
        ) -> &'a mut V {
            let key = key.into();
            match self.values.get_mut(key) {
                Some(it) if it.is_none() => {
                    *it = C::some(or_insert());
                }
                None => {
                    self.values.resize_with(key + 1, || C::none());
                    self.values[key] = C::some(or_insert());
                }
                _ => {}
            }
            // SAFETY: We made sure that some value is stored under key.
            unsafe {
                self.values
                    .get_unchecked_mut(key)
                    .as_mut()
                    .unwrap_unchecked()
            }
        }

        pub fn take(&mut self, key: K) -> C {
            let key = key.into();
            match self.values.get_mut(key) {
                Some(v) => v.take(),
                None => C::none(),
            }
        }

        sparse_set_impl!(immutable);
    };
}

pub struct SparseSet<K, V, C = Option<V>> {
    values: Vec<C>,
    _marker: PhantomData<fn() -> (K, V)>,
}

impl<K, V, C> SparseSet<K, V, C>
where
    K: Into<usize>,
    C: PureOption<V = V>,
{
    pub const fn new() -> Self {
        Self {
            values: vec![],
            _marker: PhantomData,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            values: Vec::from_iter(std::iter::repeat_with(|| C::none()).take(cap)),
            _marker: PhantomData,
        }
    }

    sparse_set_impl!(mutable);
}

pub struct ImmutableSparseSet<K, V, C = Option<V>> {
    values: Box<[C]>,
    _marker: PhantomData<fn() -> (K, V)>,
}

impl<K, V, C> ImmutableSparseSet<K, V, C>
where
    K: Into<usize>,
    C: PureOption<V = V>,
{
    sparse_set_impl!(immutable);
}

impl<K, V> From<Box<[Option<V>]>> for ImmutableSparseSet<K, V> {
    fn from(value: Box<[Option<V>]>) -> Self {
        Self {
            values: value,
            _marker: PhantomData,
        }
    }
}

impl<K, V> From<SparseSet<K, V>> for ImmutableSparseSet<K, V> {
    fn from(value: SparseSet<K, V>) -> Self {
        Self {
            values: value.values.into_boxed_slice(),
            _marker: PhantomData,
        }
    }
}

pub struct ImmutableSparseSetPtr<K, V, C = Option<V>> {
    values: BoxPtr<[C]>,
    _marker: PhantomData<fn() -> (K, V)>,
}

impl<K, V, C> ImmutableSparseSetPtr<K, V, C>
where
    K: Into<usize>,
    C: PureOption<V = V>,
{
    /// # Safety
    /// - The [`ImmutableSparseSet`] which was used to create this [`ImmutableSparseSetPtr`] may not
    /// already be dropped.
    /// - No mutable aliases must exist to the object under key `key`.
    pub unsafe fn get(&self, key: K) -> Option<&V> {
        // SAFETY: The caller's contract
        unsafe { self.values.get() }
            .get(key.into())
            .map(PureOption::as_ref)
            .flatten()
    }

    pub fn new(set: &ImmutableSparseSet<K, V, C>) -> Self {
        Self {
            values: BoxPtr::new(&set.values),
            _marker: PhantomData,
        }
    }
}
