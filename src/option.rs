/// # Safety
/// TL;DR: Methods listed in this trait must work logically exactly as in [`Option`].
pub unsafe trait PureOption {
    type V;

    fn some(val: Self::V) -> Self;

    fn none() -> Self;

    fn from(val: Option<Self::V>) -> Self;

    fn is_some(&self) -> bool;

    fn is_none(&self) -> bool {
        !self.is_some()
    }

    fn as_ref(&self) -> Option<&Self::V>;

    fn as_mut(&mut self) -> Option<&mut Self::V>;

    fn into(self) -> Option<Self::V>;

    fn replace(&mut self, value: Self::V) -> Self;

    fn take(&mut self) -> Self;
}

unsafe impl<T> PureOption for Option<T> {
    type V = T;

    fn some(val: Self::V) -> Self {
        Self::Some(val)
    }

    fn none() -> Self {
        Self::None
    }

    fn from(val: Option<Self::V>) -> Self {
        val
    }

    fn is_some(&self) -> bool {
        self.is_some()
    }

    fn as_ref(&self) -> Option<&Self::V> {
        self.as_ref()
    }

    fn as_mut(&mut self) -> Option<&mut Self::V> {
        self.as_mut()
    }

    fn into(self) -> Option<Self::V> {
        self
    }

    fn replace(&mut self, value: Self::V) -> Self {
        self.replace(value)
    }

    fn take(&mut self) -> Self {
        self.take()
    }
}
