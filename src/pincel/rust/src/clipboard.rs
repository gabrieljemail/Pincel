//! The clipboard an Input copies to and pastes from.
//!
//! Pincel can't reach the operating system's clipboard itself (it doesn't know what window system the host uses), so
//! the host can give the canvas two callbacks. Without them, copy and paste still work between inputs in the canvas,
//! through a clipboard of its own.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void, CStr, CString};

/// Returns the clipboard's text as a NUL-terminated UTF-8 string, which the host keeps valid until the next call, or
/// null if it has no text.
pub type GetFn = extern "C" fn(user: *mut c_void) -> *const c_char;
/// Puts NUL-terminated UTF-8 text on the clipboard.
pub type SetFn = extern "C" fn(user: *mut c_void, text: *const c_char);

#[derive(Clone, Copy)]
pub struct Hooks {
    pub get: Option<GetFn>,
    pub set: Option<SetFn>,
    pub user: *mut c_void,
}

// The user pointer is only handed back to the host's own functions, and a canvas is used by one thread at a time.
unsafe impl Send for Hooks {}

#[derive(Default)]
pub struct Clipboard {
    /// What was last copied inside the canvas, which is what pasting gives when the host has no clipboard to ask.
    internal: RefCell<String>,
    hooks: Cell<Option<Hooks>>,
}

impl Clipboard {
    /// Uses the host's clipboard from now on (or goes back to the internal one, for `None`).
    pub fn set_hooks(&self, hooks: Option<Hooks>) {
        self.hooks.set(hooks);
    }

    /// The text to paste.
    pub fn get(&self) -> String {
        if let Some(Hooks { get: Some(get), user, .. }) = self.hooks.get() {
            let text = get(user);
            if text.is_null() {
                return String::new();
            }
            // SAFETY: the host promises a NUL-terminated string that stays valid until the next call.
            return unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
        }
        self.internal.borrow().clone()
    }

    /// Copies `text`: to the host's clipboard if there is one, and always to the internal one.
    pub fn set(&self, text: &str) {
        *self.internal.borrow_mut() = text.to_string();
        if let Some(Hooks { set: Some(set), user, .. }) = self.hooks.get() {
            // A NUL can't be in a C string, so what follows one is cut off.
            let cut = text.split('\0').next().unwrap_or("");
            if let Ok(c_text) = CString::new(cut) {
                set(user, c_text.as_ptr());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn without_a_host_it_remembers_what_was_copied() {
        let clipboard = Clipboard::default();
        assert_eq!(clipboard.get(), "");
        clipboard.set("hello");
        assert_eq!(clipboard.get(), "hello");
    }

    thread_local! {
        static HOST: RefCell<String> = const { RefCell::new(String::new()) };
        static HOST_TEXT: RefCell<CString> = RefCell::new(CString::default());
    }

    extern "C" fn host_get(_user: *mut c_void) -> *const c_char {
        HOST.with(|h| HOST_TEXT.with(|t| {
            *t.borrow_mut() = CString::new(h.borrow().clone()).unwrap();
            t.borrow().as_ptr()
        }))
    }

    extern "C" fn host_set(_user: *mut c_void, text: *const c_char) {
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
        HOST.with(|h| *h.borrow_mut() = text);
    }

    #[test]
    fn with_host_callbacks_it_reads_and_writes_the_hosts_clipboard() {
        let clipboard = Clipboard::default();
        clipboard.set_hooks(Some(Hooks { get: Some(host_get), set: Some(host_set), user: std::ptr::null_mut() }));

        HOST.with(|h| *h.borrow_mut() = "from the OS".to_string());
        assert_eq!(clipboard.get(), "from the OS");

        clipboard.set("copied here");
        HOST.with(|h| assert_eq!(*h.borrow(), "copied here"));
        assert_eq!(clipboard.get(), "copied here", "and reads it back from the host");

        clipboard.set_hooks(None);
        assert_eq!(clipboard.get(), "copied here", "the internal copy is kept, so unplugging the host loses nothing");
    }
}
