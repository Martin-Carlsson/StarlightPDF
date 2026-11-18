//! Progress and cancellation.

/// Receives progress from a long operation and can cancel it.
pub trait Progress: Sync {
    /// Reports `done` of `total` units; returning `false` cancels.
    ///
    /// It must not change the document the operation works on.
    fn update(&self, done: u64, total: u64) -> bool;
}
