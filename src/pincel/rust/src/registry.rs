//! Which elements exist, by id, so the C interface can hand out ids instead of pointers.
//!
//! An element can be removed while the host still holds its id (a mod keeps a handle to something a screen
//! has since dropped). A raw pointer would then point at freed memory, but an id is only looked up here, and ids
//! are never reused, so a stale one finds nothing and the call is ignored.
//!
//! This guards the lookup, not the element: the host still has to take turns with a canvas, not use it from
//! two threads at once.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::element::Element;

fn map() -> MutexGuard<'static, HashMap<u32, usize>> {
    static MAP: OnceLock<Mutex<HashMap<u32, usize>>> = OnceLock::new();
    // A panic while the lock was held leaves the map itself fine, so keep going rather than poison everything.
    MAP.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

/// Records where an element is. Its address must stay the same for as long as it is in the tree, which is true
/// of anything in a `Box`.
pub(crate) fn register(el: &mut Element) {
    map().insert(el.uid, el as *mut Element as usize);
}

pub(crate) fn unregister(uid: u32) {
    map().remove(&uid);
}

/// The element with this id, if it still exists.
pub(crate) fn lookup(uid: u32) -> Option<*mut Element> {
    map().get(&uid).map(|&address| address as *mut Element)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Orientation;

    #[test]
    fn an_element_is_found_while_it_lives_and_not_after() {
        let mut root = Element::stack(Orientation::Vertical);
        let child_uid = root.add_child(Element::stack(Orientation::Vertical)).uid;
        let found = lookup(child_uid).expect("registered when added");
        assert_eq!(unsafe { (*found).uid }, child_uid);

        root.children.clear();
        assert!(lookup(child_uid).is_none(), "gone with the element");
    }
}
