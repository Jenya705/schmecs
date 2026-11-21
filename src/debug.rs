use std::borrow::Borrow;
#[cfg(not(debug_assertions))]
use std::marker::PhantomData;

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Copy)]
pub struct DebugVariable<T>(T);

#[cfg(not(debug_assertions))]
#[derive(Debug, Clone, Copy)]
pub struct DebugVariable<T>(PhantomData<T>);

impl<T> DebugVariable<T> {
    #[inline(always)]
    pub const fn new(val: T) -> Self {
        #[cfg(debug_assertions)]
        {
            Self(val)
        }
        #[cfg(not(debug_assertions))]
        {
            Self(PhantomData)
        }
    }

    #[inline(always)]
    pub fn assert(&self, predicate: impl FnOnce(&T) -> bool) {
        #[cfg(debug_assertions)]
        {
            assert!(predicate(&self.0));
        }
    }

    #[inline(always)]
    pub fn assert_eq(&self, val: impl Borrow<T>)
    where
        T: PartialEq + std::fmt::Debug,
    {
        #[cfg(debug_assertions)]
        {
            assert_eq!(&self.0, val.borrow());
        }
    }

    #[inline(always)]
    pub fn within_range(&self, val: impl Borrow<T>)
    where
        T: From<u8> + PartialOrd + Clone,
    {
        #[cfg(debug_assertions)]
        {
            assert!(((Into::<T>::into(0u8))..self.0.clone()).contains(val.borrow()))
        }
    }

    #[inline(always)]
    pub fn map<U>(self, func: impl FnOnce(T) -> U) -> DebugVariable<U> {
        #[cfg(debug_assertions)]
        {
            DebugVariable::new(func(self.0))
        }
        #[cfg(not(debug_assertions))]
        {
            DebugVariable(PhantomData)
        }
    }

    #[inline(always)]
    pub fn mutate(&mut self, func: impl FnOnce(&mut T)) {
        #[cfg(debug_assertions)]
        {
            func(&mut self.0)
        }
    }

    pub fn inspect(&self, func: impl FnOnce(&T)) {
        #[cfg(debug_assertions)]
        {
            func(&self.0)
        }
    }

    #[inline(always)]
    pub const fn as_ref(&self) -> DebugVariable<&T> {
        #[cfg(debug_assertions)]
        {
            DebugVariable::new(&self.0)
        }
        #[cfg(not(debug_assertions))]
        {
            DebugVariable(PhantomData)
        }
    }

    #[allow(unused_variables)]
    pub fn unwrap_or(self, val: T) -> T {
        #[cfg(debug_assertions)]
        {
            self.0
        }
        #[cfg(not(debug_assertions))]
        {
            val
        }
    }
}

pub trait DebugCheckedUnwrap {
    type Result;

    /// # Safety
    /// See the corresponding version of `unwrap_unchecked`.
    ///
    /// For [`Option`]: `self` must be [`Option::Some`].
    /// For [`Result`]: `self` must be [`Result::Ok`].
    unsafe fn debug_unwrap_unchecked(self) -> Self::Result;
}

impl<T> DebugCheckedUnwrap for Option<T> {
    type Result = T;

    #[inline(always)]
    unsafe fn debug_unwrap_unchecked(self) -> Self::Result {
        #[cfg(debug_assertions)]
        {
            self.unwrap()
        }
        #[cfg(not(debug_assertions))]
        {
            // SAFETY: The same contract.
            self.unwrap_unchecked()
        }
    }
}

impl<T, E> DebugCheckedUnwrap for Result<T, E> {
    type Result = T;

    #[inline(always)]
    unsafe fn debug_unwrap_unchecked(self) -> Self::Result {
        // SAFETY: The same contract.
        unsafe { self.ok().debug_unwrap_unchecked() }
    }
}

/// # Safety
/// Reaching this function is UB. See [`std::hint::unreachable_unchecked`].
#[inline(always)]
pub const unsafe fn debug_unreachable_unchecked() -> ! {
    #[cfg(debug_assertions)]
    {
        unreachable!();
    }
    #[cfg(not(debug_assertions))]
    {
        // SAFETY: The same contract.
        std::hint::unreachable_unchecked();
    }
}
