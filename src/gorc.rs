//! Why?
//!
//! 1. It eliminates Arc and Rc headers within the same allocation as the underlying object, in c++
//!    it is solved via storing header as another allocation which in our case would increase the
//!    size of the (wide) pointer by one usize.
//! 2. Despite not being Arc and Rc, references are still counted thus reducing possibility of
//!    memory leaks.
//! 3. It supports static references which cannot be deallocated and does so without any overhead
//!    except trying to drop or clone (needed for compile time data).
//!
//! Cloning and Dropping are essentially the only performance bottlenecks with this implementation, but it is
//! expected that these values won't be cloned or dropped a lot.
//!
//! You only want to use it in cases where it is not certain whether some data must actually be
//! dropped or not. If you can use generics to ensure right freeing, **DO SO**.

use std::{
    borrow::Borrow,
    cmp::Ordering,
    collections::BTreeMap,
    fmt::Debug,
    hash::Hash,
    marker::PhantomData,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{AtomicU32, Ordering::SeqCst},
};

use parking_lot::RwLock;

pub struct Gorc {
    // TODO: lockfree?
    counters: RwLock<BTreeMap<*const u8, AtomicU32>>,
}

// SAFETY: It is safe to move references of Gorc between threads
unsafe impl Sync for Gorc {}
// SAFETY: Gorc is thread safe
unsafe impl Send for Gorc {}

static GORC_GLOBAL: Gorc = Gorc::new();

impl Gorc {
    pub(crate) const fn new() -> Self {
        Self {
            counters: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn add<'a, T: ?Sized + 'a>(&self, val: Box<T>) -> GorcRef<'a, T> {
        let val = NonNull::from_mut(Box::leak(val));
        self.counters
            .write()
            .insert(val.cast().as_ptr(), AtomicU32::new(0));
        GorcRef::new(val)
    }

    pub(crate) fn inc(&self, ptr: *const u8) {
        let guard = self.counters.read();
        let Some(counter) = guard.get(&ptr) else {
            return;
        };
        counter.fetch_add(1, SeqCst);
    }

    /// # Safety
    /// - This function must be called only once for each [`Self::inc`] function called on this argument
    /// and an additional one time.
    /// - In case the pointer was registered via [`Self::add`], it had to be allocated via the
    /// global allocator.
    pub(crate) unsafe fn dec<T: ?Sized>(&self, ptr: *mut T) {
        let guard = self.counters.read();
        let ptr_u8 = ptr as *const u8;
        let Some(counter) = guard.get(&ptr_u8) else {
            return;
        };
        if counter.fetch_sub(1, SeqCst) == 0 {
            drop(guard);
            let mut guard = self.counters.write();
            guard.remove(&ptr_u8);
            drop(guard);
            // SAFETY: The caller's contract, no double free because we use SeqCst.
            drop(unsafe { Box::<T>::from_raw(ptr) });
        }
    }
}

#[repr(transparent)]
pub struct GorcRef<'a, T: ?Sized + 'a> {
    ptr: NonNull<T>,
    _marker: PhantomData<&'a T>,
}

impl<'a, 'b: 'a, T: ?Sized + 'b> Borrow<T> for GorcRef<'a, T> {
    fn borrow(&self) -> &T {
        self.as_ref()
    }
}

unsafe impl<'a, T: ?Sized + 'a> Sync for GorcRef<'a, T> where for<'b> &'b T: Sync {}
unsafe impl<'a, T: ?Sized + 'a> Send for GorcRef<'a, T> where for<'b> &'b T: Send {}

impl<'a, T: ?Sized> GorcRef<'a, T> {
    pub(crate) const fn new(ptr: NonNull<T>) -> Self {
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    pub fn from_box(val: Box<T>) -> Self {
        GORC_GLOBAL.add(val)
    }

    pub const fn from_ref(val: &'a T) -> Self {
        Self::new(NonNull::from_ref(val))
    }
}

impl<'a, T: ?Sized> From<Box<T>> for GorcRef<'a, T> {
    fn from(value: Box<T>) -> Self {
        Self::from_box(value)
    }
}

impl<'a, T: ?Sized> From<&'a T> for GorcRef<'a, T> {
    fn from(value: &'a T) -> Self {
        Self::from_ref(value)
    }
}

impl<'a, T: ?Sized> AsRef<T> for GorcRef<'a, T> {
    fn as_ref(&self) -> &T {
        // SAFETY: As long as GorcPtr is alive, the pointer must be also alive.
        unsafe { self.ptr.as_ref() }
    }
}

impl<'a, T: ?Sized> Deref for GorcRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<'a, T: ?Sized> Drop for GorcRef<'a, T> {
    fn drop(&mut self) {
        // SAFETY: GorcPtr can only be created as a result of Gorc::new or Cloning of another
        // GorcPtr, which would call Gorc::inc, thus Gorc::dec is only being called for each
        // Gorc::inc or Gorc::new.
        //
        // Gorc::new was only called in case the pointer was allocated via the global allocator
        // (i.e., instantiated from Box<T>).
        unsafe {
            GORC_GLOBAL.dec::<T>(self.ptr.as_ptr());
        }
    }
}

impl<'a, T: Debug> Debug for GorcRef<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("GorcRef").field(self.as_ref()).finish()
    }
}

impl<'a, T: ?Sized> Clone for GorcRef<'a, T> {
    fn clone(&self) -> Self {
        GORC_GLOBAL.inc(self.ptr.cast().as_ptr() as *const _);
        Self {
            ptr: self.ptr.clone(),
            _marker: self._marker.clone(),
        }
    }
}

impl<'a, T: ?Sized + Hash> Hash for GorcRef<'a, T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.ptr.hash(state);
    }
}

impl<'a, T: ?Sized + PartialEq> PartialEq for GorcRef<'a, T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl<'a, T: ?Sized + Eq> Eq for GorcRef<'a, T> {}

impl<'a, T: ?Sized + PartialOrd> PartialOrd for GorcRef<'a, T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // TODO: ref cmp
        self.as_ref().partial_cmp(other.as_ref())
    }
}

impl<'a, T: ?Sized + Ord> Ord for GorcRef<'a, T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(other)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering::SeqCst};

    use crate::gorc::GorcRef;

    struct A<'a>(pub &'a AtomicU64);

    impl<'a> Drop for A<'a> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, SeqCst);
        }
    }

    #[test]
    fn gorc_owned_drop_test() {
        let c = AtomicU64::new(1);
        let rc = GorcRef::from_box(Box::new(A(&c)));
        assert_eq!(c.load(SeqCst), 1);
        drop(rc);
        assert_eq!(c.load(SeqCst), 0);
    }

    #[test]
    fn gorc_ref_drop_test() {
        let c = AtomicU64::new(1);
        let a = A(&c);
        let rc = GorcRef::from_ref(&a);
        assert_eq!(c.load(SeqCst), 1);
        drop(rc);
        assert_eq!(c.load(SeqCst), 1);
        drop(a);
        assert_eq!(c.load(SeqCst), 0);
    }
}
