//! Interactive forms.

use alloc::sync::Arc;

use crate::{Error, ErrorCode};

/// Options for [`Page::add_field`](crate::page::Page::add_field).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AddFieldOptions {
    /// JavaScript that computes the value.
    pub calculation_script: String,
    /// The choices of a dropdown or list.
    pub choices: Vec<String>,
    /// JavaScript that formats the value.
    pub format_script: String,
    /// The height.
    pub height: f64,
    /// The kind.
    pub kind: FieldKind,
    /// Fully qualified; an existing radio field's name adds a button.
    pub name: String,
    /// Not editable.
    pub read_only: bool,
    /// Must be filled.
    pub required: bool,
    /// The tooltip.
    pub tooltip: String,
    /// JavaScript that checks a new value.
    pub validation_script: String,
    /// The value.
    pub value: String,
    /// The width.
    pub width: f64,
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
}

/// What form data covers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum DataScope {
    /// Fields and annotations.
    All,
    /// Annotations.
    Annotations,
    /// Fields.
    #[default]
    Fields,
}

/// Options for [`Field::delete`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DeleteFieldOptions;

/// A form data exchange format.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExchangeFormat {
    /// FDF (ISO 32000-2:2020, 12.7.8).
    Fdf,
    /// JSON.
    Json,
    /// XFDF (ISO 19444-1).
    #[default]
    Xfdf,
    /// XML.
    Xml,
}

/// Options for [`Document::export_data`](crate::document::Document::export_data).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ExportDataOptions {
    /// Annotations support `Fdf` and `Xfdf`.
    pub format: ExchangeFormat,
    /// Empty: all pages.
    pub pages: Vec<u32>,
    /// What to export.
    pub scope: DataScope,
}

/// A form field.
#[derive(Clone, Debug)]
pub struct Field {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Field {
    /// Removes the field and its widgets.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn delete(&self, options: &DeleteFieldOptions) -> Result<(), Error> {
        let _: &DeleteFieldOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Field::delete"))
    }

    /// The field's properties and value.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<FieldInfo, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Field::info"))
    }

    /// Sets the field's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_info(&self, options: &SetFieldInfoOptions) -> Result<(), Error> {
        let _: &SetFieldInfoOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Field::set_info"))
    }

    /// Fills the field.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_value(&self, options: &SetValueOptions) -> Result<(), Error> {
        let _: &SetValueOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Field::set_value"))
    }
}

/// A form field's properties.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FieldInfo {
    /// JavaScript that computes the value.
    pub calculation_script: String,
    /// The choices of a dropdown or list.
    pub choices: Vec<String>,
    /// JavaScript that formats the value.
    pub format_script: String,
    /// The first widget's height.
    pub height: f64,
    /// The kind.
    pub kind: FieldKind,
    /// Fully qualified.
    pub name: String,
    /// The first widget's page; `None`: no widget.
    pub page: Option<u32>,
    /// Not editable.
    pub read_only: bool,
    /// Must be filled.
    pub required: bool,
    /// The tooltip.
    pub tooltip: String,
    /// JavaScript that checks a new value.
    pub validation_script: String,
    /// The value.
    pub value: String,
    /// The first widget's width.
    pub width: f64,
    /// The first widget's left edge.
    pub x: f64,
    /// The first widget's top edge.
    pub y: f64,
}

/// The kind of a form field.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum FieldKind {
    /// A push button.
    Button,
    /// A checkbox.
    Checkbox,
    /// A dropdown.
    Dropdown,
    /// A list.
    List,
    /// A radio button group.
    Radio,
    /// A signature.
    Signature,
    /// Text.
    #[default]
    Text,
}

/// A document's form fields.
#[derive(Clone, Debug)]
pub struct Fields {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Fields {
    /// The next field, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Field>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Fields::next"))
    }
}

/// Options for [`Document::import_data`](crate::document::Document::import_data).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ImportDataOptions {
    /// The data's format.
    pub format: ExchangeFormat,
}

/// Options for [`Field::set_info`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetFieldInfoOptions {
    /// JavaScript that computes the value.
    pub calculation_script: String,
    /// The choices of a dropdown or list.
    pub choices: Vec<String>,
    /// JavaScript that formats the value.
    pub format_script: String,
    /// Not editable.
    pub read_only: bool,
    /// Must be filled.
    pub required: bool,
    /// The tooltip.
    pub tooltip: String,
    /// JavaScript that checks a new value.
    pub validation_script: String,
}

/// Options for [`Field::set_value`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetValueOptions {
    /// A checkbox's export value or `Off`; a multi-select list's values separated by line feeds.
    pub value: String,
}

/// Options for [`Document::set_xfa`](crate::document::Document::set_xfa).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SetXfaOptions;

/// Options for [`Document::xfa`](crate::document::Document::xfa).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct XfaOptions;
