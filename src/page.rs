use std::marker::PhantomData;

use crossbeam_utils::CachePadded;

pub struct PageSparseSet<K, V, const ELEMENTS: usize, const ELEMENTS_LOG2: usize> {
    pages: Vec<(usize, Option<Box<CachePadded<[Option<V>; ELEMENTS]>>>)>,
    _marker: PhantomData<fn() -> K>,
}

impl<K, V, const ELEMENTS: usize, const ELEMENTS_LOG2: usize>
    PageSparseSet<K, V, ELEMENTS, ELEMENTS_LOG2>
where
    K: Into<usize>,
{
    pub fn new() -> Self {
        Self {
            pages: vec![],
            _marker: PhantomData,
        }
    }

    #[inline(always)]
    const fn outer_inner(key: usize) -> (usize, usize) {
        (key >> ELEMENTS_LOG2, key & (ELEMENTS - 1))
    }

    pub fn get(&self, key: K) -> Option<&V> {
        let (outer, inner) = Self::outer_inner(key.into());
        // SAFETY: see below.
        unsafe {
            self.pages
                .get(outer)?
                .1
                .as_ref()?
                // SAFETY: each page has ELEMENTS elements, inner is within the bounds.
                .get_unchecked(inner)
                .as_ref()
        }
    }

    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        let (outer, inner) = Self::outer_inner(key.into());
        // SAFETY: see below.
        unsafe {
            self.pages
                .get_mut(outer)?
                .1
                .as_mut()?
                // SAFETY: each page has ELEMENTS elements, inner is within the bounds.
                .get_unchecked_mut(inner)
                .as_mut()
        }
    }

    pub fn remove(&mut self, key: K) -> Option<V> {
        let (outer, inner) = Self::outer_inner(key.into());

        let (page_len, page) = self.pages.get_mut(outer)?;

        let page_unwrapped = page.as_mut()?;

        // SAFETY: each page has ELEMENTS elements, inner is within the bounds.
        let element = unsafe { page_unwrapped.get_unchecked_mut(inner) };

        if element.is_some() {
            let element_owned = std::mem::replace(element, None);
            *page_len -= 1;
            if *page_len == 0 {
                *page = None;
            }
            element_owned
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let (outer, inner) = Self::outer_inner(key.into());

        if outer >= self.pages.len() {
            self.pages.resize_with(outer + 1, || (0, None));
        };

        let (page_len, page) = &mut self.pages[outer];

        if page.is_none() {
            *page = Some(Box::new(CachePadded::new([const { None }; ELEMENTS])));
        }

        let page = page.as_mut().unwrap();

        let res = std::mem::replace(
            // SAFETY:
            // - It is ensured that pages has at least outer + 1 elements.
            // - inner is within the bounds
            unsafe { page.get_unchecked_mut(inner) },
            Some(value),
        );

        if res.is_none() {
            *page_len += 1;
        }

        res
    }
}
