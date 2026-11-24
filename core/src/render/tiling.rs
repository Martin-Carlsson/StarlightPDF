//! Tiling patterns (ISO 32000-2:2020, 8.7.3, 11.6.7).
//!
//! A small cell is drawn once, over one period of the lattice and on its grid, and then sampled at
//! the pixel centres; a large one is drawn tile by tile in a group.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;

use crate::PdfError;
use crate::overlay::Object;
use crate::render::data::{self, Resolved};
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::function::room;
use crate::render::geometry::{Area, Matrix, Point};
use crate::render::interpret::state::{Fill, Parent, State};
use crate::render::interpret::{Caches, Content, Run, nested, paint};
use crate::render::job;
use crate::render::shading::Steps;
use crate::render::surface::{Composite, Group, Paint, Sample, Shape, Surface};
use crate::render::{blend, math};

/// The bytes the cells take, and one cell at most.
const CACHE: u64 = 0x10_0000;

/// The bytes a cell takes besides its pixels.
const CELL_SIZE: u64 = 256;

/// What a period may be longer than whole pixels and still take no more: a sixteenth.
const EXCESS: i64 = 0x1000_0000;

/// What a tile must reach into an area to paint there: 1/512 pixel.
const SLIVER: Fixed = Fixed::from_bits(0x80_0000);

/// The widest cell drawn once, which keeps its planes narrow; drawn some rows at a time, it may be
/// of any height its bytes allow.
const SIDE: i64 = 4096;

/// The most numbers of tiles held.
const NUMBERS: usize = 4096;

/// The most tiles one paint draws.
const TILES: i128 = 0x1_0000;

/// The message for a pattern that cannot be used.
const UNUSABLE: &str = "a tiling pattern cannot be used";

/// One period of a pattern, drawn.
struct Cell {
    /// Rows.
    height: i32,
    /// The pixels, premultiplied.
    pixels: Vec<[u16; 4]>,
    /// Columns.
    width: i32,
}

/// What a cell's pixels depend on.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct Key {
    /// The colour of an uncoloured one.
    color: Option<[u16; 3]>,
    /// Rows.
    height: i32,
    /// How many tilings enclose it, which decides how those within it are drawn.
    level: u32,
    /// The map onto the cell.
    matrix: [Fixed; 6],
    /// The stream's object number.
    number: u32,
    /// The number of the stream whose state it begins with.
    serial: u64,
    /// Columns.
    width: i32,
}

/// The grid of a period that is drawn once.
struct Grid {
    /// Rows.
    height: i32,
    /// The map from pixels onto the cell, its box's corner on a pixel's.
    lookup: Matrix,
    /// The map from pattern space onto the cell, that corner at its origin.
    matrix: Matrix,
    /// Columns.
    width: i32,
}

/// A tiling pattern as read.
struct Pattern {
    /// Its box, the low corner first.
    bounds: [Fixed; 4],
    /// The colour of an uncoloured one.
    color: Option<[u16; 3]>,
    /// The map from its space to pixels.
    matrix: Matrix,
    /// How the stream that selected it began.
    parent: Rc<Parent>,
    /// Its resources; none: the painter's.
    resources: Option<Resolved>,
    /// The steps, above 0.
    steps: [Fixed; 2],
    /// The stream.
    stream: Resolved,
}

/// What a tile begins with: its parent's number, its stream's and its colour.
type Origin = (u64, u32, Option<[u16; 3]>);

/// A cell repeated.
struct Repeated {
    /// The cell.
    cell: Rc<Cell>,
    /// The map from pixels onto it.
    lookup: Matrix,
}

/// The tile cells held.
#[derive(Default)]
pub(crate) struct Tiles {
    /// The cells.
    cells: BTreeMap<Key, Rc<Cell>>,
    /// Bytes they take.
    held: u64,
    /// How many tilings are being drawn, one within the other.
    level: u32,
    /// The numbers of the tiles, which begin alike wherever they are.
    numbers: BTreeMap<Origin, u64>,
    /// Bytes of the cells being drawn.
    pending: u64,
}

impl Grid {
    /// The grid of `pattern` within the bytes `most`; `None` when it is drawn tile by tile.
    fn new(pattern: &Pattern, most: u64) -> Option<Self> {
        let [step_across, step_down] = pattern.steps;
        let [left, bottom, _, _] = pattern.bounds;
        let none = Fixed::default();
        let mapped = |across, down| pattern.matrix.linear(Point { across, down });
        let (first, second) = (mapped(step_across, none), mapped(none, step_down));
        let size = |point: Point| {
            let wide = i128::from(point.across.bits()).saturating_abs();
            (wide, i128::from(point.down.bits()).saturating_abs())
        };
        let ((first_wide, first_tall), (second_wide, second_tall)) = (size(first), size(second));
        let texels = |vector: Point| {
            let length = math::hypotenuse(vector).bits().saturating_sub(EXCESS);
            (length.saturating_add(0xFFFF_FFFF) >> 32_u32).max(1)
        };
        // The more horizontal side runs right, the other down, as pixels do, unless that makes
        // the cell too wide and the other way does not.
        let turned =
            first_wide.saturating_add(second_tall) < first_tall.saturating_add(second_wide);
        let (lying, standing) = if turned {
            (second, first)
        } else {
            (first, second)
        };
        let (mut row, mut column) = if texels(lying) > SIDE && texels(standing) <= SIDE {
            (standing, lying)
        } else {
            (lying, standing)
        };
        if row.across < none {
            row = math::opposite(row);
        }
        if column.down < none {
            column = math::opposite(column);
        }
        let (width, height) = (texels(row), texels(column));
        let bytes = u64::try_from(width.saturating_mul(height))
            .ok()?
            .saturating_mul(8)
            .saturating_add(CELL_SIZE);
        if width > SIDE || bytes > most {
            return None;
        }
        let part = |value: Fixed, count: i64| {
            quotient(i128::from(value.bits()), i128::from(count))
                .map(|bits| Fixed::from_bits(saturated(bits)))
        };
        let linear = [
            part(row.across, width)?,
            part(row.down, width)?,
            part(column.across, height)?,
            part(column.down, height)?,
        ];
        let corner = pattern.matrix.apply(Point {
            across: left,
            down: bottom,
        });
        let nearest =
            |value: Fixed| Fixed::from_i64(value.plus(Fixed::from_bits(0x8000_0000)).floor());
        let inverse = |across: Fixed, down: Fixed| {
            let [xx, xy, yx, yy] = linear;
            let onto = Matrix {
                terms: [xx, xy, yx, yy, across, down],
            };
            onto.inverse()
        };
        Some(Self {
            height: i32::try_from(height).ok()?,
            matrix: pattern.matrix.then(&inverse(corner.across, corner.down)?),
            lookup: inverse(nearest(corner.across), nearest(corner.down))?,
            width: i32::try_from(width).ok()?,
        })
    }
}

impl Paint for Repeated {
    fn row(
        &mut self,
        _job: &job::Job<'_>,
        row: i32,
        left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError> {
        let cell = &*self.cell;
        let (width, height) = (i64::from(cell.width), i64::from(cell.height));
        let mut steps = Steps::new(&self.lookup, row, left);
        for sample in out.iter_mut() {
            let at = steps.advance();
            let column = at.across.floor().checked_rem_euclid(width).unwrap_or(0);
            let line = at.down.floor().checked_rem_euclid(height).unwrap_or(0);
            let place = line.saturating_mul(width).saturating_add(column);
            let found = usize::try_from(place)
                .ok()
                .and_then(|index| cell.pixels.get(index))
                .copied()
                .unwrap_or_default();
            let [red, green, blue, alpha] = found;
            if alpha == 0 {
                continue;
            }
            let color = if alpha == u16::MAX {
                [red, green, blue]
            } else {
                [red, green, blue].map(|part| blend::ratio(part, alpha))
            };
            *sample = Sample {
                color,
                opacity: alpha,
                shape: u16::MAX,
            };
        }
        Ok(())
    }
}

impl Tiles {
    /// Keeps `cell` of `bytes`, room for which was made.
    fn keep(&mut self, key: Key, cell: &Rc<Cell>, bytes: u64) {
        if self.cells.insert(key, Rc::clone(cell)).is_none() {
            self.held = self.held.saturating_add(bytes);
        }
    }

    /// Makes room for `bytes` beside the cells being drawn.
    fn make_room(&mut self, bytes: u64) {
        if self.held.saturating_add(self.pending).saturating_add(bytes) > CACHE {
            self.cells.clear();
            self.held = 0;
        }
    }
}

/// The cell of `pattern` on `grid`: held, or drawn now in parts no taller than the band. In the
/// pass without pixels it is drawn at every paint, as any band may have to.
fn drawn(run: &mut Run<'_, '_>, pattern: &Pattern, grid: &Grid) -> Result<Rc<Cell>, PdfError> {
    let job = run.job;
    let measuring = job.planes.measuring();
    let key = pattern
        .stream
        .id()
        // The painter's resources may differ at each paint.
        .filter(|_| pattern.resources.is_some())
        .map(|id| Key {
            color: pattern.color,
            height: grid.height,
            level: run.caches.tiles.level,
            matrix: grid.matrix.terms,
            number: id.number,
            serial: pattern.parent.serial,
            width: grid.width,
        });
    if !measuring
        && let Some(known) = key
            .as_ref()
            .and_then(|found| run.caches.tiles.cells.get(found))
    {
        return Ok(Rc::clone(known));
    }
    let count = i64::from(grid.width).saturating_mul(i64::from(grid.height));
    let bytes = u64::try_from(count)
        .unwrap_or(u64::MAX)
        .saturating_mul(8)
        .saturating_add(CELL_SIZE);
    let mut pixels = Vec::new();
    let mut rows = grid.height;
    if !measuring {
        job.spend(bytes >> 3_u32)?;
        run.caches.tiles.make_room(bytes);
        pixels = room(usize::try_from(count).map_err(|_error| job.limit())?)?;
        rows = i32::try_from(job.area.height()).unwrap_or(rows).max(1_i32);
    }
    run.caches.tiles.pending = run.caches.tiles.pending.saturating_add(bytes);
    let result = parted(run, pattern, grid, rows, &mut pixels);
    run.caches.tiles.pending = run.caches.tiles.pending.saturating_sub(bytes);
    result?;
    let cell = Rc::new(Cell {
        height: grid.height,
        pixels,
        width: grid.width,
    });
    if !measuring && let Some(found) = key {
        run.caches.tiles.make_room(bytes);
        run.caches.tiles.keep(found, &cell, bytes);
    }
    Ok(cell)
}

/// The box of `area`, less a sliver at each side, under `inverse`: low corner, then high.
fn extent(inverse: &Matrix, area: Area) -> [Fixed; 4] {
    let edge = |pixel: i32| Fixed::from_i64(i64::from(pixel));
    let (left, right) = (edge(area.left).plus(SLIVER), edge(area.right).minus(SLIVER));
    let (top, bottom) = (edge(area.top).plus(SLIVER), edge(area.bottom).minus(SLIVER));
    inverse.extent([left, top, right, bottom])
}

/// Paints tile by tile in a group, which the shape then cuts as one object.
fn grouped(
    run: &mut Run<'_, '_>,
    pattern: &Pattern,
    shape: Shape<'_>,
    how: &Composite,
) -> Result<(), PdfError> {
    let job = run.job;
    let joined = Composite {
        alpha: how.alpha,
        alpha_is_shape: how.alpha_is_shape,
        blend: how.blend,
        clip: how.clip.within(job, shape)?,
        soft: how.soft.clone(),
    };
    let group = Group {
        area: shape.area,
        isolated: false,
        knockout: false,
    };
    run.surface.begin(job, group)?;
    let whole = run.surface.whole();
    let result = replayed(run, None, (pattern, pattern.matrix), (shape.area, whole));
    paint::ended(run, result, &joined)
}

/// The tiles along one axis whose box, `own` moved by whole steps, meets `wanted`: the first and
/// the count.
fn lined(wanted: [Fixed; 2], own: [Fixed; 2], step: Fixed) -> Option<(i64, i128)> {
    let [low, high] = wanted.map(|value| i128::from(value.bits()));
    let [start, end] = own.map(|value| i128::from(value.bits()));
    let each = i128::from(step.bits());
    let first = end
        .saturating_sub(low)
        .checked_div_euclid(each)?
        .saturating_neg();
    let last = high.saturating_sub(start).checked_div_euclid(each)?;
    let count = last.saturating_sub(first).saturating_add(1).max(0);
    Some((saturated(first), count))
}

/// The number the tiles of `pattern` begin with, so that cells within them are found again.
fn numbered(caches: &mut Caches<'_>, pattern: &Pattern) -> Option<u64> {
    // The painter's resources may differ at each paint.
    let id = pattern
        .stream
        .id()
        .filter(|_| pattern.resources.is_some())?;
    let origin = (pattern.parent.serial, id.number, pattern.color);
    if let Some(known) = caches.tiles.numbers.get(&origin) {
        return Some(*known);
    }
    let made = caches.number();
    let numbers = &mut caches.tiles.numbers;
    if numbers.len() >= NUMBERS {
        numbers.clear();
    }
    let _none: Option<u64> = numbers.insert(origin, made);
    Some(made)
}

/// Paints `shape` with the tiling pattern `pattern`, uncoloured in `color`.
pub(crate) fn pattern(
    run: &mut Run<'_, '_>,
    pattern: &Resolved,
    color: Option<[u16; 3]>,
    shape: Shape<'_>,
    how: &Composite,
) -> Result<(), PdfError> {
    let job = run.job;
    let result = read(run, pattern, color).and_then(|found| tiled(run, &found, shape, how));
    let _painted: Option<()> = job.tolerate(result, "8.7.3.1", UNUSABLE)?;
    Ok(())
}

/// The pattern `value`, in `color` when uncoloured.
fn read(run: &Run<'_, '_>, value: &Resolved, color: Option<[u16; 3]>) -> Result<Pattern, PdfError> {
    let job = run.job;
    let owner = value.value();
    let broken = || job.objects.malformed();
    if !matches!(*owner, Object::Stream(..)) {
        return Err(broken());
    }
    let kind = data::entry(job, owner, b"PaintType")?;
    let own_color = if matches!(*kind.value(), Object::Integer(1)) {
        None
    } else if matches!(*kind.value(), Object::Integer(2)) {
        Some(color.ok_or_else(broken)?)
    } else {
        return Err(broken());
    };
    let [left, bottom, right, top] = data::numbers(job, owner, b"BBox")?.ok_or_else(broken)?;
    let step = |key: &[u8]| -> Result<Fixed, PdfError> {
        let given = Fixed::from_object(data::entry(job, owner, key)?.value()).ok_or_else(broken)?;
        let none = Fixed::default();
        // The lattice is the same either way.
        let size = given.max(none.minus(given));
        if size == none {
            Err(broken())
        } else {
            Ok(size)
        }
    };
    let resources = data::entry(job, owner, b"Resources")?;
    let parent = run.state.parent.clone().ok_or_else(broken)?;
    Ok(Pattern {
        bounds: [
            left.min(right),
            bottom.min(top),
            left.max(right),
            bottom.max(top),
        ],
        color: own_color,
        matrix: data::matrix(job, owner, b"Matrix")?.then(&parent.matrix),
        parent,
        resources: (!matches!(*resources.value(), Object::Null)).then_some(resources),
        steps: [step(b"XStep")?, step(b"YStep")?],
        stream: value.clone(),
    })
}

/// Draws the tiles of `pattern` that meet `area` of `cell`, else of the surface of `run`, its space
/// mapped by `matrix`, each from the state the pattern's parent stream began with (8.7.3.1); those
/// meeting `whole`, which every pass shares, are counted against the cap.
fn replayed(
    run: &mut Run<'_, '_>,
    cell: Option<&mut Surface>,
    (pattern, matrix): (&Pattern, Matrix),
    (area, whole): (Area, Area),
) -> Result<(), PdfError> {
    let job = run.job;
    if area.is_empty() {
        return Ok(());
    }
    let Some(inverse) = matrix.inverse() else {
        return Ok(());
    };
    let [left, bottom, right, top] = pattern.bounds;
    let [step_across, step_down] = pattern.steps;
    let tiles = |pixels: Area| {
        let [low_across, low_down, high_across, high_down] = extent(&inverse, pixels);
        lined([low_across, high_across], [left, right], step_across).zip(lined(
            [low_down, high_down],
            [bottom, top],
            step_down,
        ))
    };
    // Counted alike in every pass, so that no band draws what another leaves out.
    if tiles(whole).is_some_and(|(across, down)| across.1.saturating_mul(down.1) > TILES) {
        return Err(job.capped());
    }
    let Some(((first_column, columns), (first_line, lines))) = tiles(area) else {
        return Ok(());
    };
    if columns == 0 || lines == 0 {
        return Ok(());
    }
    let parent = &*pattern.parent;
    let mut state = State::new(matrix, area);
    parent.hand_down(&mut state);
    if let Some(color) = pattern.color {
        (state.fill, state.stroke) = (Fill::Solid(color), Fill::Solid(color));
    }
    let mut inner = Run {
        // The parent's, as its colour may be a pattern that is also being painted further in.
        active: run.active.get(..parent.active).unwrap_or_default().to_vec(),
        caches: &mut *run.caches,
        // Counted by its stack: see job::NESTING.
        depth: run.depth.saturating_add(1),
        hidden: 0,
        job,
        // Not what encloses the paint: a coloured pattern may fill an uncoloured glyph.
        locked_color: pattern.color.is_some(),
        scope: Rc::clone(&run.scope),
        state,
        surface: match cell {
            Some(own) => own,
            None => &mut *run.surface,
        },
    };
    let (one, none) = (Fixed::from_i64(1), Fixed::default());
    let mut content = Content {
        bounds: Some(pattern.bounds),
        clause: "8.7.3.1",
        locked_color: pattern.color.is_some(),
        matrix: Matrix::identity(),
        resources: pattern.resources.clone(),
        serial: numbered(inner.caches, pattern),
        stream: pattern.stream.clone(),
    };
    let moved = |index: i64, step: Fixed| {
        let bits = i128::from(index).saturating_mul(i128::from(step.bits()));
        Fixed::from_bits(saturated(bits))
    };
    let mut line = first_line;
    for _line in 0..lines {
        let mut column = first_column;
        for _column in 0..columns {
            job.spend(job::NESTING_WORK)?;
            let across = moved(column, step_across);
            let down = moved(line, step_down);
            content.matrix = Matrix {
                terms: [one, none, none, one, across, down],
            };
            nested(&mut inner, &content)?;
            column = column.saturating_add(1);
        }
        line = line.saturating_add(1);
    }
    Ok(())
}

/// Draws the cell on `grid` in parts of `rows` rows, into `pixels`.
fn parted(
    run: &mut Run<'_, '_>,
    pattern: &Pattern,
    grid: &Grid,
    rows: i32,
    pixels: &mut Vec<[u16; 4]>,
) -> Result<(), PdfError> {
    let job = run.job;
    let mut top = 0_i32;
    while top < grid.height {
        let bottom = top.saturating_add(rows).min(grid.height);
        let area = Area {
            bottom,
            left: 0,
            right: grid.width,
            top,
        };
        let cell = Area {
            bottom: grid.height,
            left: 0,
            right: grid.width,
            top: 0,
        };
        let mut surface = Surface::new(job, area, None)?.spanning(cell);
        replayed(
            run,
            Some(&mut surface),
            (pattern, grid.matrix),
            (area, cell),
        )?;
        if !job.planes.measuring() {
            for row in top..bottom {
                pixels.extend_from_slice(surface.pixels(row));
            }
        }
        top = bottom;
    }
    Ok(())
}

/// Paints `shape` with `pattern`, as one object.
fn tiled(
    run: &mut Run<'_, '_>,
    pattern: &Pattern,
    shape: Shape<'_>,
    how: &Composite,
) -> Result<(), PdfError> {
    let job = run.job;
    let area = shape
        .area
        .intersect(how.clip.area)
        .intersect(run.surface.area());
    if (!job.planes.measuring() && area.is_empty()) || pattern.matrix.inverse().is_none() {
        return Ok(());
    }
    let level = run.caches.tiles.level;
    // Each level a quarter, so that cells being drawn at once stay near the cache's size.
    let most = CACHE >> level.saturating_mul(2).min(63);
    let grid = Grid::new(pattern, most);
    run.caches.tiles.level = level.saturating_add(1);
    let result = match grid {
        Some(found) => drawn(run, pattern, &found).and_then(|cell| {
            let mut repeated = Repeated {
                cell,
                lookup: found.lookup,
            };
            run.surface.draw(job, shape, &mut repeated, how)
        }),
        None => grouped(run, pattern, Shape { area, ..shape }, how),
    };
    run.caches.tiles.level = level;
    result
}
