//! Bookmarks, destinations, viewer settings and actions.
//!
//! # Targets
//!
//! A target is a URI, `javascript:` and a script, or `#` and a fragment as in RFC 8118, such as `#page=3&zoom=150`
//! or `#nameddest=intro`, its page numbers 1-based.

use alloc::sync::Arc;

use crate::{Error, ErrorCode};

/// Options for [`Document::add_article`](crate::document::Document::add_article).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AddArticleOptions {
    /// Each bead's page index.
    pub bead_pages: Vec<u32>,
    /// Each bead's x, y, width and height, in [page coordinates](crate::page#coordinates).
    pub beads: Vec<f64>,
    /// The title.
    pub title: String,
}

/// Options for [`Document::add_bookmark`](crate::document::Document::add_bookmark).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AddBookmarkOptions {
    /// Bold title.
    pub bold: bool,
    /// The title's [color](crate::drawing#colors).
    pub color: String,
    /// Position among siblings; `None`: last.
    pub index: Option<u32>,
    /// Italic title.
    pub italic: bool,
    /// Children shown.
    pub open: bool,
    /// Where it goes, a [target](crate::navigation#targets).
    pub target: String,
    /// The title.
    pub title: String,
}

/// An article: beads read in sequence (ISO 32000-2:2020, 12.4.3).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Article {
    /// Each bead's page index.
    pub bead_pages: Vec<u32>,
    /// Each bead's x, y, width and height, in [page coordinates](crate::page#coordinates).
    pub beads: Vec<f64>,
    /// The title.
    pub title: String,
}

/// A document's articles.
#[derive(Clone, Debug)]
pub struct Articles {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Articles {
    /// The next article, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Article>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Articles::next"))
    }
}

/// A bookmark (outline item).
#[derive(Clone, Debug)]
pub struct Bookmark {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Bookmark {
    /// The nested bookmarks.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn children(&self) -> Result<Bookmarks, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Bookmark::children"))
    }

    /// Removes the bookmark and its children.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn delete(&self, options: &DeleteBookmarkOptions) -> Result<(), Error> {
        let _: &DeleteBookmarkOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Bookmark::delete"))
    }

    /// The bookmark's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<BookmarkInfo, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Bookmark::info"))
    }

    /// Sets the bookmark's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_info(&self, options: &SetBookmarkInfoOptions) -> Result<(), Error> {
        let _: &SetBookmarkInfoOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Bookmark::set_info"))
    }
}

/// A bookmark's properties.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BookmarkInfo {
    /// Bold title.
    pub bold: bool,
    /// The title's [color](crate::drawing#colors).
    pub color: String,
    /// Italic title.
    pub italic: bool,
    /// Children shown.
    pub open: bool,
    /// Where it goes, a [target](crate::navigation#targets).
    pub target: String,
    /// The title.
    pub title: String,
}

/// Bookmarks at one level.
#[derive(Clone, Debug)]
pub struct Bookmarks {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Bookmarks {
    /// The next bookmark, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn next(&self) -> Result<Option<Bookmark>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Bookmarks::next"))
    }
}

/// Options for [`Bookmark::delete`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DeleteBookmarkOptions;

/// Options for [`Document::delete_label_range`](crate::document::Document::delete_label_range).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DeleteLabelRangeOptions {
    /// First page of the range.
    pub start: u32,
}

/// A named destination.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Destination {
    /// The name.
    pub name: String,
    /// Where it goes, a [target](crate::navigation#targets).
    pub target: String,
}

/// A document's named destinations.
#[derive(Clone, Debug)]
pub struct Destinations {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Destinations {
    /// The next destination, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Destination>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Destinations::next"))
    }
}

/// A page label range (ISO 32000-2:2020, 12.4.2).
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LabelRange {
    /// The first page's number.
    pub first_number: u32,
    /// Text before the number.
    pub prefix: String,
    /// First page of the range.
    pub start: u32,
    /// The numbering style.
    pub style: LabelStyle,
}

/// A document's page label ranges.
#[derive(Clone, Debug)]
pub struct LabelRanges {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl LabelRanges {
    /// The next range, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<LabelRange>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "LabelRanges::next"))
    }
}

/// A page label numbering style.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum LabelStyle {
    /// 1, 2, 3.
    #[default]
    Decimal,
    /// a, b, c.
    LowerLetters,
    /// i, ii, iii.
    LowerRoman,
    /// Prefix only.
    Prefix,
    /// A, B, C.
    UpperLetters,
    /// I, II, III.
    UpperRoman,
}

/// How pages are laid out when opened.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum PageLayout {
    /// One continuous column.
    OneColumn,
    /// One page at a time.
    #[default]
    SinglePage,
    /// Two continuous columns, odd pages left.
    TwoColumnLeft,
    /// Two continuous columns, odd pages right.
    TwoColumnRight,
    /// Two pages at a time, odd pages left.
    TwoPageLeft,
    /// Two pages at a time, odd pages right.
    TwoPageRight,
}

/// The panel shown when opened.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum PageMode {
    /// Attachments.
    Attachments,
    /// Full screen.
    FullScreen,
    /// Layers.
    Layers,
    /// Bookmarks.
    Outline,
    /// None.
    #[default]
    Plain,
    /// Page thumbnails.
    Thumbnails,
}

/// A document-level script.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Script {
    /// The JavaScript.
    pub code: String,
    /// The name.
    pub name: String,
}

/// A document's scripts.
#[derive(Clone, Debug)]
pub struct Scripts {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Scripts {
    /// The next script, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Script>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Scripts::next"))
    }
}

/// Options for [`Bookmark::set_info`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetBookmarkInfoOptions {
    /// Bold title.
    pub bold: bool,
    /// The title's [color](crate::drawing#colors).
    pub color: String,
    /// Italic title.
    pub italic: bool,
    /// Children shown.
    pub open: bool,
    /// Where it goes, a [target](crate::navigation#targets).
    pub target: String,
    /// The title.
    pub title: String,
}

/// Options for [`Document::set_destination`](crate::document::Document::set_destination).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetDestinationOptions {
    /// The name.
    pub name: String,
    /// Where it goes, a [target](crate::navigation#targets); empty removes it.
    pub target: String,
}

/// Options for [`Document::set_label_range`](crate::document::Document::set_label_range).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetLabelRangeOptions {
    /// `None`: 1.
    pub first_number: Option<u32>,
    /// Text before the number.
    pub prefix: String,
    /// First page of the range.
    pub start: u32,
    /// The numbering style.
    pub style: LabelStyle,
}

/// Options for [`Document::set_script`](crate::document::Document::set_script).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetScriptOptions {
    /// The JavaScript; empty removes it.
    pub code: String,
    /// The name.
    pub name: String,
}

/// Options for [`Document::set_viewer_settings`](crate::document::Document::set_viewer_settings).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetViewerSettingsOptions {
    /// How pages are laid out.
    pub layout: PageLayout,
    /// The panel shown.
    pub mode: PageMode,
    /// Opening page, layout and zoom, a [target](crate::navigation#targets).
    pub open_target: String,
    /// Viewer preferences to turn on.
    pub preferences: Vec<ViewerPreference>,
}

/// A viewer preference (ISO 32000-2:2020, 12.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ViewerPreference {
    /// Center the window on the screen.
    CenterWindow,
    /// Show the document title, not the file name.
    DisplayDocTitle,
    /// Fit the window to the first page.
    FitWindow,
    /// Hide the menu bar.
    HideMenubar,
    /// Hide the toolbars.
    HideToolbar,
    /// Hide window controls.
    HideWindowUi,
    /// Read right to left.
    RightToLeft,
}

/// How viewers open the document.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ViewerSettings {
    /// How pages are laid out.
    pub layout: PageLayout,
    /// The panel shown.
    pub mode: PageMode,
    /// Opening page, layout and zoom, a [target](crate::navigation#targets).
    pub open_target: String,
    /// Viewer preferences turned on.
    pub preferences: Vec<ViewerPreference>,
}
