//! Layers (optional content).

use alloc::sync::Arc;

use crate::{Error, ErrorCode};

/// Options for [`Document::add_layer`](crate::document::Document::add_layer).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AddLayerOptions {
    /// The name.
    pub name: String,
    /// Visible by default; default `true`.
    pub visible: bool,
}

impl Default for AddLayerOptions {
    fn default() -> Self {
        Self {
            name: String::new(),
            visible: true,
        }
    }
}

/// A layer.
#[derive(Clone, Debug)]
pub struct Layer {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Layer {
    /// The layer's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<LayerInfo, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Layer::info"))
    }

    /// Shows or hides the layer by default.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_visibility(&self, options: &SetVisibilityOptions) -> Result<(), Error> {
        let _: &SetVisibilityOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Layer::set_visibility",
        ))
    }
}

/// A layer's properties.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LayerInfo {
    /// Viewers may not change its visibility.
    pub locked: bool,
    /// The name.
    pub name: String,
    /// Visible by default.
    pub visible: bool,
}

/// A document's layers.
#[derive(Clone, Debug)]
pub struct Layers {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Layers {
    /// The next layer, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Layer>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Layers::next"))
    }
}

/// Options for [`Layer::set_visibility`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SetVisibilityOptions {
    /// Visible by default.
    pub visible: bool,
}
