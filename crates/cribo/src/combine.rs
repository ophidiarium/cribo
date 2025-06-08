use indexmap::IndexSet;
use std::path::PathBuf;

pub trait Combine {
    /// Combine two values, preferring the values in `self`.
    ///
    /// The logic follows that of Cargo's `config.toml`:
    ///
    /// > If a key is specified in multiple config files, the values will get merged together.
    /// > Numbers, strings, and booleans will use the value in the deeper config directory taking
    /// > precedence over ancestor directories, where the home directory is the lowest priority.
    /// > Arrays will be joined together with higher precedence items being placed later in the
    /// > merged array.
    ///
    /// ...with one exception: we place items with higher precedence earlier in the merged array.
    #[must_use]
    fn combine(self, other: Self) -> Self;
}

macro_rules! impl_combine_or {
    ($name:ty) => {
        impl Combine for Option<$name> {
            fn combine(self, other: Option<$name>) -> Option<$name> {
                self.or(other)
            }
        }
    };
}

impl_combine_or!(String);
impl_combine_or!(bool);
impl_combine_or!(PathBuf);

impl<T> Combine for Option<Vec<T>> {
    /// Combine two vectors by extending the higher precedence vector (`self`) with the lower
    /// precedence vector (`other`), placing higher precedence items first.
    fn combine(self, other: Option<Vec<T>>) -> Option<Vec<T>> {
        match (self, other) {
            (Some(mut a), Some(b)) => {
                a.extend(b);
                Some(a)
            }
            (a, b) => a.or(b),
        }
    }
}

impl<T> Combine for Option<IndexSet<T>>
where
    T: Eq + std::hash::Hash,
{
    /// Combine two IndexSets by extending the set in `self` with the set in `other`, if they're
    /// both `Some`.
    fn combine(self, other: Option<IndexSet<T>>) -> Option<IndexSet<T>> {
        match (self, other) {
            (Some(mut a), Some(b)) => {
                a.extend(b);
                Some(a)
            }
            (a, b) => a.or(b),
        }
    }
}

impl Combine for serde::de::IgnoredAny {
    fn combine(self, _other: Self) -> Self {
        self
    }
}

impl Combine for Option<serde::de::IgnoredAny> {
    fn combine(self, _other: Self) -> Self {
        self
    }
}
