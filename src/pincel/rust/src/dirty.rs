//! Tracking of when the element tree changes, so a canvas only lays out and rasterizes again when it must.
//!
//! Everything that changes how the tree looks (adding an element, setting a style, changing text) calls
//! [`invalidate`]. A canvas remembers the [`revision`] it last laid out at and does the work again only if it moved.
//! Setters should only invalidate when a value really changes, so setting the same style every frame is free.
//!
//! The counter is shared by the whole process, so the tree can be built on one thread and drawn on another, which
//! is how a game with a render thread works. (A canvas itself is still not safe to use from two threads at once.
//! The host has to take turns.) A change to any canvas makes every canvas lay out again, which only costs time.
//! In tests it is per thread instead, so tests running side by side can't disturb each other's counts.

#[cfg(not(test))]
mod counter {
    use std::sync::atomic::{AtomicU64, Ordering};

    static REVISION: AtomicU64 = AtomicU64::new(0);

    pub fn bump() {
        REVISION.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get() -> u64 {
        REVISION.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod counter {
    use std::cell::Cell;

    thread_local! {
        static REVISION: Cell<u64> = const { Cell::new(0) };
    }

    pub fn bump() {
        REVISION.with(|r| r.set(r.get() + 1));
    }

    pub fn get() -> u64 {
        REVISION.with(Cell::get)
    }
}

/// Says that something in some tree changed in a way that needs layout and drawing again.
/// An element that changes itself in [`crate::element::Element::update`] calls this.
pub fn invalidate() {
    counter::bump();
}

/// A number that goes up whenever [`invalidate`] is called.
pub fn revision() -> u64 {
    counter::get()
}

/// Sets `slot` to `value`, and invalidates, only if that is a change.
pub fn set<T: PartialEq>(slot: &mut T, value: T) {
    if *slot != value {
        *slot = value;
        invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_only_invalidates_on_a_change() {
        let mut value = 1;
        let before = revision();
        set(&mut value, 1);
        assert_eq!(revision(), before, "same value, nothing to redo");
        set(&mut value, 2);
        assert_eq!((value, revision()), (2, before + 1));
    }
}
