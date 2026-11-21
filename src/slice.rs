use std::{marker::PhantomData, ops::Deref};

use crate::pure::{ClonePure, DerefPure, OrdPure};

/// # Safety
/// After the creation of this [`SliceWrapper`], return value of [`SliceWrapper::slice`] must always
/// be the same.
pub unsafe trait SliceWrapper {
    type Element;

    /// It is safe to assume, that [`Self::check_preconditions`] holds true for the returned slice.
    fn slice(&self) -> &[Self::Element];

    fn check_preconditions(slice: &[Self::Element]) -> bool;
}

pub trait VecWrapper: SliceWrapper + Sized {
    type Inner: ?Sized + VecWrapper<Element = Self::Element>;

    fn from_vec(inner: Self::Inner) -> Option<Self> {
        Self::check_preconditions(inner.slice())
            // SAFETY: `Self::check_preconditions` holds true.
            .then(|| unsafe { Self::from_vec_unchecked(inner) })
    }

    /// # Safety
    /// [`Self::check_preconditions`](`SliceWrapper::check_preconditions`) must hold true.
    unsafe fn from_vec_unchecked(inner: Self::Inner) -> Self;
}

pub trait UnsizedArrayWrapper: SliceWrapper {
    type Inner: ?Sized + UnsizedArrayWrapper<Element = Self::Element>;

    fn new_ref(inner: &Self::Inner) -> Option<&Self> {
        Self::check_preconditions(inner.slice())
            // SAFETY: `Self::check_preconditions` holds true.
            .then(|| unsafe { Self::new_ref_unchecked(inner) })
    }

    /// # Safety
    /// [`Self::check_preconditions`](`SliceWrapper::check_preconditions`) must hold true.
    unsafe fn new_ref_unchecked(inner: &Self::Inner) -> &Self;
}

pub trait ConstArrayWrapper: SliceWrapper + Sized {
    type Inner: ConstArrayWrapper<Element = Self::Element>;

    fn new_const(inner: Self::Inner) -> Option<Self> {
        Self::check_preconditions(inner.slice())
            // SAFETY: `Self::check_preconditions` holds true.
            .then(|| unsafe { Self::new_const_unchecked(inner) })
    }

    /// # Safety
    /// [`Self::check_preconditions`](`SliceWrapper::check_preconditions`) must hold true.
    unsafe fn new_const_unchecked(inner: Self::Inner) -> Self;
}

// SAFETY: Rust enforces immutability of references.
unsafe impl<T> SliceWrapper for [T] {
    type Element = T;

    fn slice(&self) -> &[Self::Element] {
        &self
    }

    fn check_preconditions(_: &[Self::Element]) -> bool {
        true
    }
}

impl<T> UnsizedArrayWrapper for [T] {
    type Inner = [T];

    unsafe fn new_ref_unchecked(inner: &Self::Inner) -> &Self {
        &inner
    }
}

// SAFETY: Rust enforces immutability.
unsafe impl<T, const LEN: usize> SliceWrapper for [T; LEN] {
    type Element = T;

    fn slice(&self) -> &[Self::Element] {
        self.as_slice()
    }

    fn check_preconditions(_: &[Self::Element]) -> bool {
        true
    }
}

impl<T, const LEN: usize> ConstArrayWrapper for [T; LEN] {
    type Inner = [T; LEN];

    unsafe fn new_const_unchecked(inner: Self::Inner) -> Self {
        inner
    }
}

// SAFETY: Rust enforces immutability.
unsafe impl<T> SliceWrapper for Vec<T> {
    type Element = T;

    fn slice(&self) -> &[Self::Element] {
        self.as_slice()
    }

    fn check_preconditions(_: &[Self::Element]) -> bool {
        true
    }
}

macro_rules! precond {
    ($inner: expr, $debug_func: expr) => {
        debug_assert!(
            $debug_func($inner.slice()),
            "Unsafe precondition violated for {name}",
            name = std::any::type_name::<Self>(),
        );
    };
}

macro_rules! wrapper {
    ($name: ident<$($ty:ident),*> = $func: expr, $debug_func: expr) => {
        #[repr(transparent)]
        pub struct $name<T: ?Sized>(T);

        // SAFETY: Propagates to T's SliceWrapper safety contract, which says that the slice
        // returned is the same.
        unsafe impl<T: ?Sized> SliceWrapper for $name<T>
        where
            T: SliceWrapper,
            $(
                T::Element: $ty,
            )*
        {
            type Element = T::Element;

            fn slice(&self) -> &[Self::Element] {
                self.0.slice()
            }

            fn check_preconditions(slice: &[Self::Element]) -> bool {
                $func(slice)
            }
        }

        impl<T> VecWrapper for $name<T>
        where
            T: VecWrapper,
            $(T::Element: $ty,)*
        {
            type Inner = T;

            unsafe fn from_vec_unchecked(inner: Self::Inner) -> Self {
                precond!(inner, $debug_func);
                Self(inner)
            }
        }

        impl<T: ?Sized> UnsizedArrayWrapper for $name<T>
        where
            T: UnsizedArrayWrapper,
            $(T::Element: $ty,)*
        {
            type Inner = T;

            unsafe fn new_ref_unchecked(inner: &T) -> &Self {
                precond!(inner, $debug_func);
                // SAFETY: Self is transparent to T hence so are the references.
                unsafe {
                    std::mem::transmute(inner)
                }
            }
        }

        impl<T> ConstArrayWrapper for $name<T>
        where
            T: ConstArrayWrapper,
            $(T::Element: $ty,)*
        {
            type Inner = T;

            unsafe fn new_const_unchecked(inner: T) -> Self {
                precond!(inner, $debug_func);
                Self(inner)
            }
        }

        impl<T: ?Sized> Deref for $name<T> where T: SliceWrapper, {
            type Target = [T::Element];

            fn deref(&self) -> &Self::Target {
                self.0.slice()
            }
        }

        impl<T: ?Sized> AsRef<[T::Element]> for $name<T> where T: SliceWrapper, {
            fn as_ref(&self) -> &[T::Element] {
                self.0.slice()
            }
        }
    };
}

fn sorted_precondition<T>(slice: &[T]) -> bool
where
    T: PartialOrd + Ord,
{
    slice.is_sorted()
}

fn set_precondition<T>(slice: &[T]) -> bool
where
    T: PartialEq + Eq,
{
    for (i, v1) in slice.iter().enumerate() {
        for v2 in slice[i + 1..].iter() {
            if v1 == v2 {
                return false;
            }
        }
    }
    true
}

fn set_precondition_debug<T>(slice: &[T]) -> bool
where
    T: PartialEq + Eq,
{
    slice.len() > 500 || set_precondition(slice)
}

fn sorted_set_precondition<T>(slice: &[T]) -> bool
where
    T: PartialOrd + Ord,
{
    let mut iter = slice.iter();
    let Some(mut prev) = iter.next() else {
        return true;
    };
    while let Some(cur) = iter.next() {
        if prev >= cur {
            return false;
        }
        prev = cur;
    }
    true
}

wrapper!(Sorted<OrdPure> = sorted_precondition, sorted_precondition);
// wrapper!(Set<PartialEq, Eq> = set_precondition, set_precondition_debug);
wrapper!(SortedSet<OrdPure> = sorted_set_precondition, sorted_set_precondition);

#[repr(C)]
pub struct Partitioned<W: ?Sized, S, const ADDITIONAL: usize> {
    _marker: PhantomData<fn() -> W>,
    starts: [usize; ADDITIONAL],
    slice: S,
}

impl<W: ?Sized, S, const ADDITIONAL: usize> std::hash::Hash for Partitioned<W, S, ADDITIONAL>
where
    S: std::hash::Hash,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.starts.hash(state);
        self.slice.hash(state);
    }
}

impl<W: ?Sized, S, const ADDITIONAL: usize> PartialEq for Partitioned<W, S, ADDITIONAL>
where
    S: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.starts == other.starts && self.slice == other.slice
    }
}

impl<W: ?Sized, S, const ADDITIONAL: usize> Eq for Partitioned<W, S, ADDITIONAL> where S: Eq {}

impl<W: ?Sized, S, const ADDITIONAL: usize> Clone for Partitioned<W, S, ADDITIONAL>
where
    // Why using ClonePure instead of Clone? Using clone doesn't ensure that the values don't
    // change, changing of any value might break the safety contract which was used to create this
    // object, which itself might lead to an unsafe precondition being false, which might lead to an
    // UB.
    //
    // A lot of other ECS libraries (or generally libraries) deal with it by ensuring safety via
    // prechecks, hiding of the implementation (which is partially done here as well) or just generally
    // ignore this.
    [usize; ADDITIONAL]: ClonePure,
    S: ClonePure,
{
    fn clone(&self) -> Self {
        Self {
            _marker: self._marker.clone(),
            starts: self.starts.clone(),
            slice: self.slice.clone(),
        }
    }
}

impl<T, W: ?Sized, S, const ADDITIONAL: usize> Partitioned<W, S, ADDITIONAL>
where
    W: UnsizedArrayWrapper<Element = T, Inner = [T]>,
    S: DerefPure + Deref<Target = [T]>,
{
    #[inline(always)]
    fn __get_slice<'a>(
        starts: &[usize; ADDITIONAL],
        slice: &'a S,
        index: usize,
    ) -> &'a [W::Element] {
        let slice: &[T] = slice.as_ref();
        let l = if index == 0 { 0 } else { starts[index - 1] };
        // NOTE: If starts goes before slice and slice is &[Element],
        // (which is always true when Partitioned has #[repr(C)] and slice is &[Element])
        // then actually it is just starts[index].
        let r = if index == ADDITIONAL {
            slice.len()
        } else {
            starts[index]
        };
        &slice[l..r]
    }

    pub fn check_preconditions(starts: &[usize; ADDITIONAL], slice: &S) -> bool {
        for i in 0..ADDITIONAL + 1 {
            if !W::check_preconditions(Self::__get_slice(starts, slice, i)) {
                return false;
            }
        }
        true
    }

    pub fn new(starts: [usize; ADDITIONAL], slice: S) -> Option<Self> {
        Self::check_preconditions(&starts, &slice)
            // SAFETY: `Self::check_preconditions` holds true.
            .then(|| unsafe { Self::new_unchecked(starts, slice) })
    }

    /// # Safety
    /// [`Self::check_preconditions`] must hold true for these values.
    pub unsafe fn new_unchecked(starts: [usize; ADDITIONAL], slice: S) -> Self {
        #[cfg(debug_assertions)]
        {
            for i in 0..ADDITIONAL + 1 {
                // SAFETY: `Self::check_preconditions` holds true as per the contract
                // hence `W::check_preconditions` holds true for each subslice.
                let _ = unsafe { W::new_ref_unchecked(Self::__get_slice(&starts, &slice, i)) };
            }
        }
        Self {
            _marker: PhantomData,
            starts,
            slice,
        }
    }

    #[inline(always)]
    pub fn get_slice<'a>(&'a self, index: usize) -> &'a W
    where
        T: 'a,
    {
        // SAFETY: `Self::check_preconditions` holds true hence `W::check_preconditions` holds true
        // for each subslice.
        unsafe { W::new_ref_unchecked(Self::__get_slice(&self.starts, &self.slice, index)) }
    }

    pub fn get_whole(&self) -> &S {
        &self.slice
    }

    pub fn get_starts(&self) -> &[usize; ADDITIONAL] {
        &self.starts
    }

    pub fn into_parts(self) -> ([usize; ADDITIONAL], S) {
        (self.starts, self.slice)
    }
}

#[cfg(test)]
mod tests {
    use crate::slice::{
        ConstArrayWrapper, Partitioned, SliceWrapper, Sorted, SortedSet, UnsizedArrayWrapper,
    };

    #[test]
    fn sorted_test() {
        assert!(Sorted::new_ref([1, 2, 3].as_slice()).is_some());
        assert!(Sorted::new_ref([1, 2, 2].as_slice()).is_some());
        assert!(Sorted::new_ref([1, 3, 2].as_slice()).is_none());
        assert!(Sorted::new_const([1, 2, 3]).is_some());
        assert!(Sorted::new_const([1, 2, 2]).is_some());
        assert!(Sorted::new_const([1, 3, 2]).is_none());
    }

    // #[test]
    // fn set_test() {
    //     assert!(Set::new_ref([1, 2, 3].as_slice()).is_some());
    //     assert!(Set::new_ref([1, 2, 2].as_slice()).is_none());
    //     assert!(Set::new_ref([1, 3, 2].as_slice()).is_some());
    //     assert!(Set::new_const([1, 2, 3]).is_some());
    //     assert!(Set::new_const([1, 2, 2]).is_none());
    //     assert!(Set::new_const([1, 3, 2]).is_some());
    // }

    #[test]
    fn sorted_set_test() {
        assert!(SortedSet::new_ref([1, 2, 3].as_slice()).is_some());
        assert!(SortedSet::new_ref([1, 2, 2].as_slice()).is_none());
        assert!(SortedSet::new_ref([1, 3, 2].as_slice()).is_none());
        assert!(SortedSet::new_const([1, 2, 3]).is_some());
        assert!(SortedSet::new_const([1, 2, 2]).is_none());
        assert!(SortedSet::new_const([1, 3, 2]).is_none());
    }

    #[test]
    fn partitioned_test() {
        let p = Partitioned::<SortedSet<[i32]>, Box<[i32]>, 1>::new([2], Box::new([1, 2, 1, 3]))
            .unwrap();

        assert_eq!(p.get_slice(0).slice(), [1, 2]);
        assert_eq!(p.get_slice(1).slice(), [1, 3]);
    }
}
