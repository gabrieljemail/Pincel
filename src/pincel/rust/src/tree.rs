//! Editing the element tree through an element's id: removing, moving and reordering.
//!
//! These work from a raw pointer, which the C interface gets from an id, rather than from a reference. An
//! element's parent holds the box it lives in, so taking an element out of its parent is something the element
//! can't do through its own reference.

use std::ptr;

use crate::dirty::invalidate;
use crate::element::Element;

/// Takes `el` out of its parent and returns it, with everything inside it. Dropping it frees them.
/// `None` for a canvas's root, and for an element that isn't in a tree.
///
/// # Safety
/// `el` must point at a live element, and nothing else may be using the tree.
pub unsafe fn detach(el: *mut Element) -> Option<Box<Element>> {
    let parent = (*el).parent;
    if parent.is_null() {
        return None;
    }
    let siblings = &mut (*parent).children;
    let index = siblings.iter().position(|child| ptr::eq(&**child, el))?;
    let mut removed = siblings.remove(index);
    removed.parent = ptr::null_mut();
    invalidate();
    Some(removed)
}

/// Removes `el` from the tree and frees it and everything inside it. Returns false, doing nothing, for a canvas's root.
///
/// # Safety
/// As for [`detach`]. Nothing may use the element (or anything inside it) afterwards.
pub unsafe fn remove(el: *mut Element) -> bool {
    detach(el).is_some()
}

/// Whether `ancestor` is `el` or one of the elements `el` is inside.
unsafe fn is_self_or_ancestor(ancestor: *const Element, el: *const Element) -> bool {
    let mut current = el;
    while !current.is_null() {
        if ptr::eq(current, ancestor) {
            return true;
        }
        current = (*current).parent;
    }
    false
}

/// Moves `el`, with everything inside it, to be the child at `index` of `new_parent` (the last one, for an index past
/// it). That can be its current parent, which reorders it. Returns false, changing nothing, for a canvas's root, or if
/// `new_parent` is `el` or inside it, which would make a loop.
///
/// # Safety
/// As for [`detach`], and `new_parent` must point at a live element too.
pub unsafe fn move_to(el: *mut Element, new_parent: *mut Element, index: usize) -> bool {
    if (*el).parent.is_null() || is_self_or_ancestor(el, new_parent) {
        return false;
    }
    let Some(mut moved) = detach(el) else { return false };
    moved.parent = new_parent;
    let siblings = &mut (*new_parent).children;
    siblings.insert(index.min(siblings.len()), moved);
    invalidate();
    true
}

/// Where `el` is among its parent's children, or `None` for a canvas's root.
///
/// # Safety
/// As for [`detach`].
pub unsafe fn index_of(el: *const Element) -> Option<usize> {
    let parent = (*el).parent;
    if parent.is_null() {
        return None;
    }
    (*parent).children.iter().position(|child| ptr::eq(&**child, el))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;
    use crate::element::Orientation;
    use crate::registry;

    fn ids(parent: &Element) -> Vec<String> {
        parent.children.iter().map(|c| c.id.clone()).collect()
    }

    fn named(id: &str) -> Element {
        let mut el = Element::stack(Orientation::Vertical);
        el.id = id.to_string();
        el
    }

    /// A canvas whose root holds a, b and c, with a holding a1 and a2.
    fn canvas() -> Canvas {
        let mut canvas = Canvas::new();
        for id in ["a", "b", "c"] {
            canvas.root.add_child(named(id));
        }
        canvas.root.children[0].add_child(named("a1"));
        canvas.root.children[0].add_child(named("a2"));
        canvas
    }

    fn at(canvas: &mut Canvas, i: usize) -> *mut Element {
        &mut *canvas.root.children[i]
    }

    #[test]
    fn children_can_be_inserted_at_a_position() {
        let mut root = Element::stack(Orientation::Vertical);
        root.add_child(named("a"));
        root.add_child(named("c"));
        root.insert_child(1, named("b"));
        root.insert_child(0, named("first"));
        root.insert_child(99, named("last"));
        assert_eq!(ids(&root), ["first", "a", "b", "c", "last"]);
    }

    #[test]
    fn removing_a_child_frees_it_and_everything_in_it() {
        let mut canvas = canvas();
        let a_uid = canvas.root.children[0].uid;
        let a1_uid = canvas.root.children[0].children[0].uid;
        assert!(registry::lookup(a1_uid).is_some());

        assert!(unsafe { remove(at(&mut canvas, 0)) });
        assert_eq!(ids(&canvas.root), ["b", "c"]);
        assert!(registry::lookup(a_uid).is_none() && registry::lookup(a1_uid).is_none(), "a stale id finds nothing");
    }

    #[test]
    fn the_root_cannot_be_removed_or_moved() {
        let mut canvas = canvas();
        let root: *mut Element = &mut *canvas.root;
        let child = at(&mut canvas, 1);
        assert!(!unsafe { remove(root) });
        assert!(!unsafe { move_to(root, child, 0) });
        assert_eq!(ids(&canvas.root), ["a", "b", "c"]);
    }

    #[test]
    fn clearing_children_removes_them_all() {
        let mut canvas = canvas();
        let uids: Vec<u32> = canvas.root.children.iter().map(|c| c.uid).collect();
        canvas.root.clear_children();
        assert!(canvas.root.children.is_empty());
        assert!(uids.iter().all(|&uid| registry::lookup(uid).is_none()));
    }

    #[test]
    fn an_element_can_be_reordered_within_its_parent() {
        let mut canvas = canvas();
        let root: *mut Element = &mut *canvas.root;
        assert!(unsafe { move_to(at(&mut canvas, 2), root, 0) }); // c to the front.
        assert_eq!(ids(&canvas.root), ["c", "a", "b"]);
        assert!(unsafe { move_to(at(&mut canvas, 0), root, 99) }); // And to the end again.
        assert_eq!(ids(&canvas.root), ["a", "b", "c"]);
    }

    #[test]
    fn an_element_can_move_to_another_parent_with_its_contents() {
        let mut canvas = canvas();
        let a1 = &mut *canvas.root.children[0].children[0] as *mut Element;
        let b = at(&mut canvas, 1);
        let uid = unsafe { (*a1).uid };

        assert!(unsafe { move_to(a1, b, 0) });
        assert_eq!(ids(&canvas.root.children[0]), ["a2"]);
        assert_eq!(ids(&canvas.root.children[1]), ["a1"]);
        assert_eq!(unsafe { index_of(a1) }, Some(0));
        assert!(registry::lookup(uid).is_some(), "it was moved, not freed");

        // And it can be removed from its new place just like any other.
        assert!(unsafe { remove(a1) });
        assert!(canvas.root.children[1].children.is_empty());
    }

    #[test]
    fn moving_an_element_into_itself_or_what_it_holds_is_refused() {
        let mut canvas = canvas();
        let a = at(&mut canvas, 0);
        let a1 = &mut *canvas.root.children[0].children[0] as *mut Element;
        assert!(!unsafe { move_to(a, a, 0) });
        assert!(!unsafe { move_to(a, a1, 0) });
        assert_eq!(ids(&canvas.root), ["a", "b", "c"]);
        assert_eq!(ids(&canvas.root.children[0]), ["a1", "a2"]);
    }

    #[test]
    fn an_element_built_before_it_was_added_keeps_working_parent_links() {
        let mut canvas = Canvas::new();
        let mut group = named("group");
        group.add_child(named("inner")); // Built first, then the group (with its child) is added.
        canvas.root.add_child(group);

        let inner = &mut *canvas.root.children[0].children[0] as *mut Element;
        assert!(unsafe { remove(inner) }, "the inner one found its parent at the group's new address");
        assert!(canvas.root.children[0].children.is_empty());
    }

    #[test]
    fn find_looks_down_the_tree_by_id() {
        let canvas = canvas();
        assert_eq!(canvas.root.find("a2").map(|e| e.id.as_str()), Some("a2"));
        assert!(canvas.root.find("nope").is_none());
    }
}
