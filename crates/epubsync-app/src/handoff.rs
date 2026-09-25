//! A value on its way back from a background task. An Iced message must
//! be `Clone` and `Debug`, and a `Library` is neither, so a task hands
//! its values back inside this handle and the update takes them out. The
//! import sends the library back this way and the sync sends the library
//! and the device.

use std::fmt;
use std::sync::{Arc, Mutex};

pub struct Handoff<T>(Arc<Mutex<Option<T>>>);

impl<T> Handoff<T> {
    pub fn new(value: T) -> Handoff<T> {
        Handoff(Arc::new(Mutex::new(Some(value))))
    }

    /// Takes the value out. A second take gets None.
    pub fn take(&self) -> Option<T> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// A clone shares the one value, so the take of either handle empties
/// both. The derived `Clone` would ask for `T: Clone`, which a `Library`
/// is not.
impl<T> Clone for Handoff<T> {
    fn clone(&self) -> Handoff<T> {
        Handoff(Arc::clone(&self.0))
    }
}

impl<T> fmt::Debug for Handoff<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Handoff")
    }
}
