//! Paths, shadings and patterns a mark meets: cut where it is and clipped out of it
//! (ISO 32000-2:2020, 8.5.2 to 8.5.4, 8.7).

use core::mem;

use crate::overlay::Object;
use crate::render::data::{self, Category, Resolved, Scope};
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::geometry::{Matrix, Point};
use crate::render::interpret::state::Fill;
use crate::render::interpret::{Run, content, numbers};
use crate::render::job::{self, Job};
use crate::render::math;
use crate::render::path::Curve;
use crate::render::redact::marks::{self, Mark};
use crate::render::stroke::{self, Join, Style};
use crate::syntax::write_name;
use crate::{ErrorCode, PdfError};

/// Work of cutting a segment by an area.
const CUT_WORK: u64 = 24;

/// Half the side, in default user space, of the square a clip keeps outside the marks.
const FAR: i64 = 0x10_0000;

/// A half.
const HALF: Fixed = Fixed::from_bits(1 << 31_u32);

/// The most points of a path held to be cut.
const HELD: usize = 0x8000;

/// One.
const ONE: Fixed = Fixed::from_i64(1);

/// What a path draws from the point before.
#[derive(Clone, Copy)]
enum Drawn {
    /// A curve by two control points to the last.
    Curve([Point; 3]),
    /// A line to the point.
    Line(Point),
}

/// What walking a subpath meets, in order.
#[derive(Clone, Copy)]
enum Item {
    /// It enters an area, or passes from the one it is in into another, at the point.
    Enter(usize, Point),
    /// It leaves one at the point, the length along the subpath there.
    Exit(Point, Fixed),
    /// It draws outside, continuing a curve where it is a piece of one but the first.
    Keep(Drawn, bool),
}

/// A path as it is built, held whole so that it can be cut.
#[derive(Default)]
pub(super) struct Outline {
    /// The rule of the clip it makes, `W*` being even-odd; none: it makes none.
    clip: Option<bool>,
    /// The map of its first point, and whether a later one had another.
    matrix: Option<(Matrix, bool)>,
    /// It is too long to hold, so none of it is.
    over: bool,
    /// The points held.
    points: usize,
    /// Its subpaths.
    subpaths: Vec<Subpath>,
}

/// A stroke's subpath after a cut.
struct Piece {
    /// What it draws, and whether each continues a curve.
    drawn: Vec<(Drawn, bool)>,
    /// The length along the subpath it was cut from where it starts.
    length: Fixed,
    /// Where it starts.
    start: Point,
}

/// A point of a path, in default user space and as given.
#[derive(Clone, Copy)]
struct Place {
    /// In default user space.
    page: Point,
    /// In the user space it was given in.
    user: Point,
}

/// A segment of a path, from the point before.
#[derive(Clone, Copy)]
enum Step {
    /// A curve by two control points to the last.
    Curve([Place; 3]),
    /// A line.
    Line(Place),
}

/// A subpath.
struct Subpath {
    /// It is closed.
    closed: bool,
    /// Where it starts.
    start: Place,
    /// Its segments.
    steps: Vec<Step>,
}

/// Walks a subpath through the areas.
struct Walker<'areas> {
    /// The one it is in, while in one.
    area: usize,
    /// The areas: convex and counterclockwise.
    areas: &'areas [Vec<Point>],
    /// It is in one.
    inside: bool,
    /// What it met.
    items: Vec<Item>,
    /// The length so far.
    length: Fixed,
}

impl Outline {
    /// Adds the path construction operator `name` with `operands`, given under `matrix` (8.5.2).
    pub(super) fn add(
        &mut self,
        job: &Job<'_>,
        matrix: &Matrix,
        name: &[u8],
        operands: &[Object],
    ) -> Result<(), PdfError> {
        let given: Vec<Fixed> = match name {
            b"l" | b"m" => numbers::<2>(job, operands)?.to_vec(),
            b"c" => numbers::<6>(job, operands)?.to_vec(),
            b"re" | b"v" | b"y" => numbers::<4>(job, operands)?.to_vec(),
            _ => Vec::new(),
        };
        if self.over {
            return Ok(());
        }
        self.points = self.points.saturating_add(given.len());
        if self.points > HELD {
            self.over = true;
            self.subpaths = Vec::new();
            return Ok(());
        }
        match self.matrix {
            Some((first, ref mut moved)) => *moved |= first.terms != matrix.terms,
            None => self.matrix = Some((*matrix, false)),
        }
        let place = |across: Fixed, down: Fixed| {
            let user = Point { across, down };
            Place {
                page: matrix.apply(user),
                user,
            }
        };
        match *given.as_slice() {
            [across, down] if name == b"m" => self.subpaths.push(Subpath {
                closed: false,
                start: place(across, down),
                steps: Vec::new(),
            }),
            [left, bottom, width, height] if name == b"re" => {
                let (right, top) = (left.plus(width), bottom.plus(height));
                let corners = [place(right, bottom), place(right, top), place(left, top)];
                self.subpaths.push(Subpath {
                    closed: true,
                    start: place(left, bottom),
                    steps: corners.map(Step::Line).to_vec(),
                });
            }
            _ if name == b"h" => {
                if let Some(last) = self.subpaths.last_mut() {
                    last.closed = true;
                }
            }
            _ => {
                let Some(last) = self.subpaths.last() else {
                    return Ok(());
                };
                let current = if last.closed { last.start } else { last.end() };
                let step = match *given.as_slice() {
                    [across, down] => Step::Line(place(across, down)),
                    [one, two, three, four, five, six] => {
                        Step::Curve([place(one, two), place(three, four), place(five, six)])
                    }
                    [one, two, three, four] if name == b"v" => {
                        Step::Curve([current, place(one, two), place(three, four)])
                    }
                    [one, two, three, four] => {
                        let end = place(three, four);
                        Step::Curve([place(one, two), end, end])
                    }
                    _ => return Ok(()),
                };
                if last.closed {
                    // After `h` a segment starts a subpath where the closed one started.
                    self.subpaths.push(Subpath {
                        closed: false,
                        start: current,
                        steps: Vec::new(),
                    });
                }
                if let Some(open) = self.subpaths.last_mut() {
                    open.steps.push(step);
                }
            }
        }
        Ok(())
    }

    /// `W` or, being `even`, `W*` came.
    pub(super) const fn clip(&mut self, even: bool) {
        self.clip = Some(even);
    }

    /// Whether it is held: not too long.
    pub(super) const fn holds(&self) -> bool {
        !self.over
    }
}

impl Step {
    /// Where it ends.
    const fn end(self) -> Place {
        match self {
            Self::Curve([_, _, end]) | Self::Line(end) => end,
        }
    }

    /// It as drawn in the user space it was given in.
    fn user(self) -> Drawn {
        match self {
            Self::Curve(points) => Drawn::Curve(points.map(|place| place.user)),
            Self::Line(end) => Drawn::Line(end.user),
        }
    }
}

impl Subpath {
    /// Where it ends.
    fn end(&self) -> Place {
        self.steps.last().map_or(self.start, |step| step.end())
    }
}

impl Walker<'_> {
    /// A curve from `from` by `points` (8.5.2.2): whole where no area meets its hull, else as the
    /// pieces strokes and fills draw it in default user space, each whole or as its chord;
    /// gives how many.
    fn curve(&mut self, from: Place, points: [Place; 3]) -> i64 {
        let [first, second, end] = points;
        let corners = [from.user, first.user, second.user, end.user];
        let curve = Curve::new(from.user, [first.user, second.user, end.user]);
        let count = Curve::new(from.page, [first.page, second.page, end.page]).pieces();
        if !self.meets(&corners) {
            self.out(from.user, Fixed::default());
            self.items.push(Item::Keep(
                Drawn::Curve([first.user, second.user, end.user]),
                false,
            ));
            let mut before = from.user;
            for index in 1..=count {
                let next = curve.step(index, count);
                self.length = self.length.plus(math::hypotenuse(between(before, next)));
                before = next;
            }
            return 1;
        }
        for index in 0..count {
            let piece = split(corners, &curve, index, count);
            let [start, one, two, finish] = piece;
            let continues = index > 0;
            if self.meets(&piece) {
                self.line(start, finish, continues);
            } else {
                self.out(start, Fixed::default());
                self.items
                    .push(Item::Keep(Drawn::Curve([one, two, finish]), continues));
                self.length = self.length.plus(math::hypotenuse(between(start, finish)));
            }
        }
        count
    }

    /// A line from `from` to `to`, less what is in the areas; `continues` a curve, when one of
    /// its pieces.
    fn line(&mut self, from: Point, to: Point, continues: bool) {
        let mut spans: Vec<(Fixed, Fixed, usize)> = self
            .areas
            .iter()
            .enumerate()
            .filter_map(|(area, shape)| {
                within(shape, from, to).map(|(low, high)| (low, high, area))
            })
            .collect();
        spans.sort_by_key(|span| span.0);
        let long = math::hypotenuse(between(from, to));
        let mut done = Fixed::default();
        for (low, high, area) in spans {
            if low > done {
                self.out(along(from, to, done), long.times(done));
                let end = along(from, to, low);
                self.items.push(Item::Keep(Drawn::Line(end), continues));
            }
            // Past where the one it is in ends, it goes on in this one, which holds `low` too.
            if high > done {
                if !mem::replace(&mut self.inside, true) || self.area != area {
                    self.items.push(Item::Enter(area, along(from, to, low)));
                    self.area = area;
                }
                done = high;
            }
        }
        if done < ONE {
            self.out(along(from, to, done), long.times(done));
            self.items.push(Item::Keep(Drawn::Line(to), continues));
        }
        self.length = self.length.plus(long);
    }

    /// Whether an area meets the hull of `points`.
    fn meets(&self, points: &[Point]) -> bool {
        let hull = marks::hull(points.to_vec());
        self.areas.iter().any(|area| marks::overlap(area, &hull))
    }

    /// Leaves the area it is in, if any, at `at`, `further` along the segment.
    fn out(&mut self, at: Point, further: Fixed) {
        if mem::take(&mut self.inside) {
            self.items.push(Item::Exit(at, self.length.plus(further)));
        }
    }
}

/// The point `part` of the way from `from` to `to`.
fn along(from: Point, to: Point, part: Fixed) -> Point {
    Point {
        across: from.across.plus(to.across.minus(from.across).times(part)),
        down: from.down.plus(to.down.minus(from.down).times(part)),
    }
}

/// The areas of `marks` in the user space `inverse` maps default user space to: convex and
/// counterclockwise.
fn areas(marks: &[Mark], inverse: &Matrix) -> Vec<Vec<Point>> {
    marks
        .iter()
        .flat_map(|mark| mark.areas.iter())
        .map(|area| marks::hull(area.iter().map(|point| inverse.apply(*point)).collect()))
        .filter(|area| area.len() >= 3)
        .collect()
}

/// The vector from `from` to `to`.
const fn between(from: Point, to: Point) -> Point {
    Point {
        across: to.across.minus(from.across),
        down: to.down.minus(from.down),
    }
}

/// Fails [`crate::ErrorCode::Unsupported`] where `paint` is a tiling pattern whose cell shows
/// text or images, which cutting the cells out would leave in the file (8.7.3).
pub(super) fn cell(run: &mut Run<'_, '_>, paint: &Fill) -> Result<(), PdfError> {
    let Fill::Pattern(ref found) = *paint else {
        return Ok(());
    };
    let job = run.job;
    let pattern = data::followed(job, &found.pattern)?;
    let kind = data::entry(job, pattern.value(), b"PatternType")?;
    if matches!(*kind.value(), Object::Integer(1)) && shows(run, &pattern, 0)? {
        return Err(job.unsupported());
    }
    Ok(())
}

/// Appends `subpaths` to fill or clip: where one passes through areas, that part becomes lines
/// within them from where it enters, through a point each two it passes between share, to where
/// it leaves, which changes what is inside only there (8.5.3.3).
fn chorded(
    job: &Job<'_>,
    out: &mut String,
    areas: &[Vec<Point>],
    subpaths: &[Subpath],
) -> Result<(), PdfError> {
    for subpath in subpaths {
        let items = walked(job, areas, subpath, true)?;
        let Some(first) = items
            .iter()
            .position(|item| matches!(*item, Item::Exit(..)))
        else {
            let entered: Vec<(usize, Point)> = items
                .iter()
                .filter_map(|item| match *item {
                    Item::Enter(area, at) => Some((area, at)),
                    Item::Exit(..) | Item::Keep(..) => None,
                })
                .collect();
            if entered.is_empty() {
                written(out, subpath);
                continue;
            }
            // Inside throughout: around through where it passes between areas.
            let corners: Vec<Point> = entered
                .iter()
                .zip(entered.iter().cycle().skip(1))
                .filter(|&(&(area, _), &(next, _))| area != next)
                .map(|(&(area, _), &(next, at))| shared(areas, area, next).unwrap_or(at))
                .collect();
            if !corners.is_empty() {
                polygon(out, &corners);
            }
            continue;
        };
        let (after, before) = items.split_at(first);
        let Some((&Item::Exit(start, _), rest)) = before.split_first() else {
            continue;
        };
        point(out, start, "m");
        let (mut inside, mut current) = (false, 0);
        for item in rest.iter().chain(after).chain([&Item::Exit(start, ONE)]) {
            match *item {
                Item::Enter(area, at) => {
                    if mem::replace(&mut inside, true) && area != current {
                        point(out, shared(areas, current, area).unwrap_or(at), "l");
                    }
                    current = area;
                }
                Item::Exit(at, _) => {
                    if mem::take(&mut inside) {
                        point(out, at, "l");
                    }
                }
                Item::Keep(step, _) => {
                    if !inside {
                        drawn(out, step);
                    }
                }
            }
        }
        out.push_str(" h");
    }
    Ok(())
}

/// The fraction of the way from `start` to `end`, signed distances from a side, where it is
/// crossed: 0 to 1.
fn crossing(start: i128, end: i128) -> Fixed {
    let under = start.saturating_sub(end);
    let zeros = start
        .unsigned_abs()
        .leading_zeros()
        .min(under.unsigned_abs().leading_zeros());
    // Kept to 94 bits, so that 32 more fit.
    let shift = 128_u32.saturating_sub(zeros).saturating_sub(94);
    let [above, below] = [start, under].map(|value| value.checked_shr(shift).unwrap_or_default());
    let found = quotient(above.saturating_mul(1_i128 << 32_u32), below).unwrap_or_default();
    Fixed::from_bits(saturated(found)).clamp(Fixed::default(), ONE)
}

/// Appends a dash pattern of `style` moved on by `length` (8.4.3.6).
fn dash(out: &mut String, style: &Style, length: Fixed) {
    out.push_str(" [");
    for (at, entry) in style.dash.iter().enumerate() {
        if at > 0 {
            out.push(' ');
        }
        marks::decimal(out, *entry);
    }
    out.push_str("] ");
    marks::decimal(out, style.phase.plus(length));
    out.push_str(" d");
}

/// Appends `step`.
fn drawn(out: &mut String, step: Drawn) {
    match step {
        Drawn::Curve(points) => {
            for at in points {
                for value in [at.across, at.down] {
                    out.push(' ');
                    marks::decimal(out, value);
                }
            }
            out.push_str(" c");
        }
        Drawn::Line(to) => point(out, to, "l"),
    }
}

/// Appends a clip that keeps all but `areas` of a square far around them: one even-odd clip
/// each (8.5.4).
fn excluded(out: &mut String, areas: &[Vec<Point>], inverse: &Matrix) {
    let far = Fixed::from_i64(FAR);
    let near = Fixed::default().minus(far);
    let square: Vec<Point> = marks::rectangle([near, near, far, far])
        .into_iter()
        .map(|corner| inverse.apply(corner))
        .collect();
    for area in areas {
        polygon(out, &square);
        polygon(out, area);
        out.push_str(" W* n");
    }
}

/// Whether `point` is inside `area`, not on its edge.
fn holds(area: &[Point], point: Point) -> bool {
    sides(area).all(|(corner, next)| marks::cross(corner, next, point) > 0)
}

/// What paints `outline` as `name` does, cut where the areas of `marks` are and clipped out of
/// them, then clips as it did (8.5.3, 8.5.4).
pub(super) fn path(
    run: &Run<'_, '_>,
    marks: &[Mark],
    outline: Outline,
    name: &[u8],
) -> Result<String, PdfError> {
    let job = run.job;
    if outline.over {
        return Err(job.limit());
    }
    let matrix = run.state.matrix;
    let inverse = matrix.inverse().ok_or_else(|| job.unsupported())?;
    let areas = areas(marks, &inverse);
    let mut subpaths = outline.subpaths;
    if outline
        .matrix
        .is_some_and(|(first, moved)| moved || first.terms != matrix.terms)
    {
        for subpath in &mut subpaths {
            let mapped = |place: &mut Place| place.user = inverse.apply(place.page);
            mapped(&mut subpath.start);
            for step in &mut subpath.steps {
                match *step {
                    Step::Curve(ref mut points) => points.iter_mut().for_each(mapped),
                    Step::Line(ref mut end) => mapped(end),
                }
            }
        }
    }
    if matches!(name, b"b" | b"b*" | b"s")
        && let Some(last) = subpaths.last_mut()
    {
        last.closed = true;
    }
    let mut out = String::new();
    if name != b"n" {
        out.push('q');
        excluded(&mut out, &areas, &inverse);
        let rule = match name {
            b"B" | b"b" | b"F" | b"f" => Some(" f"),
            b"B*" | b"b*" | b"f*" => Some(" f*"),
            _ => None,
        };
        if let Some(filling) = rule {
            chorded(job, &mut out, &areas, &subpaths)?;
            out.push_str(filling);
        }
        if matches!(name, b"B" | b"B*" | b"b" | b"b*" | b"S" | b"s") {
            stroked(job, &mut out, &areas, (&subpaths, &run.state.style))?;
        }
        out.push_str(" Q");
    }
    if let Some(even) = outline.clip {
        chorded(job, &mut out, &areas, &subpaths)?;
        out.push_str(if even { " W* n" } else { " W n" });
    }
    Ok(out)
}

/// The pieces of a stroke's subpath that `items`, met walking it, leave; `dashed`, a closed one
/// is also cut where it started, as its dashes start there again.
fn pieces(items: &[Item], subpath: &Subpath, dashed: bool) -> Vec<Piece> {
    let begun = |start: Point, length: Fixed| Piece {
        drawn: Vec::new(),
        length,
        start,
    };
    let mut piece = None;
    let (before, after) = if subpath.closed {
        let Some(first) = items
            .iter()
            .position(|item| matches!(*item, Item::Exit(..)))
        else {
            return Vec::new();
        };
        let (front, back) = items.split_at(first);
        (back, front)
    } else {
        piece = Some(begun(subpath.start.user, Fixed::default()));
        (items, &[][..])
    };
    let mut made = Vec::new();
    let mut walk = |item: &Item, open: &mut Option<Piece>| match *item {
        Item::Enter(..) => made.extend(open.take().filter(|found| !found.drawn.is_empty())),
        Item::Exit(at, length) => *open = Some(begun(at, length)),
        Item::Keep(step, continues) => {
            if let Some(found) = open.as_mut() {
                found.drawn.push((step, continues));
            }
        }
    };
    let ending = Item::Enter(0, subpath.start.user);
    for item in before {
        walk(item, &mut piece);
    }
    if dashed && subpath.closed && piece.is_some() {
        walk(&ending, &mut piece);
        walk(
            &Item::Exit(subpath.start.user, Fixed::default()),
            &mut piece,
        );
    }
    for item in after {
        walk(item, &mut piece);
    }
    walk(&ending, &mut piece);
    made
}

/// Appends `at` and `operator`.
fn point(out: &mut String, at: Point, operator: &str) {
    for value in [at.across, at.down] {
        out.push(' ');
        marks::decimal(out, value);
    }
    out.push(' ');
    out.push_str(operator);
}

/// Appends the closed polygon `corners`.
fn polygon(out: &mut String, corners: &[Point]) {
    for (at, corner) in corners.iter().enumerate() {
        point(out, *corner, if at == 0 { "m" } else { "l" });
    }
    out.push_str(" h");
}

/// `sh` of the shading `key`, clipped out of the areas of `marks` (8.7.4.2).
pub(super) fn shading(run: &Run<'_, '_>, marks: &[Mark], key: &[u8]) -> Result<String, PdfError> {
    let inverse = run
        .state
        .matrix
        .inverse()
        .ok_or_else(|| run.job.unsupported())?;
    let mut out = String::from("q");
    excluded(&mut out, &areas(marks, &inverse), &inverse);
    out.push(' ');
    write_name(&mut out, key);
    out.push_str(" sh Q");
    Ok(out)
}

/// A point that areas `first` and `second` share, none inside another when one is: a corner of
/// where they overlap (Sutherland-Hodgman); none where rounding parts them.
fn shared(areas: &[Vec<Point>], first: usize, second: usize) -> Option<Point> {
    let (mut kept, other) = (areas.get(first)?.clone(), areas.get(second)?);
    for (corner, next) in sides(other) {
        let mut clipped = Vec::new();
        for (start, end) in sides(&kept) {
            let (from, to) = (
                marks::cross(corner, next, start),
                marks::cross(corner, next, end),
            );
            if from >= 0 {
                clipped.push(start);
            }
            if (from < 0) != (to < 0) {
                clipped.push(along(start, end, crossing(from, to)));
            }
        }
        kept = clipped;
    }
    kept.iter()
        .copied()
        .find(|point| !areas.iter().any(|area| holds(area, *point)))
        .or_else(|| kept.first().copied())
}

/// Whether the content of `found`, a pattern or a form, shows text or images, it `depth` forms
/// deep; damaged content may.
fn shows(run: &mut Run<'_, '_>, found: &Resolved, depth: u8) -> Result<bool, PdfError> {
    let job = run.job;
    if depth >= job::NESTING {
        return Ok(true);
    }
    let resources = data::entry(job, found.value(), b"Resources")?;
    let scope = Scope::new(resources);
    let opened = data::open(job, &mut run.caches.streams, found)?;
    let mut operations = content::Operations::new(Box::new(content::Ending::new(job, opened)));
    let mut operands = Vec::new();
    loop {
        let next = match operations.next(job, &mut operands) {
            Ok(next) => next,
            Err(error) if error.code() == ErrorCode::Malformed => return Ok(true),
            Err(error) => return Err(error),
        };
        let Some(name) = next else {
            return Ok(false);
        };
        match name.bytes() {
            b"'" | b"\"" | b"BI" | b"TJ" | b"Tj" => return Ok(true),
            b"Do" => {
                let Some(key) = operands.last().and_then(data::name) else {
                    continue;
                };
                let Some(xobject) = data::named(job, &scope, Category::XObject, key)? else {
                    continue;
                };
                let kind = data::entry(job, xobject.value(), b"Subtype")?;
                let shown = match data::name(kind.value()) {
                    Some(b"Image") => true,
                    Some(b"Form") => shows(run, &xobject, depth.saturating_add(1))?,
                    Some(_) | None => false,
                };
                if shown {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }
}

/// Each side of `polygon`: a corner and the next.
fn sides(polygon: &[Point]) -> impl Iterator<Item = (Point, Point)> {
    polygon
        .iter()
        .copied()
        .zip(polygon.iter().skip(1).chain(polygon.first()).copied())
}

/// Piece `index` of `count` of `curve`, whose points are `corners`, as its own four points.
fn split(corners: [Point; 4], curve: &Curve, index: i64, count: i64) -> [Point; 4] {
    let part = |at: i64| {
        if at >= count {
            return ONE;
        }
        let found = quotient(i128::from(at) << 32_u32, i128::from(count)).unwrap_or_default();
        Fixed::from_bits(saturated(found))
    };
    let next = index.saturating_add(1);
    let (from, to) = (part(index), part(next));
    let (start, finish) = (curve.step(index, count), curve.step(next, count));
    let [zero, one, two, three] = corners;
    let [first, second, third] = [between(zero, one), between(one, two), between(two, three)];
    // A third of the direction at a part, as a quadratic of the control polygon's sides.
    let slope = |at: Fixed| along(along(first, second, at), along(second, third, at), at);
    let scaled = |vector: Point, by: Fixed| Point {
        across: vector.across.times(by),
        down: vector.down.times(by),
    };
    let span = to.minus(from);
    let leaving = scaled(slope(from), span);
    let arriving = scaled(slope(to), span);
    [
        start,
        Point {
            across: start.across.plus(leaving.across),
            down: start.down.plus(leaving.down),
        },
        Point {
            across: finish.across.minus(arriving.across),
            down: finish.down.minus(arriving.down),
        },
        finish,
    ]
}

/// Appends `subpaths` stroked in `style`, those cut as pieces (8.5.3.2): at once, but those
/// with dashes, moved on to where each starts, and those only of a curve's pieces, joined round
/// as within a curve.
fn stroked(
    job: &Job<'_>,
    out: &mut String,
    areas: &[Vec<Point>],
    (subpaths, style): (&[Subpath], &Style),
) -> Result<(), PdfError> {
    let dashed = !style.dash.is_empty() && !stroke::invalid(style);
    let join = match style.join {
        Join::Bevel => " 2 j",
        Join::Miter => " 0 j",
        Join::Round => " 1 j",
    };
    let (mut whole, mut apart) = (String::new(), String::new());
    for subpath in subpaths {
        let items = walked(job, areas, subpath, subpath.closed)?;
        if !items.iter().any(|item| matches!(*item, Item::Enter(..))) {
            written(&mut whole, subpath);
            continue;
        }
        for piece in pieces(&items, subpath, dashed) {
            let curved = piece.drawn.len() > 1
                && piece.drawn.iter().skip(1).all(|step| step.1)
                && !matches!(style.join, Join::Round);
            let into = if dashed || curved {
                if dashed {
                    dash(&mut apart, style, piece.length);
                }
                apart.push_str(if curved { " 1 j" } else { join });
                &mut apart
            } else {
                &mut whole
            };
            point(into, piece.start, "m");
            for step in piece.drawn {
                drawn(into, step.0);
            }
            if dashed || curved {
                into.push_str(" S");
            }
        }
    }
    if !whole.is_empty() {
        out.push_str(&whole);
        out.push_str(" S");
    }
    out.push_str(&apart);
    Ok(())
}

/// What walking `subpath`, and back to its start when `closing`, meets of `areas`.
fn walked(
    job: &Job<'_>,
    areas: &[Vec<Point>],
    subpath: &Subpath,
    closing: bool,
) -> Result<Vec<Item>, PdfError> {
    let mut walker = Walker {
        area: 0,
        areas,
        inside: false,
        items: Vec::new(),
        length: Fixed::default(),
    };
    let cost = CUT_WORK.saturating_mul(u64::try_from(areas.len()).unwrap_or(u64::MAX));
    let mut at = subpath.start;
    let back = closing.then_some(Step::Line(subpath.start));
    for step in subpath.steps.iter().copied().chain(back) {
        let count = match step {
            Step::Curve(points) => walker.curve(at, points),
            Step::Line(end) => {
                walker.line(at.user, end.user, false);
                1
            }
        };
        job.spend(cost.saturating_mul(u64::try_from(count).unwrap_or(u64::MAX)))?;
        at = step.end();
    }
    Ok(walker.items)
}

/// The part of the line from `from` to `to` in `area`, from 0 to 1 along it; none where it only
/// touches it.
fn within(area: &[Point], from: Point, to: Point) -> Option<(Fixed, Fixed)> {
    let (mut low, mut high) = (Fixed::default(), ONE);
    for (corner, next) in sides(area) {
        let (start, end) = (
            marks::cross(corner, next, from),
            marks::cross(corner, next, to),
        );
        match (start < 0, end < 0) {
            (true, true) => return None,
            (true, false) => low = low.max(crossing(start, end)),
            (false, true) => high = high.min(crossing(start, end)),
            (false, false) => {}
        }
    }
    if low >= high {
        return None;
    }
    let middle = along(from, to, low.plus(high.minus(low).times(HALF)));
    holds(area, middle).then_some((low, high))
}

/// Appends `subpath` as it was.
fn written(out: &mut String, subpath: &Subpath) {
    point(out, subpath.start.user, "m");
    for step in &subpath.steps {
        drawn(out, step.user());
    }
    if subpath.closed {
        out.push_str(" h");
    }
}
