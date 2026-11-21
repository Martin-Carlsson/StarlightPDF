//! Page operations across a document.

/// Options for [`Document::impose`](crate::document::Document::impose).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ImposeOptions {
    /// Let annotations and fields drop.
    pub allow_loss: bool,
    /// `None`: 2.
    pub columns: Option<u32>,
    /// How pages go onto sheets.
    pub kind: Imposition,
    /// Points.
    pub margin: f64,
    /// `None`: 1.
    pub rows: Option<u32>,
    /// Points; `None`: A4 landscape for [`Imposition::NUp`], else A4.
    pub sheet_height: Option<f64>,
    /// Points; `None`: A4 landscape for [`Imposition::NUp`], else A4.
    pub sheet_width: Option<f64>,
}

/// How pages go onto sheets.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Imposition {
    /// Folded sheets in reading order.
    Booklet,
    /// `columns` × `rows` pages per sheet.
    #[default]
    NUp,
    /// One page enlarged across `columns` × `rows` sheets.
    Poster,
    /// One page cut into `columns` × `rows` sheets at its size.
    Tile,
}

/// Options for [`Document::insert_blank_page`](crate::document::Document::insert_blank_page).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct InsertBlankPageOptions {
    /// Points; `None`: A4.
    pub height: Option<f64>,
    /// The new page's position; `None`: after the last page.
    pub index: Option<u32>,
    /// Points; `None`: A4.
    pub width: Option<f64>,
}

/// Options for [`Document::insert_pages`](crate::document::Document::insert_pages).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct InsertPagesOptions {
    /// Where they go; `None`: after the last page.
    pub index: Option<u32>,
    /// `None`: all pages.
    pub pages: Option<Vec<u32>>,
}

/// Options for [`Document::insert_toc`](crate::document::Document::insert_toc).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct InsertTocOptions {
    /// Bookmark levels; `None`: all.
    pub depth: Option<u32>,
    /// Where it goes.
    pub index: u32,
    /// The heading.
    pub title: String,
}

/// Where a split starts each part.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum SplitBy {
    /// A part per bookmark at `level`.
    Bookmarks,
    /// `count` pages per part.
    #[default]
    Pages,
    /// A part starts at each of `starts`.
    Ranges,
    /// Parts of at most `max_bytes`.
    Size,
}

/// Options for [`Document::split`](crate::document::Document::split).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SplitOptions {
    /// Leaves out what a part cannot keep instead of failing with `LossRefused`.
    pub allow_loss: bool,
    /// Where parts start.
    pub by: SplitBy,
    /// Pages per part, for [`SplitBy::Pages`].
    pub count: u32,
    /// Bookmark level, 0 the top, for [`SplitBy::Bookmarks`].
    pub level: u32,
    /// Bytes per part, for [`SplitBy::Size`].
    pub max_bytes: u64,
    /// First pages of the parts, for [`SplitBy::Ranges`].
    pub starts: Vec<u32>,
}
