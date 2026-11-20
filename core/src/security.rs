//! Encryption, permissions and sanitizing.

/// An encryption algorithm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Cipher {
    /// AES with a 128-bit key.
    Aes128,
    /// AES with a 256-bit key.
    Aes256,
    /// RC4 with a 40- to 128-bit key.
    Rc4,
    /// No encryption.
    Unencrypted,
}

/// How a document is protected.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Encryption {
    /// Certificate-based rather than password-based.
    pub certificate: bool,
    /// The algorithm.
    pub cipher: Cipher,
    /// What is denied.
    pub restrictions: Vec<Permission>,
}

/// Data a reader does not see.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HiddenData {
    /// Embedded files.
    Attachments,
    /// Layers hidden by default.
    HiddenLayers,
    /// Text that is not displayed.
    HiddenText,
    /// Document information and XMP metadata.
    Metadata,
    /// Application data such as `PieceInfo`.
    PrivateData,
    /// JavaScript and actions.
    Scripts,
}

/// A user permission (ISO 32000-2:2020, 7.6.4.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Permission {
    /// Add or change annotations and fill forms.
    Annotate,
    /// Insert, rotate or delete pages and change bookmarks.
    Assemble,
    /// Copy or extract text and graphics.
    Copy,
    /// Change the content.
    Edit,
    /// Fill form fields.
    FillForms,
    /// Print.
    Print,
    /// Print at full quality.
    PrintHighQuality,
}

/// Options for [`Document::sanitize`](crate::document::Document::sanitize).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SanitizeOptions {
    /// What to remove; `None`: all of it.
    pub remove: Option<Vec<HiddenData>>,
}
