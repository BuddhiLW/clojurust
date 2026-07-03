use crate::Value;

/// An immutable persistent vector backed by `rpds::Vector`.
///
/// A vector may additionally be tagged as a *map entry* (see
/// [`PersistentVector::map_entry`]): a two-element `[key val]` pair produced
/// by seq'ing a map, `find`, or the `map-entry` builtin. Map entries behave
/// exactly like vectors (equality, hashing, printing, indexing) — the tag
/// only answers `map-entry?` — and, as in Clojure, any derived vector
/// (`conj`, `assoc`, `pop`, ...) is a plain vector again.
#[derive(Debug, Clone)]
pub struct PersistentVector {
    inner: rpds::VectorSync<Value>,
    is_map_entry: bool,
}

impl PersistentVector {
    pub fn empty() -> Self {
        Self {
            inner: rpds::VectorSync::new_sync(),
            is_map_entry: false,
        }
    }

    pub fn from_vector(vector: rpds::VectorSync<Value>) -> Self {
        Self {
            inner: vector,
            is_map_entry: false,
        }
    }

    /// Build a `[key val]` pair tagged as a map entry.
    pub fn map_entry(key: Value, val: Value) -> Self {
        let mut inner = rpds::VectorSync::new_sync();
        inner = inner.push_back(key);
        inner = inner.push_back(val);
        Self {
            inner,
            is_map_entry: true,
        }
    }

    /// True only for vectors created via [`PersistentVector::map_entry`].
    pub fn is_map_entry(&self) -> bool {
        self.is_map_entry
    }

    pub fn count(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Append a value. O(log n) amortized.
    pub fn conj(&self, val: Value) -> Self {
        Self {
            inner: self.inner.push_back(val),
            is_map_entry: false,
        }
    }

    /// Return the element at `idx`, or `None` if out of bounds.
    pub fn nth(&self, idx: usize) -> Option<&Value> {
        self.inner.get(idx)
    }

    /// Last element.
    pub fn peek(&self) -> Option<&Value> {
        self.inner.last()
    }

    /// Return a new vector with element `idx` replaced, or appended if `idx == len`.
    pub fn assoc_nth(&self, idx: usize, val: Value) -> Option<Self> {
        if idx == self.inner.len() {
            Some(self.conj(val))
        } else {
            Some(Self {
                inner: self.inner.set(idx, val)?,
                is_map_entry: false,
            })
        }
    }

    /// Remove the last element. Returns `None` if empty.
    pub fn pop(&self) -> Option<Self> {
        Some(Self {
            inner: self.inner.drop_last()?,
            is_map_entry: false,
        })
    }

    /// Iterate over elements in index order.
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.inner.iter()
    }

    pub fn inner(&self) -> &rpds::VectorSync<Value> {
        &self.inner
    }
}

impl std::iter::FromIterator<Value> for PersistentVector {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        let mut v = rpds::VectorSync::new_sync();
        for item in iter {
            v = v.push_back(item);
        }
        Self {
            inner: v,
            is_map_entry: false,
        }
    }
}

impl PartialEq for PersistentVector {
    fn eq(&self, other: &Self) -> bool {
        if self.inner.len() != other.inner.len() {
            return false;
        }
        self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl cljrs_gc::Trace for PersistentVector {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for v in self.inner.iter() {
            v.trace(visitor);
        }
    }

    fn gc_size_extra(&self) -> usize {
        // Per element: Arc<T> allocation (16 overhead) + thin ptr in leaf node (8).
        let n = self.inner.len();
        n * (24 + std::mem::size_of::<Value>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn int(n: i64) -> Value {
        Value::Long(n)
    }

    #[test]
    fn test_empty() {
        let v = PersistentVector::empty();
        assert!(v.is_empty());
        assert_eq!(v.count(), 0);
        assert!(v.nth(0).is_none());
    }

    #[test]
    fn test_conj_small() {
        let v = PersistentVector::from_iter([int(1), int(2), int(3)]);
        assert_eq!(v.count(), 3);
        assert_eq!(v.nth(0), Some(&int(1)));
        assert_eq!(v.nth(2), Some(&int(3)));
    }

    #[test]
    fn test_conj_forces_tail_flush() {
        let v = PersistentVector::from_iter((0..33).map(int));
        assert_eq!(v.count(), 33);
        for i in 0..33 {
            assert_eq!(v.nth(i), Some(&int(i as i64)), "nth({i}) wrong");
        }
    }

    #[test]
    fn test_large() {
        let n = 1025;
        let v = PersistentVector::from_iter((0..n).map(|i| int(i as i64)));
        assert_eq!(v.count(), n);
        for i in 0..n {
            assert_eq!(v.nth(i), Some(&int(i as i64)));
        }
    }

    #[test]
    fn test_peek() {
        let v = PersistentVector::from_iter([int(1), int(2), int(3)]);
        assert_eq!(v.peek(), Some(&int(3)));
    }

    #[test]
    fn test_assoc_nth() {
        let v = PersistentVector::from_iter([int(1), int(2), int(3)]);
        let v2 = v.assoc_nth(1, int(99)).unwrap();
        assert_eq!(v2.nth(0), Some(&int(1)));
        assert_eq!(v2.nth(1), Some(&int(99)));
        assert_eq!(v2.nth(2), Some(&int(3)));
        // Original unchanged.
        assert_eq!(v.nth(1), Some(&int(2)));
    }

    #[test]
    fn test_pop() {
        let v = PersistentVector::from_iter([int(1), int(2), int(3)]);
        let v2 = v.pop().unwrap();
        assert_eq!(v2.count(), 2);
        assert_eq!(v2.nth(0), Some(&int(1)));
        assert_eq!(v2.nth(1), Some(&int(2)));
    }

    #[test]
    fn test_equality() {
        let a = PersistentVector::from_iter([int(1), int(2)]);
        let b = PersistentVector::from_iter([int(1), int(2)]);
        let c = PersistentVector::from_iter([int(1), int(3)]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_map_entry_flag() {
        let e = PersistentVector::map_entry(int(1), int(2));
        assert!(e.is_map_entry());
        assert_eq!(e.count(), 2);
        assert_eq!(e.nth(0), Some(&int(1)));
        assert_eq!(e.nth(1), Some(&int(2)));
        // Equal to a plain vector with the same elements.
        assert_eq!(e, PersistentVector::from_iter([int(1), int(2)]));
        // Plain constructors never produce map entries.
        assert!(!PersistentVector::from_iter([int(1), int(2)]).is_map_entry());
        // Derived vectors are plain vectors again.
        assert!(!e.conj(int(3)).is_map_entry());
        assert!(!e.assoc_nth(0, int(9)).unwrap().is_map_entry());
        assert!(!e.pop().unwrap().is_map_entry());
    }

    #[test]
    fn test_iter_order() {
        let v = PersistentVector::from_iter((0..10).map(|i| int(i as i64)));
        let items: Vec<_> = v.iter().cloned().collect();
        assert_eq!(items, (0..10).map(|i| int(i as i64)).collect::<Vec<_>>());
    }
}
