//! Per-pixel buffers, counted so the pass without pixels knows what a band takes.

use alloc::rc::Rc;
use core::cell::Cell;

use crate::render::OPERATION;
use crate::render::geometry::Area;
use crate::{ErrorCode, PdfError};

/// The most columns of 0 between runs of a row that join them.
const JOINED: usize = 16;

/// A buffer over an area, empty while measuring.
pub(crate) struct Plane<T> {
    /// The area.
    pub area: Area,
    /// The count it leaves when dropped.
    pub planes: Planes,
    /// The bytes per row it counts.
    pub size: u64,
    /// Each row's runs of columns outside which its values are 0, as its maker found them; none
    /// where not known. Whoever changes values after clears them.
    pub spans: Spans,
    /// The values, row after row.
    pub values: Vec<T>,
}

/// Each row's runs of columns of a plane outside which its values are 0.
#[derive(Default)]
pub(crate) struct Spans {
    /// The runs of the row being found, from its first column, near ones joined.
    found: Vec<(usize, usize)>,
    /// Where each row's runs end in `runs`.
    rows: Vec<usize>,
    /// The runs, each row's after the one before's.
    runs: Vec<(i32, i32)>,
}

/// The planes alive, counted in bytes per row.
#[derive(Clone)]
pub(crate) struct Planes {
    /// The bytes now and at most.
    count: Rc<Cell<(u64, u64)>>,
    /// Whether planes only count.
    measure: bool,
}

impl<T> Drop for Plane<T> {
    fn drop(&mut self) {
        let (now, peak) = self.planes.count.get();
        self.planes.count.set((now.saturating_sub(self.size), peak));
    }
}

impl<T: Copy> Plane<T> {
    /// A plane over `area` that only counts its bytes, its values held elsewhere.
    pub(crate) fn counted(planes: &Planes, area: Area) -> Result<Self, PdfError> {
        let size = area
            .width()
            .checked_mul(size_of::<T>())
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, OPERATION))?;
        planes.hold(size);
        Ok(Self {
            area,
            planes: planes.clone(),
            size,
            spans: Spans::default(),
            values: Vec::new(),
        })
    }

    /// Counts `more` bytes per row beside its values, until it is dropped.
    pub(crate) fn counts(&mut self, more: u64) {
        self.planes.hold(more);
        self.size = self.size.saturating_add(more);
    }

    /// A plane of `fill` over `area`.
    pub(crate) fn new(planes: &Planes, area: Area, fill: T) -> Result<Self, PdfError> {
        let limit = || PdfError::new(ErrorCode::LimitExceeded, OPERATION);
        let size = area
            .width()
            .checked_mul(size_of::<T>())
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or_else(limit)?;
        let mut values = Vec::new();
        if !planes.measure {
            let count = area.width().checked_mul(area.height()).ok_or_else(limit)?;
            values
                .try_reserve_exact(count)
                .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
            values.resize(count, fill);
        }
        planes.hold(size);
        Ok(Self {
            area,
            planes: planes.clone(),
            size,
            spans: Spans::default(),
            values,
        })
    }

    /// The values of row `row`; empty outside the area and while measuring.
    pub(crate) fn row(&self, row: i32) -> &[T] {
        span(self.area, row)
            .and_then(|(from, to)| self.values.get(from..to))
            .unwrap_or_default()
    }

    /// The values of row `row` to change; empty outside the area and while measuring.
    pub(crate) fn row_mut(&mut self, row: i32) -> &mut [T] {
        span(self.area, row)
            .and_then(|(from, to)| self.values.get_mut(from..to))
            .unwrap_or_default()
    }
}

impl Planes {
    /// Counts `size` until the plane is dropped.
    fn hold(&self, size: u64) {
        let (now, peak) = self.count.get();
        let next = now.saturating_add(size);
        self.count.set((next, peak.max(next)));
    }

    /// Whether planes only count.
    pub(crate) const fn measuring(&self) -> bool {
        self.measure
    }

    /// No planes yet; with `measure`, they only count.
    pub(crate) fn new(measure: bool) -> Self {
        Self {
            count: Rc::new(Cell::new((0, 0))),
            measure,
        }
    }

    /// The most bytes per row alive at once.
    pub(crate) fn peak(&self) -> u64 {
        self.count.get().1
    }
}

impl Spans {
    /// The bytes the runs of a row of `width` columns may take.
    pub(crate) fn bytes(width: usize) -> u64 {
        // The runs being found count with each row too, as they are held once a mask.
        let row = most_runs(width)
            .saturating_mul(size_of::<(i32, i32)>().saturating_add(size_of::<(usize, usize)>()))
            .saturating_add(size_of::<usize>());
        u64::try_from(row).unwrap_or(u64::MAX)
    }

    /// Forgets them all: they are not known.
    pub(crate) fn clear(&mut self) {
        self.found.clear();
        self.rows.clear();
        self.runs.clear();
    }

    /// Ends the row being found, whose first column is `left`.
    pub(crate) fn end_row(&mut self, left: i32) {
        let column = |index: usize| left.saturating_add(i32::try_from(index).unwrap_or(i32::MAX));
        self.runs.extend(
            self.found
                .drain(..)
                .map(|(from, to)| (column(from), column(to))),
        );
        self.rows.push(self.runs.len());
    }

    /// Adds columns `from` to `to` of the row being found; one near the last joins it, as
    /// painting a few columns of 0 costs less than a run apart.
    pub(crate) fn push(&mut self, (from, to): (usize, usize)) {
        match self.found.last_mut() {
            Some(last) if last.1.saturating_add(JOINED) >= from => last.1 = to,
            Some(_) | None => self.found.push((from, to)),
        }
    }

    /// Room for the runs of `rows` rows of `width` columns, all a row can have, so that finding
    /// them never allocates.
    pub(crate) fn reserve(&mut self, rows: usize, width: usize) -> Result<(), PdfError> {
        let most = most_runs(width);
        let failed = |_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION);
        self.found.try_reserve_exact(most).map_err(failed)?;
        self.rows.try_reserve_exact(rows).map_err(failed)?;
        self.runs
            .try_reserve_exact(rows.saturating_mul(most))
            .map_err(failed)
    }

    /// The runs of row `index`, from the top; none where not known.
    pub(crate) fn row(&self, index: usize) -> Option<&[(i32, i32)]> {
        let end = *self.rows.get(index)?;
        let start = index
            .checked_sub(1)
            .and_then(|at| self.rows.get(at))
            .copied()
            .unwrap_or(0);
        self.runs.get(start..end)
    }
}

/// The most runs a row of `width` columns can have: those nearer are joined.
const fn most_runs(width: usize) -> usize {
    width.div_ceil(JOINED.saturating_add(1)).saturating_add(1)
}

/// Where row `row` of `area` lies.
fn span(area: Area, row: i32) -> Option<(usize, usize)> {
    let index = usize::try_from(row.checked_sub(area.top)?).ok()?;
    let width = area.width();
    let from = index.checked_mul(width)?;
    Some((from, from.checked_add(width)?))
}
