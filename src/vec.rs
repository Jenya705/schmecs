use std::{
    alloc::Layout,
    marker::PhantomData,
    ptr::NonNull,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

use crate::{
    AbortOnDrop,
    math::{PowerOfTwo, USize},
};

mod __hidden {
    pub trait Sealed {}
}

pub unsafe trait AtomicVecInitializationStrategy: __hidden::Sealed {
    type T;

    const DROPS: bool;

    const INITIALIZED: bool;

    /// # Safety
    /// - `layout` must have non-zero size.
    /// - `layout`'s size must correspond to an array of `bucket_size` elements of type [`Self::T`].
    unsafe fn alloc(layout: Layout, bucket_size: usize) -> *mut u8;

    /// # Safety
    /// - `ptr` must have been allocated via [`Self::alloc`].
    /// - `layout` must be the same layout which was used to allocate `ptr`.
    /// - `layout`'s size must correspond to an array of `bucket_size` elements of type [`Self::T`].
    unsafe fn dealloc(ptr: *mut u8, layout: Layout, bucket_size: usize);
}

macro_rules! initialization_strategy {
    ($name: ident) => {
        pub struct $name<T>(std::marker::PhantomData<fn() -> T>);
        impl<T> __hidden::Sealed for $name<T> {}
    };
}

initialization_strategy!(AtomicVecUninitInitializationStrategy);

unsafe impl<T> AtomicVecInitializationStrategy for AtomicVecUninitInitializationStrategy<T> {
    type T = T;

    const DROPS: bool = false;

    const INITIALIZED: bool = false;

    unsafe fn alloc(layout: Layout, _bucket_size: usize) -> *mut u8 {
        // SAFETY: The caller ensures that `layout` has a non-zero size.
        unsafe { std::alloc::alloc(layout) }
    }

    unsafe fn dealloc(ptr: *mut u8, layout: Layout, _bucket_size: usize) {
        // SAFETY: The caller ensures that `layout` is the same one used to allocate `ptr` as well
        // as `ptr` was allocated via [`Self::alloc`] (i.e., the same allocator in this case).
        unsafe { std::alloc::dealloc(ptr, layout) }
    }
}

initialization_strategy!(AtomicVecDefaultInitializationStrategy);

unsafe impl<T> AtomicVecInitializationStrategy for AtomicVecDefaultInitializationStrategy<T>
where
    T: Default,
{
    type T = T;

    const DROPS: bool = true;

    const INITIALIZED: bool = true;

    unsafe fn alloc(layout: Layout, bucket_size: usize) -> *mut u8 {
        // SAFETY: The caller's contract ensures that `layout` has a non-zero size.
        let ptr = unsafe { std::alloc::alloc(layout) };
        let mut ptr_t = ptr.cast::<T>();
        for _ in 0..bucket_size {
            // SAFETY: The caller's contract ensures that ptr corresponds to an array of
            // `bucket_size` elements
            unsafe {
                ptr_t.write(T::default());
                ptr_t = ptr_t.add(1)
            };
        }
        ptr
    }

    unsafe fn dealloc(ptr: *mut u8, layout: Layout, bucket_size: usize) {
        if std::mem::needs_drop::<T>() {
            let mut ptr_t = ptr.cast::<T>();
            for _ in 0..bucket_size {
                // SAFETY: The caller ensures that ptr corresponds to an array of `bucket_size`
                // elements.
                unsafe {
                    std::ptr::drop_in_place(ptr_t);
                    ptr_t = ptr_t.add(1);
                }
            }
        }
        // SAFETY: The caller ensures that `layout` is the same one used to allocate `ptr` as well
        // as `ptr` was allocated via [`Self::alloc`] (i.e., the same allocator in this case)
        unsafe { std::alloc::dealloc(ptr, layout) };
    }
}

pub type PushOnlyUninitAtomicVec<T, const BUCKETS: usize = 32, const FIRST_BUCKET_SIZE: usize = 8> =
    PushOnlyAtomicVec<T, AtomicVecUninitInitializationStrategy<T>, BUCKETS, FIRST_BUCKET_SIZE>;

pub type PushOnlyDefaultAtomicVec<
    T,
    const BUCKETS: usize = 32,
    const FIRST_BUCKET_SIZE: usize = 8,
> = PushOnlyAtomicVec<T, AtomicVecDefaultInitializationStrategy<T>, BUCKETS, FIRST_BUCKET_SIZE>;

/// Supports the following atomic operations:
/// - push(element)
/// - get(index)
pub struct PushOnlyAtomicVec<T, I, const BUCKETS: usize = 32, const FIRST_BUCKET_SIZE: usize = 8>
where
    I: AtomicVecInitializationStrategy<T = T>,
    USize<FIRST_BUCKET_SIZE>: PowerOfTwo,
{
    buckets: Box<[AtomicPtr<T>; BUCKETS]>,
    len: AtomicUsize,
    _marker: PhantomData<fn() -> I>,
}

impl<T, I, const BUCKETS: usize, const FIRST_BUCKET_SIZE: usize> Drop
    for PushOnlyAtomicVec<T, I, BUCKETS, FIRST_BUCKET_SIZE>
where
    I: AtomicVecInitializationStrategy<T = T>,
    USize<FIRST_BUCKET_SIZE>: PowerOfTwo,
{
    fn drop(&mut self) {
        if !I::DROPS && std::mem::needs_drop::<T>() {
            for i in 0..*self.len.get_mut() {
                // SAFETY: i is within the bounds.
                unsafe { std::ptr::drop_in_place(self.get_unchecked_ptr(i).as_ptr()) };
            }
        }
        for (i, bucket) in self.buckets.iter_mut().enumerate() {
            let ptr = *bucket.get_mut();
            if ptr.is_null() {
                break;
            }
            // SAFETY:
            // - all pointers are allocated via I.
            // - bucket layouts and bucket sizes are also calculated via these function which are
            // pure hence they must be equal.
            unsafe { I::dealloc(ptr.cast(), Self::bucket_layout(i), Self::bucket_size(i)) };
        }
    }
}

#[inline(always)]
const fn bucket_size(first_bucket_size: usize, bucket: usize) -> usize {
    first_bucket_size * (1 << bucket)
}

#[inline(always)]
const fn bucket_index(first_bucket_size: usize, index: usize) -> (usize, usize) {
    let pos = index + first_bucket_size;
    let hi_bit = pos.ilog2() as usize;
    let bucket = hi_bit - (first_bucket_size.ilog2() as usize);
    let index = pos ^ (1usize << hi_bit);
    (bucket, index)
}

impl<T, const BUCKETS: usize, const FIRST_BUCKET_SIZE: usize>
    PushOnlyAtomicVec<T, AtomicVecDefaultInitializationStrategy<T>, BUCKETS, FIRST_BUCKET_SIZE>
where
    T: Default,
    USize<FIRST_BUCKET_SIZE>: PowerOfTwo,
{
    pub fn get_ptr(&self, index: usize) -> Option<NonNull<T>> {
        if self.load_len() <= index {
            None
        } else {
            Some(unsafe { self.get_unchecked_ptr(index) })
        }
    }

    /// # Safety
    /// - There may not exist any mutable aliases to the same element (i.e., the element with the
    /// same index).
    pub unsafe fn get(&self, index: usize) -> Option<&T> {
        // SAFETY: The caller's contract.
        self.get_ptr(index).map(|v| unsafe { v.as_ref() })
    }

    /// # Safety
    /// - There may not exist any mutable and immutable aliases to the same element (i.e., the element with the
    /// same index).
    pub unsafe fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        // SAFETY: The caller's contract.
        self.get_ptr(index).map(|mut v| unsafe { v.as_mut() })
    }
}

impl<T, I, const BUCKETS: usize, const FIRST_BUCKET_SIZE: usize>
    PushOnlyAtomicVec<T, I, BUCKETS, FIRST_BUCKET_SIZE>
where
    I: AtomicVecInitializationStrategy<T = T>,
    USize<FIRST_BUCKET_SIZE>: PowerOfTwo,
{
    fn bucket_layout(bucket: usize) -> Layout {
        Layout::new::<T>()
            .repeat(Self::bucket_size(bucket))
            .unwrap()
            .0
    }

    const fn bucket_index(index: usize) -> (usize, usize) {
        bucket_index(FIRST_BUCKET_SIZE, index)
    }

    const fn bucket_size(bucket: usize) -> usize {
        bucket_size(FIRST_BUCKET_SIZE, bucket)
    }

    pub fn new() -> Self {
        Self {
            buckets: Box::new([const { AtomicPtr::new(std::ptr::null_mut()) }; BUCKETS]),
            len: AtomicUsize::new(0),
            _marker: PhantomData,
        }
    }

    pub fn from_iter(iter: impl Iterator<Item = T>) -> Self {
        let res = Self::new();
        for v in iter {
            res.push(v);
        }
        res
    }

    /// # Abort
    /// This method will abort the program if `func` panics and
    /// [`AtomicVecInitializationStrategy::INITIALIZED`] is false.
    pub fn push_deferred(&self, func: impl FnOnce(usize) -> T) -> usize {
        let guard = (!I::INITIALIZED).then(|| {
            AbortOnDrop("It is unsafe to panic when trying to push a value into an atomic vector with an unitializing initialization strategy.")
        });

        // SAFETY: guard ensures that.
        let id = unsafe { self.reserve() };
        let val = func(id);
        // SAFETY: `id` was returned from `reserve` and wasn't initialized beforehand.
        unsafe { self.insert_unchecked(id, val) };
        std::mem::forget(guard);

        id
    }

    /// # Abort
    /// This method will abort the program if `func` panics and
    /// [`AtomicVecInitializationStrategy::INITIALIZED`] is false.
    pub fn push_many_deferred(&self, mut func: impl FnMut(usize) -> T, count: usize) -> usize {
        let guard = (!I::INITIALIZED).then(|| {
            AbortOnDrop("It is unsafe to panic when trying to push a value into an atomic vector with an unitializing initialization strategy.")
        });

        // SAFETY: guard ensures that.
        let start = unsafe { self.reserve_many(count) };

        for i in start..start + count {
            unsafe { self.insert_unchecked(i, func(i)) };
        }

        std::mem::forget(guard);

        start
    }

    /// # Safety
    /// If [`AtomicVecInitializationStrategy::DROPS`] is false then
    /// it must be ensured that this vec cannot be dropped before the element with the returned
    /// index was initialized.
    pub unsafe fn reserve(&self) -> usize {
        // Relaxed is used for len here because the only important thing is that these indicies are
        // contiguous for each other and they do not rely on anything else because it can only
        // increase.
        self.len.fetch_add(1, Ordering::Relaxed)
    }

    /// # Safety
    /// If [`AtomicVecInitializationStrategy::DROPS`] is false then
    /// it must be ensured that this vec cannot be dropped before all the elements in the range from
    /// the returned index upto `count` elements were initialized.
    pub unsafe fn reserve_many(&self, count: usize) -> usize {
        self.len.fetch_add(count, Ordering::Relaxed)
    }

    pub fn load_len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    /// # Safety
    /// `index` must be returned from [`Self::reserve`] and not be initialized beforehand.
    pub unsafe fn insert_unchecked(&self, index: usize, value: T) {
        if const { std::mem::size_of::<T>() == 0 } {
            return;
        }
        let (outer, inner) = Self::bucket_index(index);
        let bucket_ptr = self.buckets.get(outer).unwrap();

        // We use Acquire, Release here because we need data behind the pointer (dependent data) to be
        // synchronized between the threads in the same time as buckets' pointers.
        let mut loaded_bucket_ptr = bucket_ptr.load(Ordering::Acquire);

        if loaded_bucket_ptr.is_null() {
            let bucket_size = Self::bucket_size(outer);
            let layout = Self::bucket_layout(outer);

            // SAFETY:
            // - Layout is not zero-sized because T is not ZST.
            // - `Self::bucket_size` was used to calculate `layout`.
            let allocated = unsafe { I::alloc(layout, bucket_size) };

            let res = bucket_ptr.compare_exchange(
                std::ptr::null_mut(),
                allocated.cast(),
                Ordering::Release,
                Ordering::Acquire,
            );

            if let Err(_) = res {
                // SAFETY:
                // - `allocated` was allocated via `I::alloc`.
                // - `layout` is the same layout passed to `I::alloc`.
                // - `Self::bucket_size` was used to calculate `layout`.
                unsafe { I::dealloc(allocated, layout, bucket_size) };
            }

            loaded_bucket_ptr = bucket_ptr.load(Ordering::Acquire);
        }

        // SAFETY: `bucket_index` always returns valid indicies for allocation, alignment
        // comes from the rust's guarantees.
        unsafe { loaded_bucket_ptr.add(inner).write(value) };
    }

    pub fn push(&self, value: T) -> usize {
        self.push_deferred(|_| value)
    }

    /// # Safety
    /// - `index` must be within the bounds if `T` is not ZST.
    pub unsafe fn get_unchecked_ptr(&self, index: usize) -> NonNull<T> {
        if const { std::mem::size_of::<T>() == 0 } {
            // SAFETY: T is ZST.
            return NonNull::<T>::dangling();
        }
        let (outer, inner) = Self::bucket_index(index);
        // SAFETY: `index` is valid.
        let bucket_ptr = unsafe { self.buckets.get_unchecked(outer) };
        let bucket_ptr = bucket_ptr.load(Ordering::Acquire);
        // SAFETY: `inner` is always less than the length of the allocation and the pointer is
        // not null.
        unsafe { NonNull::new_unchecked(bucket_ptr.add(inner)) }
    }

    /// # Safety
    /// - See [`Self::get_unchecked_ptr`]
    /// - There may not exist any mutable aliases to the same element (i.e., the element with the
    /// same index).
    /// - If [`AtomicVecInitializationStrategy::INITIALIZED`] is false
    /// then `index` must be initialized (i.e., it must be returned by [`Self::push`] at some point of time
    /// before using [`Self::get_unchecked`]) if `T` is not ZST.
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        // SAFETY:
        // - The same contract.
        // - It is safe to dereference this value as `index` is initialized as per the contract.
        unsafe { self.get_unchecked_ptr(index).as_ref() }
    }

    /// # Safety
    /// - See [`Self::get_unchecked_ptr`]
    /// - There may not exist any mutable and immutable aliases to the same element (i.e., the element with the
    /// same index).
    /// - If [`AtomicVecInitializationStrategy::INITIALIZED`] is false
    /// then `index` must be initialized (i.e., it must be returned by [`Self::push`] at some point of time
    /// before using [`Self::get_unchecked`]) if `T` is not ZST.
    pub unsafe fn get_unchecked_mut(&self, index: usize) -> &mut T {
        // SAFETY:
        // - The same contract.
        // - It is safe to dereference this value as `index` is initialized as per the contract.
        unsafe { self.get_unchecked_ptr(index).as_mut() }
    }
    ///
    /// # Safety
    /// - See [`Self::get_unchecked_ptr`]
    /// - There may not exist any mutable aliases to any of the returned elements.
    /// - If [`AtomicVecInitializationStrategy::INITIALIZED`] is false
    /// then all `indicies` must be initialized.
    pub unsafe fn get_disjoint_unchecked<const LEN: usize>(
        &self,
        indicies: [usize; LEN],
    ) -> [&T; LEN] {
        // SAFETY: The caller's contract.
        indicies.map(|i| unsafe { self.get_unchecked_ptr(i).as_ref() })
    }

    /// # Safety
    /// - See [`Self::get_unchecked_ptr`]
    /// - There may not exist any mutable and immutable aliases to any of the returned elements.
    /// - `indicies` must be unique.
    /// - If [`AtomicVecInitializationStrategy::INITIALIZED`] is false
    /// all `indicies` must be initialized.
    pub unsafe fn get_disjoint_unchecked_mut<const LEN: usize>(
        &self,
        indicies: [usize; LEN],
    ) -> [&mut T; LEN] {
        // SAFETY: The caller's contract.
        indicies.map(|i| unsafe { self.get_unchecked_ptr(i).as_mut() })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::vec::{AtomicVecUninitInitializationStrategy, PushOnlyAtomicVec};

    struct DroppedIndicator<'a, T>(T, &'a AtomicBool);

    impl<'a, T> Drop for DroppedIndicator<'a, T> {
        fn drop(&mut self) {
            self.1.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn push_only_atomic_vec_test() {
        let vec = PushOnlyAtomicVec::<
            DroppedIndicator<()>,
            AtomicVecUninitInitializationStrategy<DroppedIndicator<()>>,
        >::new();
        let atomics = [const { AtomicBool::new(false) }; if cfg!(miri) { 100 } else { 16000 }];
        for a in atomics.iter() {
            vec.push(DroppedIndicator((), a));
        }
        drop(vec);
        for a in atomics {
            assert_eq!(a.load(Ordering::SeqCst), true);
        }
    }
}

#[cfg(kani)]
mod proofs {
    use super::{bucket_index, bucket_size};

    #[kani::proof]
    fn push_only_atomic_vec_indicies_proof() {
        let first_bucket_size: usize = kani::any();
        kani::assume(first_bucket_size.is_power_of_two() && first_bucket_size <= 16);
        let index: usize = kani::any();
        kani::assume(index < (1 << 31));

        let (outer, inner) = bucket_index(first_bucket_size, index);

        assert!(inner < bucket_size(first_bucket_size, outer));

        let l_is_other_bucket = inner == 0;
        let r_is_other_bucket = inner == bucket_size(first_bucket_size, outer) - 1;

        if index != 0 {
            let lindex = index - 1;
            let (louter, linner) = bucket_index(first_bucket_size, lindex);
            if l_is_other_bucket {
                assert!(
                    louter == outer - 1 && linner == bucket_size(first_bucket_size, louter) - 1
                );
            } else {
                assert!(louter == outer && linner + 1 == inner);
            }
        }

        let rindex = index + 1;
        let (router, rinner) = bucket_index(first_bucket_size, rindex);
        if r_is_other_bucket {
            assert!(router == outer + 1 && rinner == 0);
        } else {
            assert!(router == outer && rinner == inner + 1);
        }
    }
}
