//! Descriptor data that is either compiled in (`Static`) or built at runtime
//! (`Owned`), e.g. the settings a connector declares when it connects.

use std::{fmt, ops::Deref, sync::Arc};

/// A value that is either `'static` (a `const` declaration) or shared and
/// owned (a descriptor built at runtime). Both compare and read the same.
pub enum Shared<T: ?Sized + 'static> {
    Static(&'static T),
    Owned(Arc<T>),
}

impl<T: ?Sized> Shared<T> {
    /// Wraps a value built at runtime.
    pub fn new(value: impl Into<Arc<T>>) -> Self {
        Self::Owned(value.into())
    }
}

impl<T> Shared<T> {
    pub fn owned(value: T) -> Self {
        Self::Owned(Arc::new(value))
    }
}

impl<T: ?Sized> Clone for Shared<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Static(value) => Self::Static(value),
            Self::Owned(value) => Self::Owned(Arc::clone(value)),
        }
    }
}

impl<T: ?Sized> Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        match self {
            Self::Static(value) => value,
            Self::Owned(value) => value,
        }
    }
}

impl<T: ?Sized> AsRef<T> for Shared<T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: ?Sized + PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: ?Sized + Eq> Eq for Shared<T> {}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl From<&'static str> for Shared<str> {
    fn from(value: &'static str) -> Self {
        Self::Static(value)
    }
}

impl From<String> for Shared<str> {
    fn from(value: String) -> Self {
        Self::Owned(value.into())
    }
}

impl<T> From<Vec<T>> for Shared<[T]> {
    fn from(value: Vec<T>) -> Self {
        Self::Owned(value.into())
    }
}

impl<T> From<&'static [T]> for Shared<[T]> {
    fn from(value: &'static [T]) -> Self {
        Self::Static(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_static_and_owned_compare_by_value() {
        assert_eq!(Shared::Static("a"), Shared::<str>::from("a".to_string()));
        assert_ne!(Shared::Static("a"), Shared::<str>::from("b".to_string()));
        let numbers: Shared<[i64]> = vec![1, 2].into();
        assert_eq!(numbers, Shared::Static(&[1, 2][..]));
        assert_eq!(format!("{:?}", Shared::Static("a")), "\"a\"");
    }
}
