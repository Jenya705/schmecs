use crate::bitset::PrimitiveBitSet;

pub struct SortedCombinedIter<'a, I> {
    iters: &'a mut [I],
}

impl<'a, I> SortedCombinedIter<'a, I> {
    pub fn new(iters: &'a mut [I]) -> Self {
        assert!(iters.len() <= 32);
        Self { iters }
    }
}

impl<'a, T, I> Iterator for SortedCombinedIter<'a, I>
where
    I: Iterator<Item = T> + Clone,
    T: PartialOrd + Ord,
{
    type Item = (T, PrimitiveBitSet<u32>);

    fn next(&mut self) -> Option<Self::Item> {
        let mut bitset = PrimitiveBitSet::<u32>::empty();

        let mut min_value = None;

        for (index, iter) in self.iters.iter().enumerate() {
            let index = index as u32;
            let value = iter.clone().next();
            match (&mut min_value, value) {
                (None, None) => {}
                (None, value @ Some(_)) => {
                    bitset.set(index);
                    min_value = value;
                }
                (Some(cur), Some(this)) if this.lt(cur) => {
                    bitset.clear();
                    bitset.set(index);
                    *cur = this;
                }
                (Some(cur), Some(this)) if this.eq(cur) => {
                    bitset.set(index);
                }
                (Some(_), Some(_) | None) => {}
            }
        }

        if min_value.is_some() {
            for iter in self.iters.iter_mut() {
                if iter.clone().next() == min_value {
                    iter.next();
                }
            }
        }

        min_value.map(|v| (v, bitset))
    }
}

pub trait IteratorExt: Iterator {
    fn next_if(&mut self, func: impl FnOnce(&Self::Item) -> bool) -> Option<Self::Item>
    where
        Self: Sized,
        Self: Clone;
}

impl<I> IteratorExt for I
where
    I: Iterator,
{
    fn next_if(&mut self, func: impl FnOnce(&Self::Item) -> bool) -> Option<Self::Item>
    where
        Self: Sized,
        Self: Clone,
    {
        match self.clone().next() {
            Some(v) if func(&v) => {
                self.next();
                Some(v)
            }
            Some(_) | None => None,
        }
    }
}
