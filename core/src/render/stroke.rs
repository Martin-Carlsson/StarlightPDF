//! Strokes: caps, joins, the miter limit and dashes (ISO 32000-2:2020, 8.4.3, 8.5.3.2).
//!
//! The pen is a circle in user space. A stroke is gathered as outlines, joined where the inner
//! sides meet so that overlaps do not darken edges, and needs only the first and last piece.

use alloc::rc::Rc;

use crate::PdfError;
use crate::render::coverage::{Coverage, Rule};
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::geometry::{Area, Matrix, Point};
use crate::render::job::Job;
use crate::render::math;
use crate::render::path::{Curve, Event};
use crate::render::plane::Plane;

/// Work of halving an arc.
const ARC_WORK: u64 = 4;

/// Work of a dash.
const DASH_WORK: u64 = 80;

/// How often an arc of at most a quarter circle is halved.
const HALVINGS: u8 = 8;

/// Half a pixel, the least radius of the pen.
const HALF: Fixed = Fixed::from_bits(1 << 31_u32);

/// One.
const ONE: Fixed = Fixed::from_i64(1);

/// Work of a straight piece.
const PIECE_WORK: u64 = 48;

/// The share of a curve whose chord gives a wide pen its direction at either end.
const TIP: Fixed = Fixed::from_bits(1 << 24_u32);

/// The most the inner sides are cut back at a join, in radii.
const TRIM: Fixed = Fixed::from_i64(4);

/// Two.
const TWO: Fixed = Fixed::from_i64(2);

/// A line's ends (8.4.3.3).
#[derive(Clone, Copy)]
pub(crate) enum Cap {
    /// Squared off at the end.
    Butt,
    /// A half circle.
    Round,
    /// Half the width beyond the end.
    Square,
}

/// A path's corners (8.4.3.4).
#[derive(Clone, Copy)]
pub(crate) enum Join {
    /// Cut off.
    Bevel,
    /// The edges extended.
    Miter,
    /// An arc.
    Round,
}

/// How two pieces meet.
#[derive(Clone, Copy)]
struct Bend {
    /// The inner sides are cut back to where they meet.
    cut: bool,
    /// The inner side is the left one.
    left: bool,
    /// From the corner to where the inner sides meet.
    meeting: Option<Point>,
    /// The cosine of the angle turned, and one.
    spread: Fixed,
}

/// One outline being drawn: a subpath, or a dash of it.
#[derive(Clone, Copy)]
struct Contour {
    /// Its end so far.
    at: Point,
    /// Its first piece.
    head: Head,
    /// Its last piece.
    last: Segment,
    /// It starts the subpath, so closing joins to it.
    leading: bool,
    /// Where both sides of the last piece start; none: it is the first.
    starts: Option<[Point; 2]>,
}

/// The dashes (8.4.3.6).
#[derive(Clone)]
struct Dashes {
    /// The entries before the pattern repeats: twice the array when its count is odd.
    cycle: usize,
    /// The lengths.
    entries: Rc<[Fixed]>,
    /// Where each subpath starts in them.
    first: Phase,
    /// The map from pixels to user space's vectors.
    inverse: Matrix,
    /// The length that repeats.
    period: i128,
}

/// A contour's first piece, kept until it is known what it starts with.
#[derive(Clone, Copy)]
struct Head {
    /// Where it starts.
    at: Point,
    /// Where both its sides end.
    ends: [Point; 2],
    /// The piece.
    segment: Segment,
}

/// A stroke's pen.
pub(crate) struct Pen {
    /// The map from pixels to radii.
    back: Matrix,
    /// The ends.
    cap: Cap,
    /// The dashes; none: solid.
    dashes: Option<Dashes>,
    /// The map from radii to pixels.
    forward: Matrix,
    /// The corners.
    join: Join,
    /// The square of the miter limit.
    miter: Fixed,
    /// The longest radius, in pixels.
    radius: Fixed,
    /// How far beyond its points a stroke reaches, in pixels.
    spill: Fixed,
}

/// A place in the dashes.
#[derive(Clone, Copy, Default)]
struct Phase {
    /// The entry; an even one paints.
    index: usize,
    /// What is left of it.
    left: i128,
}

/// A point of the pen's circle.
#[derive(Clone, Copy)]
struct Rim {
    /// In pixels.
    place: Point,
    /// In radii from the centre.
    radial: Point,
}

/// A straight piece.
#[derive(Clone, Copy)]
struct Segment {
    /// Its direction, one radius long.
    direction: Point,
    /// Its length, in radii.
    length: Fixed,
    /// The radius to its side, in pixels.
    normal: Point,
}

/// Turns segments into the edges of their stroke.
pub(crate) struct Stroker {
    /// The outline being drawn.
    contour: Option<Contour>,
    /// What is covered.
    cover: Coverage,
    /// The current point.
    current: Point,
    /// The subpath has more than its first point.
    drawn: bool,
    /// The first dash, which closing joins to.
    leading: Option<Head>,
    /// The subpath has length.
    lengthy: bool,
    /// The pen.
    pen: Pen,
    /// Where the subpath is in the dashes.
    phase: Phase,
    /// Where the subpath started.
    start: Point,
}

/// How lines are stroked, in user space.
#[derive(Clone)]
pub(crate) struct Style {
    /// The ends.
    pub cap: Cap,
    /// Dashes and gaps; none: solid.
    pub dash: Rc<[Fixed]>,
    /// The corners.
    pub join: Join,
    /// The miter limit.
    pub miter: Fixed,
    /// Where in the dashes a line starts.
    pub phase: Fixed,
    /// The width.
    pub width: Fixed,
}

impl Dashes {
    /// Entry `index` of the cycle.
    fn entry(&self, index: usize) -> i128 {
        let count = self.entries.len();
        let found = self.entries.get(index.checked_sub(count).unwrap_or(index));
        i128::from(found.copied().unwrap_or_default().bits())
    }

    /// The entry after `phase`.
    fn next(&self, phase: Phase) -> Phase {
        let after = phase.index.saturating_add(1);
        let index = if after < self.cycle { after } else { 0 };
        Phase {
            index,
            left: self.entry(index),
        }
    }

    /// The dashes of `style`, `inverse` giving lengths in user space; none: solid.
    fn read(
        job: &Job<'_>,
        style: &Style,
        inverse: Option<Matrix>,
    ) -> Result<Option<Self>, PdfError> {
        let entries = &style.dash;
        if entries.is_empty() {
            return Ok(None);
        }
        job.spend(u64::try_from(entries.len()).unwrap_or(u64::MAX))?;
        if invalid(style) {
            job.warn("8.4.3.6", "a dash pattern is invalid")?;
            return Ok(None);
        }
        let total = entries.iter().fold(0_i128, |sum, entry| {
            sum.saturating_add(i128::from(entry.bits()))
        });
        // Under a singular map a stroke is a solid hairline.
        let Some(linear) = inverse else {
            return Ok(None);
        };
        let repeats = if entries.len() & 1 == 1 { 2_u8 } else { 1_u8 };
        let mut dashes = Self {
            cycle: entries.len().saturating_mul(usize::from(repeats)),
            entries: Rc::clone(entries),
            first: Phase::default(),
            inverse: linear,
            period: total.saturating_mul(i128::from(repeats)),
        };
        let mut past = i128::from(style.phase.bits())
            .checked_rem_euclid(dashes.period)
            .unwrap_or(0);
        let mut first = Phase {
            index: 0,
            left: dashes.entry(0),
        };
        while past > 0 && past >= first.left {
            job.spend(1)?;
            past = past.saturating_sub(first.left);
            first = dashes.next(first);
        }
        first.left = first.left.saturating_sub(past);
        dashes.first = first;
        Ok(Some(dashes))
    }
}

impl Pen {
    /// The margin outside which a piece without a join covers nothing.
    fn margin(&self) -> Fixed {
        self.radius.times(TWO).plus(TWO)
    }

    /// The pen of `style` under `matrix`, at least a pixel wide.
    pub(crate) fn new(job: &Job<'_>, style: &Style, matrix: &Matrix) -> Result<Self, PdfError> {
        let none = Fixed::default();
        let [aa, bb, cc, dd, _, _] = matrix.terms;
        let linear = Matrix {
            terms: [aa, bb, cc, dd, none, none],
        };
        let inverse = linear.inverse();
        let half = Fixed::from_bits(style.width.bits().saturating_abs()).times(HALF);
        let hairline = Matrix {
            terms: [HALF, none, none, HALF, none, none],
        };
        let (wide, longest) = if inverse.is_some() {
            widened(linear.terms.map(|term| term.times(half))).unwrap_or((hairline, HALF))
        } else {
            (hairline, HALF)
        };
        let twice = Matrix {
            terms: [TWO, none, none, TWO, none, none],
        };
        let (forward, back, radius) = wide
            .inverse()
            .map_or((hairline, twice, HALF), |back| (wide, back, longest));
        let farthest = match style.join {
            Join::Miter => style.miter.max(TWO),
            Join::Bevel | Join::Round => TWO,
        };
        let limit = style.miter.max(none);
        Ok(Self {
            back,
            cap: style.cap,
            dashes: Dashes::read(job, style, inverse)?,
            forward,
            join: style.join,
            miter: limit.times(limit),
            radius,
            spill: radius.times(farthest).plus(TWO),
        })
    }

    /// The piece along `vector`; none without length.
    fn segment(&self, vector: Point) -> Option<Segment> {
        let direction = math::unit(self.back.linear(math::scaled(vector)))?;
        Some(Segment {
            direction,
            length: math::hypotenuse(self.back.linear(vector)),
            normal: self.forward.linear(math::turned(direction)),
        })
    }

    /// How far beyond its points a stroke reaches.
    pub(crate) const fn spill(&self) -> Fixed {
        self.spill
    }
}

impl Stroker {
    /// The arc about `centre` from `from` to `to`, at most a quarter circle.
    fn arc(
        &mut self,
        job: &Job<'_>,
        centre: Point,
        rims: [Rim; 2],
        halvings: u8,
    ) -> Result<(), PdfError> {
        let [from, to] = rims;
        let chord = between(from.radial, to.radial);
        // The arc is within an eighth of a pixel of a chord of which this is at most 1; a quarter
        // would make an octagon of a thin pen.
        let flat = dot(chord, chord).times(self.pen.radius) <= ONE;
        // At most a quarter circle, the arc strays from its chord by at most half the chord turned
        // under the pen.
        let none = Fixed::default();
        let size = |vector: Point| {
            let [across, down] =
                [vector.across, vector.down].map(|part| part.max(none.minus(part)));
            across.plus(down)
        };
        let bulge = self.pen.forward.linear(math::turned(chord));
        let reach = size(between(from.place, to.place))
            .plus(size(bulge))
            .plus(TWO);
        if !flat
            && halvings > 0
            && !self.cover.misses(&[from.place, to.place], reach)
            && let Some(radial) = math::unit(shifted(from.radial, to.radial))
        {
            job.spend(ARC_WORK)?;
            let middle = Rim {
                place: shifted(centre, self.pen.forward.linear(radial)),
                radial,
            };
            let fewer = halvings.saturating_sub(1);
            self.arc(job, centre, [from, middle], fewer)?;
            return self.arc(job, centre, [middle, to], fewer);
        }
        self.cover.edge(job, from.place, to.place)
    }

    /// How `pair` meets.
    fn bend(&self, pair: [Segment; 2]) -> Bend {
        let [before, after] = pair;
        let (one, two) = (before.direction, after.direction);
        let none = Fixed::default();
        let turn = one.across.times(two.down).minus(one.down.times(two.across));
        let spread = ONE.plus(dot(one, two));
        let tangent = (spread > none)
            .then_some(spread)
            .and_then(|divisor| turn.max(none.minus(turn)).over(divisor));
        let left = turn > none;
        let side = math::turned(one);
        let inward = if left { side } else { math::opposite(side) };
        let meeting = tangent.map(|back| {
            self.pen.forward.linear(Point {
                across: inward.across.minus(one.across.times(back)),
                down: inward.down.minus(one.down.times(back)),
            })
        });
        let cut = tangent.is_some_and(|back| {
            let twice = back.plus(back);
            back <= TRIM && twice <= before.length && twice <= after.length
        });
        Bend {
            cut,
            left,
            meeting,
            spread,
        }
    }

    /// Both sides of a piece from `starts` to `ends`.
    fn body(
        &mut self,
        job: &Job<'_>,
        starts: [Point; 2],
        ends: [Point; 2],
    ) -> Result<(), PdfError> {
        let ([left, right], [left_end, right_end]) = (starts, ends);
        self.cover.edge(job, left, left_end)?;
        self.cover.edge(job, right_end, right)
    }

    /// The cap at `at` of a piece that ends there along `segment`; gives both its sides there.
    fn cap(&mut self, job: &Job<'_>, at: Point, segment: Segment) -> Result<[Point; 2], PdfError> {
        let ends = sides(at, segment.normal);
        let [left, right] = ends;
        match self.pen.cap {
            Cap::Butt => self.cover.edge(job, left, right)?,
            Cap::Round => {
                let side = math::turned(segment.direction);
                let rims = [
                    Rim {
                        place: left,
                        radial: side,
                    },
                    Rim {
                        place: right,
                        radial: math::opposite(side),
                    },
                ];
                self.halved(job, at, rims, segment.direction)?;
            }
            Cap::Square => {
                let out = self.pen.forward.linear(segment.direction);
                let (left_out, right_out) = (shifted(left, out), shifted(right, out));
                self.cover.edge(job, left, left_out)?;
                self.cover.edge(job, left_out, right_out)?;
                self.cover.edge(job, right_out, right)?;
            }
        }
        Ok(ends)
    }

    /// Joins the subpath's end to its start (8.5.2.1).
    fn close(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        self.line(job, self.start, self.pen.join)?;
        if let Some(contour) = self.contour {
            let first = if contour.leading {
                Some(contour.head)
            } else {
                self.leading
            };
            if let Some(target) = first {
                let pair = [contour.last, target.segment];
                let [ends, starts] = self.corner(job, contour.at, pair, self.pen.join)?;
                let head = self.tail(job, contour, ends)?;
                if contour.leading {
                    self.body(job, starts, head.ends)?;
                } else {
                    self.open_head(job, head)?;
                    self.body(job, starts, target.ends)?;
                }
                (self.contour, self.leading) = (None, None);
            }
        }
        self.end(job)?;
        self.current = self.start;
        Ok(())
    }

    /// The join at `at` of the pieces `pair`; gives where the first's sides end and the second's
    /// start.
    fn corner(
        &mut self,
        job: &Job<'_>,
        at: Point,
        pair: [Segment; 2],
        join: Join,
    ) -> Result<[[Point; 2]; 2], PdfError> {
        let [before, after] = pair;
        let bend = self.bend(pair);
        let [mut left_end, mut right_end] = sides(at, before.normal);
        let [mut left_start, mut right_start] = sides(at, after.normal);
        // Always in the direction of the outline.
        let (one, two) = (
            math::turned(before.direction),
            math::turned(after.direction),
        );
        let rims = if bend.left {
            [
                Rim {
                    place: right_start,
                    radial: math::opposite(two),
                },
                Rim {
                    place: right_end,
                    radial: math::opposite(one),
                },
            ]
        } else {
            [
                Rim {
                    place: left_end,
                    radial: one,
                },
                Rim {
                    place: left_start,
                    radial: two,
                },
            ]
        };
        self.outside(job, at, (pair, bend, join), rims)?;
        if bend.cut
            && let Some(offset) = bend.meeting
        {
            let inner = shifted(at, offset);
            if bend.left {
                (left_end, left_start) = (inner, inner);
            } else {
                (right_end, right_start) = (inner, inner);
            }
        } else {
            let (into, out) = if bend.left {
                (left_end, left_start)
            } else {
                (right_start, right_end)
            };
            self.cover.edge(job, into, at)?;
            self.cover.edge(job, at, out)?;
        }
        Ok([[left_end, right_end], [left_start, right_start]])
    }

    /// The curve by `points`: round within, the pen's join where it starts.
    fn curve(&mut self, job: &Job<'_>, points: [Point; 3]) -> Result<(), PdfError> {
        let curve = Curve::new(self.current, points);
        let count = curve.pieces();
        let tipped = self.pen.radius > TWO && count < i64::from(u8::MAX);
        let mut join = self.pen.join;
        for index in 1..=count {
            let near_start = (tipped && index == 1).then(|| curve.at(TIP));
            let near_end = (tipped && index == count).then(|| curve.at(ONE.minus(TIP)));
            let step = Some(curve.step(index, count));
            for point in [near_start, near_end, step].into_iter().flatten() {
                let moves = point.across != self.current.across || point.down != self.current.down;
                self.line(job, point, join)?;
                if moves {
                    join = Join::Round;
                }
            }
        }
        Ok(())
    }

    /// The dashes along a line from `from` to `to`, which is `segment`.
    fn dashed(
        &mut self,
        job: &Job<'_>,
        line: ([Point; 2], Segment, Join),
        dashes: &Dashes,
    ) -> Result<(), PdfError> {
        let ([from, to], segment, join) = line;
        let length = i128::from(math::hypotenuse(dashes.inverse.linear(between(from, to))).bits());
        let seen = self.seen([from, to], length);
        let mut done = 0_i128;
        loop {
            job.spend(DASH_WORK)?;
            let rest = length.saturating_sub(done);
            let through = self.phase.left > rest;
            let until = if through {
                length
            } else {
                done.saturating_add(self.phase.left)
            };
            let shows = done == 0 || through || (until >= seen[0] && done <= seen[1]);
            if self.phase.index & 1 == 0 && shows {
                let start = if done == 0 {
                    from
                } else {
                    along([from, to], done, length)
                };
                let end = if through {
                    to
                } else {
                    along([from, to], until, length)
                };
                let long =
                    i128::from(segment.length.bits()).saturating_mul(until.saturating_sub(done));
                let piece = Segment {
                    length: quotient(long, length)
                        .map_or(segment.length, |bits| Fixed::from_bits(saturated(bits))),
                    ..segment
                };
                self.extend(job, [start, end], (piece, join, done == 0))?;
                if !through {
                    self.end_dash(job)?;
                }
            }
            if through {
                self.phase.left = self.phase.left.saturating_sub(rest);
                return Ok(());
            }
            self.phase = dashes.next(self.phase);
            done = until;
            // Whole periods out of sight change nothing.
            let target = if done < seen[0] {
                seen[0].min(length)
            } else if done > seen[1] {
                length
            } else {
                done
            };
            if let Some(periods) = target.saturating_sub(done).checked_div(dashes.period) {
                done = done.saturating_add(periods.saturating_mul(dashes.period));
            }
            let point = self.phase.index & 1 == 0 && self.phase.left == 0;
            if done >= length && !point {
                return Ok(());
            }
        }
    }

    /// Ends the open subpath: its caps, or a dot for one without length.
    pub(crate) fn end(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        if self.drawn && !self.lengthy && matches!(self.pen.cap, Cap::Round) {
            let direction = Point {
                across: ONE,
                down: Fixed::default(),
            };
            let segment = Segment {
                direction,
                length: Fixed::default(),
                normal: self.pen.forward.linear(math::turned(direction)),
            };
            let ends = self.cap(job, self.start, segment)?;
            let head = Head {
                at: self.start,
                ends,
                segment,
            };
            self.open_head(job, head)?;
        }
        self.end_dash(job)?;
        if let Some(head) = self.leading.take() {
            self.open_head(job, head)?;
        }
        (self.drawn, self.lengthy) = (false, false);
        self.phase = self
            .pen
            .dashes
            .as_ref()
            .map_or_else(Phase::default, |dashes| dashes.first);
        Ok(())
    }

    /// Ends the outline being drawn with a cap.
    fn end_dash(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        let Some(contour) = self.contour.take() else {
            return Ok(());
        };
        let ends = self.cap(job, contour.at, contour.last)?;
        let head = self.tail(job, contour, ends)?;
        if contour.leading {
            self.leading = Some(head);
            Ok(())
        } else {
            self.open_head(job, head)
        }
    }

    /// Adds `event`.
    pub(crate) fn event(&mut self, job: &Job<'_>, event: Event) -> Result<(), PdfError> {
        match event {
            Event::Close => {
                self.drawn = true;
                self.close(job)
            }
            Event::Curve(points) => {
                self.drawn = true;
                self.curve(job, points)
            }
            Event::Line(point) => {
                self.drawn = true;
                self.line(job, point, self.pen.join)
            }
            Event::Move(point) => {
                self.end(job)?;
                (self.current, self.start) = (point, point);
                Ok(())
            }
        }
    }

    /// Continues the outline with a piece between `ends`, or begins one.
    fn extend(
        &mut self,
        job: &Job<'_>,
        ends: [Point; 2],
        piece: (Segment, Join, bool),
    ) -> Result<(), PdfError> {
        let ([from, to], (segment, join, first)) = (ends, piece);
        self.contour = Some(match self.contour {
            Some(mut contour) => {
                let pair = [contour.last, segment];
                let [finished, starts] = self.corner(job, contour.at, pair, join)?;
                match contour.starts {
                    Some(held) => self.body(job, held, finished)?,
                    None => contour.head.ends = finished,
                }
                Contour {
                    at: to,
                    last: segment,
                    starts: Some(starts),
                    ..contour
                }
            }
            None => Contour {
                at: to,
                head: Head {
                    at: from,
                    ends: [to, to],
                    segment,
                },
                last: segment,
                leading: first && !self.lengthy && self.leading.is_none(),
                starts: None,
            },
        });
        Ok(())
    }

    /// What it covers.
    pub(crate) fn finish(self, job: &Job<'_>) -> Result<Plane<u8>, PdfError> {
        self.cover.finish(job, Rule::NonZero)
    }

    /// The arc about `centre` between `rims` by way of `middle`, at most half a circle.
    fn halved(
        &mut self,
        job: &Job<'_>,
        centre: Point,
        rims: [Rim; 2],
        middle: Point,
    ) -> Result<(), PdfError> {
        let [from, to] = rims;
        let top = Rim {
            place: shifted(centre, self.pen.forward.linear(middle)),
            radial: middle,
        };
        self.arc(job, centre, [from, top], HALVINGS)?;
        self.arc(job, centre, [top, to], HALVINGS)
    }

    /// Whether nothing is to be gathered.
    pub(crate) const fn idle(&self, job: &Job<'_>) -> bool {
        self.cover.idle(job)
    }

    /// A line to `point`, joined as `join` says.
    fn line(&mut self, job: &Job<'_>, point: Point, join: Join) -> Result<(), PdfError> {
        job.spend(PIECE_WORK)?;
        let Some(segment) = self.pen.segment(between(self.current, point)) else {
            return Ok(());
        };
        let ends = [self.current, point];
        if let Some(dashes) = self.pen.dashes.clone() {
            self.dashed(job, (ends, segment, join), &dashes)?;
        } else {
            self.extend(job, ends, (segment, join, true))?;
        }
        (self.current, self.lengthy) = (point, true);
        Ok(())
    }

    /// The covered share of `area`.
    pub(crate) fn mask(&self, job: &Job<'_>, area: Area) -> Result<Plane<u8>, PdfError> {
        self.cover.mask(job, Rule::NonZero, area)
    }

    /// Nothing stroked with `pen` in `area`.
    pub(crate) fn new(job: &Job<'_>, pen: Pen, area: Area) -> Result<Self, PdfError> {
        Ok(Self {
            contour: None,
            cover: Coverage::new(job, area)?,
            current: Point::default(),
            drawn: false,
            leading: None,
            lengthy: false,
            phase: pen
                .dashes
                .as_ref()
                .map_or_else(Phase::default, |dashes| dashes.first),
            pen,
            start: Point::default(),
        })
    }

    /// The cap and sides that begin a contour with `head`.
    fn open_head(&mut self, job: &Job<'_>, head: Head) -> Result<(), PdfError> {
        let reversed = Segment {
            direction: math::opposite(head.segment.direction),
            normal: math::opposite(head.segment.normal),
            ..head.segment
        };
        let [right, left] = self.cap(job, head.at, reversed)?;
        self.body(job, [left, right], head.ends)
    }

    /// The outer side of a join at `at`, between `rims`.
    fn outside(
        &mut self,
        job: &Job<'_>,
        at: Point,
        joined: ([Segment; 2], Bend, Join),
        rims: [Rim; 2],
    ) -> Result<(), PdfError> {
        let ([before, after], bend, join) = joined;
        let [from, to] = rims;
        match join {
            Join::Bevel => {}
            Join::Miter => {
                if let Some(offset) = bend.meeting
                    && bend.spread.times(self.pen.miter) >= TWO
                {
                    let tip = shifted(at, math::opposite(offset));
                    self.cover.edge(job, from.place, tip)?;
                    return self.cover.edge(job, tip, to.place);
                }
            }
            Join::Round => {
                // Past a quarter turn the radii's bisector loses bits, the directions' keeps them.
                if bend.spread < ONE
                    && let Some(middle) = math::unit(between(after.direction, before.direction))
                {
                    return self.halved(job, at, rims, middle);
                }
                return self.arc(job, at, rims, HALVINGS);
            }
        }
        self.cover.edge(job, from.place, to.place)
    }

    /// The part of a line's `length` where what is drawn may reach the area, whole periods aside.
    fn seen(&self, ends: [Point; 2], length: i128) -> [i128; 2] {
        let [from, to] = ends;
        let area = self.cover.area();
        let margin = i128::from(self.pen.margin().bits());
        let (mut low, mut high) = (0_i128, length);
        for (start, end, before, after) in [
            (from.across, to.across, area.left, area.right),
            (from.down, to.down, area.top, area.bottom),
        ] {
            let [origin, finish] = [start, end].map(|value| i128::from(value.bits()));
            let least = (i128::from(before) << 32_u32).saturating_sub(margin);
            let most = (i128::from(after) << 32_u32).saturating_add(margin);
            let run = finish.saturating_sub(origin);
            let reached =
                |side: i128| quotient(side.saturating_sub(origin).saturating_mul(length), run);
            if let (Some(one), Some(two)) = (reached(least), reached(most)) {
                low = low.max(one.min(two));
                high = high.min(one.max(two));
            } else if origin < least || origin > most {
                return [length.saturating_add(1), -1];
            } else {
                // Along this axis all of it may reach.
            }
        }
        if low > high {
            [length.saturating_add(1), -1]
        } else {
            [low, high]
        }
    }

    /// How far beyond its points it reaches.
    pub(crate) const fn spill(&self) -> Fixed {
        self.pen.spill()
    }

    /// Ends `contour`'s last piece at `ends`; gives its first.
    fn tail(
        &mut self,
        job: &Job<'_>,
        contour: Contour,
        ends: [Point; 2],
    ) -> Result<Head, PdfError> {
        match contour.starts {
            Some(starts) => {
                self.body(job, starts, ends)?;
                Ok(contour.head)
            }
            None => Ok(Head {
                ends,
                ..contour.head
            }),
        }
    }
}

/// The point `done` of `length` along the line between `ends`.
fn along(ends: [Point; 2], done: i128, length: i128) -> Point {
    let [from, to] = ends;
    let part = |start: Fixed, end: Fixed| {
        let origin = i128::from(start.bits());
        let run = i128::from(end.bits()).saturating_sub(origin);
        quotient(run.saturating_mul(done), length).map_or(end, |moved| {
            Fixed::from_bits(saturated(origin.saturating_add(moved)))
        })
    };
    Point {
        across: part(from.across, to.across),
        down: part(from.down, to.down),
    }
}

/// The vector from `from` to `to`.
const fn between(from: Point, to: Point) -> Point {
    Point {
        across: to.across.minus(from.across),
        down: to.down.minus(from.down),
    }
}

/// How far `one` goes along `two`.
fn dot(one: Point, two: Point) -> Fixed {
    one.across.times(two.across).plus(one.down.times(two.down))
}

/// `point` moved by `vector`.
const fn shifted(point: Point, vector: Point) -> Point {
    Point {
        across: point.across.plus(vector.across),
        down: point.down.plus(vector.down),
    }
}

/// Both sides at `at` of a line with the radius `normal` to its side.
const fn sides(at: Point, normal: Point) -> [Point; 2] {
    [shifted(at, normal), shifted(at, math::opposite(normal))]
}

/// The pen mapping `terms`, widened to a pixel across, and its longest radius; none: thinner
/// every way.
fn widened(terms: [Fixed; 6]) -> Option<(Matrix, Fixed)> {
    let [aa, bb, cc, dd, _, _] = terms;
    let [half_a, half_b, half_c, half_d] = [aa, bb, cc, dd].map(|term| term.times(HALF));
    // A rotation and a reflection whose sizes add up to the long radius and differ by the short.
    let (rotated, mirrored) = (half_a.plus(half_d), half_a.minus(half_d));
    let (shear, spin) = (half_b.plus(half_c), half_b.minus(half_c));
    let rotation = math::hypotenuse(Point {
        across: rotated,
        down: spin,
    });
    let reflection = math::hypotenuse(Point {
        across: mirrored,
        down: shear,
    });
    let longest = rotation.plus(reflection);
    if longest <= HALF {
        return None;
    }
    let none = Fixed::default();
    if rotation.minus(reflection) >= HALF || reflection.minus(rotation) >= HALF {
        return Some((
            Matrix {
                terms: [aa, bb, cc, dd, none, none],
            },
            longest,
        ));
    }
    let (wide, narrow) = (
        longest.plus(HALF).times(HALF),
        longest.minus(HALF).times(HALF),
    );
    let (turns, flips) = if rotation >= reflection {
        (wide, narrow)
    } else {
        (narrow, wide)
    };
    let scale = |part: Fixed, size: Fixed, to: Fixed| part.over(size).unwrap_or(none).times(to);
    let (even, odd) = (
        scale(rotated, rotation, turns),
        scale(spin, rotation, turns),
    );
    let (level, sloped) = (
        scale(mirrored, reflection, flips),
        scale(shear, reflection, flips),
    );
    let widest = [
        even.plus(level),
        odd.plus(sloped),
        sloped.minus(odd),
        even.minus(level),
        none,
        none,
    ];
    Some((Matrix { terms: widest }, longest))
}

/// Whether the dashes of `style` are invalid: some below 0, or none above (8.4.3.6).
pub(crate) fn invalid(style: &Style) -> bool {
    let entries = &style.dash;
    let total = entries.iter().fold(0_i128, |sum, entry| {
        sum.saturating_add(i128::from(entry.bits()))
    });
    !entries.is_empty() && (total <= 0 || entries.iter().any(|entry| *entry < Fixed::default()))
}
