use std::{marker::PhantomData, ptr::NonNull};

pub struct BoxPtr<T: ?Sized> {
    ptr: NonNull<T>,
    _marker: PhantomData<T>,
}

impl<T: ?Sized> BoxPtr<T> {
    /// # Safety
    /// - The [`Box`] used to create this object may not be dropped.
    /// - There must be no mutable alias to the underlying object.
    pub unsafe fn get(&self) -> &T {
        // SAFETY: the caller's contract.
        unsafe { self.ptr.as_ref() }
    }

    pub fn new(val: &Box<T>) -> Self {
        Self {
            ptr: NonNull::from_ref(val.as_ref()),
            _marker: PhantomData,
        }
    }
}

// SAFETY: BoxPtr<T> acts like Box<T>
unsafe impl<T: ?Sized> Sync for BoxPtr<T> where Box<T>: Sync {}
// SAFETY: BoxPtr<T> acts like Box<T>
unsafe impl<T: ?Sized> Send for BoxPtr<T> where Box<T>: Send {}
