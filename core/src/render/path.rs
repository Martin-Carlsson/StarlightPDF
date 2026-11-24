//! Paths in device space (ISO 32000-2:2020, 8.5.2).

use alloc::rc::Rc;
use core::cell::RefCell;

use crate::render::OPERATION;
use crate::render::coverage::{Coverage, Rule};
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::geometry::{Area, Matrix, Point};
use crate::render::job::Job;
use crate::render::math;
use crate::render::plane::Plane;
use crate::render::stroke::{self, Pen, Stroker, Style};
use crate::{ErrorCode, PdfError};

/// Work of finding the pieces of a curve.
const CURVE_WORK: u64 = 24;

/// The most points, and the most segments, a path holds: 512 KiB together.
const HELD: usize = 0x7800;

/// Work of keeping a segment.
const KEEP_WORK: u64 = 6;

/// The most pieces of a curve.
const PIECES: i64 = 256;

/// A segment.
#[derive(Clone, Copy)]
pub(crate) enum Event {
    /// The line back to the subpath's start.
    Close,
    /// A cubic Bezier curve.
    Curve([Point; 3]),
    /// A line.
    Line(Point),
    /// The start of a subpath.
    Move(Point),
}

/// A segment, held apart from its points.
#[derive(Clone, Copy)]
enum Verb {
    /// The line back to the subpath's start.
    Close,
    /// A curve of three points.
    Curve,
    /// A line.
    Line,
    /// The start of a subpath.
    Move,
}

/// A cubic Bezier curve.
#[derive(Clone, Copy)]
pub(crate) struct Curve {
    /// Its four points.
    points: [Point; 4],
}

/// The edges that fill a path.
struct Filler {
    /// What they cover.
    cover: Coverage,
    /// The current point.
    current: Point,
    /// Where the subpath started.
    start: Point,
}

/// A path.
pub(crate) struct Path {
    /// Where it may paint.
    area: Area,
    /// Its dashes are invalid, warned of only once it is stroked.
    bad_dash: bool,
    /// Left, top, right and bottom of its points.
    extent: Option<[Fixed; 4]>,
    /// A subpath is begun and not closed.
    open: bool,
    /// The map and style current at its first point.
    pen: (Matrix, Style),
    /// The points held.
    points: Vec<Point>,
    /// Past what is held: what its segments cover.
    streamed: Option<RefCell<Streamed>>,
    /// The segments held.
    verbs: Vec<Verb>,
}

/// A path too long to hold, filled and stroked as it arrives.
struct Streamed {
    /// Filled.
    fill: Filler,
    /// Stroked.
    stroke: Stroker,
}

impl Curve {
    /// The point at `part`, from 0 to one.
    pub(crate) fn at(&self, part: Fixed) -> Point {
        let between = |from: Fixed, to: Fixed| {
            let [start, end, share] = [from, to, part].map(|value| i128::from(value.bits()));
            let moved = end
                .saturating_sub(start)
                .saturating_mul(share)
                .saturating_add(0x8000_0000)
                >> 32_u32;
            Fixed::from_bits(saturated(start.saturating_add(moved)))
        };
        let middle = |from: Point, to: Point| Point {
            across: between(from.across, to.across),
            down: between(from.down, to.down),
        };
        let [start, first, second, end] = self.points;
        let (early, inner, late) = (
            middle(start, first),
            middle(first, second),
            middle(second, end),
        );
        middle(middle(early, inner), middle(inner, late))
    }

    /// The curve from `start` by `points`.
    pub(crate) const fn new(start: Point, points: [Point; 3]) -> Self {
        let [first, second, end] = points;
        Self {
            points: [start, first, second, end],
        }
    }

    /// How many lines draw it to within a quarter pixel.
    pub(crate) fn pieces(&self) -> i64 {
        let [start, first, second, end] = self.points;
        let bend = |one: Fixed, two: Fixed, three: Fixed| {
            let [before, middle, after] = [one, two, three].map(|value| i128::from(value.bits()));
            let found = before
                .saturating_add(after)
                .saturating_sub(middle.saturating_mul(2));
            Fixed::from_bits(saturated(found))
        };
        let second_difference = |one: Point, two: Point, three: Point| {
            math::hypotenuse(Point {
                across: bend(one.across, two.across, three.across),
                down: bend(one.down, two.down, three.down),
            })
        };
        let most =
            second_difference(start, first, second).max(second_difference(first, second, end));
        // n lines are at most 3/4 of it over n squared away.
        let count = math::root(most.times(Fixed::from_i64(3)));
        Fixed::from_bits(count.bits().saturating_add(0xFFFF_FFFF))
            .floor()
            .clamp(1, PIECES)
    }

    /// Where line `index` of `count` ends.
    pub(crate) fn step(&self, index: i64, count: i64) -> Point {
        if index >= count {
            let [_, _, _, end] = self.points;
            return end;
        }
        let part = quotient(i128::from(index) << 32_u32, i128::from(count)).unwrap_or(0);
        self.at(Fixed::from_bits(saturated(part)))
    }
}

impl Filler {
    /// Closes the subpath, as filling does (8.5.3.1).
    fn end(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        self.line(job, self.start)
    }

    /// Adds `event`.
    fn event(&mut self, job: &Job<'_>, event: Event) -> Result<(), PdfError> {
        match event {
            Event::Close => self.end(job),
            Event::Curve(points) => {
                let [first, second, end] = points;
                let hull = [self.current, first, second, end];
                if self.cover.misses(&hull, Fixed::default()) {
                    return self.line(job, end);
                }
                job.spend(CURVE_WORK)?;
                let curve = Curve::new(self.current, points);
                let count = curve.pieces();
                for index in 1..=count {
                    self.line(job, curve.step(index, count))?;
                }
                Ok(())
            }
            Event::Line(point) => self.line(job, point),
            Event::Move(point) => {
                self.end(job)?;
                (self.current, self.start) = (point, point);
                Ok(())
            }
        }
    }

    /// A line to `point`.
    fn line(&mut self, job: &Job<'_>, point: Point) -> Result<(), PdfError> {
        self.cover.edge(job, self.current, point)?;
        self.current = point;
        Ok(())
    }

    /// Nothing filled in `area`.
    fn new(job: &Job<'_>, area: Area) -> Result<Self, PdfError> {
        Ok(Self {
            cover: Coverage::new(job, area)?,
            current: Point::default(),
            start: Point::default(),
        })
    }
}

impl Path {
    /// The pixels it touches, within its area.
    pub(crate) fn bounds(&self) -> Area {
        self.touched(Fixed::default())
    }

    /// Closes the subpath.
    pub(crate) fn close(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        if !self.open {
            return Ok(());
        }
        self.open = false;
        self.push(job, Event::Close)
    }

    /// Adds a cubic Bezier curve.
    pub(crate) fn curve_to(&mut self, job: &Job<'_>, points: [Point; 3]) -> Result<(), PdfError> {
        if self.extent.is_none() {
            let [first, _, _] = points;
            self.move_to(job, first)?;
        }
        self.open = true;
        for point in points {
            self.reach(point);
        }
        self.push(job, Event::Curve(points))
    }

    /// Runs `feed` on each segment held.
    fn each<Feed>(&self, mut feed: Feed) -> Result<(), PdfError>
    where
        Feed: FnMut(Event) -> Result<(), PdfError>,
    {
        let mut points = self.points.iter().copied();
        let mut next = || points.next().unwrap_or_default();
        for verb in &self.verbs {
            feed(match *verb {
                Verb::Close => Event::Close,
                Verb::Curve => Event::Curve([next(), next(), next()]),
                Verb::Line => Event::Line(next()),
                Verb::Move => Event::Move(next()),
            })?;
        }
        Ok(())
    }

    /// What `rule` fills within `area`.
    pub(crate) fn fill(
        &self,
        job: &Job<'_>,
        rule: Rule,
        area: Area,
    ) -> Result<Plane<u8>, PdfError> {
        let within = area.intersect(self.area);
        if let Some(streamed) = self.streamed.as_ref() {
            let mut held = streamed.borrow_mut();
            held.fill.end(job)?;
            return held.fill.cover.mask(job, rule, within);
        }
        let mut filler = Filler::new(job, within)?;
        if !filler.cover.idle(job) {
            self.each(|event| filler.event(job, event))?;
            filler.end(job)?;
        }
        filler.cover.finish(job, rule)
    }

    /// Adds a line.
    pub(crate) fn line_to(&mut self, job: &Job<'_>, point: Point) -> Result<(), PdfError> {
        if self.extent.is_none() {
            return self.move_to(job, point);
        }
        self.open = true;
        self.reach(point);
        self.push(job, Event::Line(point))
    }

    /// Starts a subpath.
    pub(crate) fn move_to(&mut self, job: &Job<'_>, point: Point) -> Result<(), PdfError> {
        self.open = true;
        self.reach(point);
        self.push(job, Event::Move(point))
    }

    /// An empty path in `area`, begun under `matrix` with the stroke `style`.
    pub(crate) const fn new(area: Area, matrix: Matrix, style: Style) -> Self {
        Self {
            area,
            bad_dash: false,
            extent: None,
            open: false,
            pen: (matrix, style),
            points: Vec::new(),
            streamed: None,
            verbs: Vec::new(),
        }
    }

    /// Streams what is held, and everything after, to a fill and a stroke at once.
    fn overflow(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        // It may only be filled: invalid dashes are solid, and warned of when it is stroked.
        self.bad_dash = stroke::invalid(&self.pen.1);
        let style = if self.bad_dash {
            Style {
                dash: Rc::new([]),
                ..self.pen.1.clone()
            }
        } else {
            self.pen.1.clone()
        };
        let pen = Pen::new(job, &style, &self.pen.0)?;
        let mut streamed = Streamed {
            fill: Filler::new(job, self.area)?,
            stroke: Stroker::new(job, pen, self.area)?,
        };
        self.each(|event| streamed.event(job, event))?;
        self.streamed = Some(RefCell::new(streamed));
        (self.points, self.verbs) = (Vec::new(), Vec::new());
        Ok(())
    }

    /// Adds `event`.
    fn push(&mut self, job: &Job<'_>, event: Event) -> Result<(), PdfError> {
        job.spend(KEEP_WORK)?;
        let (verb, count) = match event {
            Event::Close => (Verb::Close, 0),
            Event::Curve(_) => (Verb::Curve, 3),
            Event::Line(_) => (Verb::Line, 1),
            Event::Move(_) => (Verb::Move, 1),
        };
        let full = self.verbs.len() >= HELD || self.points.len().saturating_add(count) > HELD;
        if full && self.streamed.is_none() {
            self.overflow(job)?;
        }
        if let Some(streamed) = self.streamed.as_mut() {
            return streamed.get_mut().event(job, event);
        }
        room(&mut self.points, count)?;
        room(&mut self.verbs, 1)?;
        self.verbs.push(verb);
        match event {
            Event::Close => {}
            Event::Curve(points) => self.points.extend(points),
            Event::Line(point) | Event::Move(point) => self.points.push(point),
        }
        Ok(())
    }

    /// Grows the extent to `point`.
    fn reach(&mut self, point: Point) {
        let (across, down) = (point.across, point.down);
        let [left, top, right, bottom] = self.extent.unwrap_or([across, down, across, down]);
        self.extent = Some([
            left.min(across),
            top.min(down),
            right.max(across),
            bottom.max(down),
        ]);
    }

    /// What stroking it in `style` under `matrix` covers within `area`.
    pub(crate) fn stroke(
        &self,
        job: &Job<'_>,
        style: &Style,
        matrix: &Matrix,
        area: Area,
    ) -> Result<Plane<u8>, PdfError> {
        if let Some(streamed) = self.streamed.as_ref() {
            if self.bad_dash {
                job.warn("8.4.3.6", "a dash pattern is invalid")?;
            }
            let mut held = streamed.borrow_mut();
            held.stroke.end(job)?;
            let within = area.intersect(self.touched(held.stroke.spill()));
            return held.stroke.mask(job, within);
        }
        let pen = Pen::new(job, style, matrix)?;
        let within = area.intersect(self.touched(pen.spill()));
        let mut stroker = Stroker::new(job, pen, within)?;
        if !stroker.idle(job) {
            self.each(|event| stroker.event(job, event))?;
            stroker.end(job)?;
        }
        stroker.finish(job)
    }

    /// The pixels within `margin` of its points, within its area.
    fn touched(&self, margin: Fixed) -> Area {
        let found = self.extent.map(|[left, top, right, bottom]| {
            Matrix::identity().bounds([
                left.minus(margin),
                top.minus(margin),
                right.plus(margin),
                bottom.plus(margin),
            ])
        });
        found.unwrap_or_default().intersect(self.area)
    }
}

impl Streamed {
    /// Adds `event` to both.
    fn event(&mut self, job: &Job<'_>, event: Event) -> Result<(), PdfError> {
        if self.fill.cover.idle(job) {
            return Ok(());
        }
        self.fill.event(job, event)?;
        self.stroke.event(job, event)
    }
}

/// Makes room for `extra` more, never past [`HELD`].
fn room<T>(values: &mut Vec<T>, extra: usize) -> Result<(), PdfError> {
    let needed = values.len().saturating_add(extra);
    if needed <= values.capacity() {
        return Ok(());
    }
    let wanted = values
        .capacity()
        .saturating_mul(2)
        .clamp(16, HELD)
        .max(needed);
    values
        .try_reserve_exact(wanted.saturating_sub(values.len()))
        .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))
}
