//! Fonts: from codes to glyphs and advances (ISO 32000-2:2020, 9.6 to 9.9).

use alloc::collections::BTreeMap;
use alloc::rc::Rc;

use crate::font_program::encoding::{self, Base};
use crate::font_program::{Fault, Outline, Program, Table};
use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::pull::number;
use crate::render::data::{self, Resolved};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Area, Matrix, Point};
use crate::render::interpret::Run;
use crate::render::job::Job;
use crate::render::lent::{Clear, Yielding};
use crate::render::path::Path;
use crate::render::text::cmap::{Cmap, RANGE, Ranges};
use crate::render::text::type3::Procedures;
use crate::{ErrorCode, PdfError};

/// The most items of `Differences` that mean something: each code once, with a number before it.
const DIFFERENCES: usize = 512;

/// The longest name a reader takes (Annex C).
const NAME: usize = 127;

/// The most bytes of a CID to glyph map: two a CID.
const CID_MAP: u64 = 0x2_0000;

/// The most bytes of fonts alive at once, as of a codec's data: 256 MiB.
const HELD: u64 = 0x1000_0000;

/// The most entries of a cache, so that finding one stays cheap.
const ENTRIES: usize = 256;

/// The last CID.
const LAST_CID: i64 = 0xFFFF;

/// The bytes of outlines held.
const OUTLINES: u64 = 0x8_0000;

/// The bytes a font's dictionary gives at least.
const TABLE_SIZE: u64 = 0x1100;

/// The thousand of glyph space (9.2.4).
pub(crate) const THOUSAND: Fixed = Fixed::from_i64(1000);

/// What a render keeps of fonts.
#[derive(Default)]
pub(crate) struct Cache {
    /// Outlines by program and glyph.
    outlines: BTreeMap<(u32, u32), Rc<Traced>>,
    /// Their bytes.
    outlines_held: u64,
    /// Programs by stream; none: it cannot be used.
    programs: Yielding<Kept<Option<Rc<Embedded>>>>,
    /// What dictionaries give, by theirs.
    tables: Yielding<Kept<Rc<Tables>>>,
}

/// A code and what the font says of it.
#[derive(Clone, Copy)]
pub(crate) struct Code {
    /// The advance, in the font's units.
    pub advance: Fixed,
    /// Its bytes.
    pub length: usize,
    /// The position vector of vertical writing.
    pub origin: [Fixed; 2],
    /// How far up the advance goes, in the font's units: a Type 3 font's may.
    pub rise: Fixed,
    /// The code, or its CID.
    pub selector: u32,
    /// It is the single byte 32, which `Tw` widens.
    pub spaced: bool,
}

/// What a Type 0 font gives (9.7).
struct Composite {
    /// The `CMap`; none: a predefined one other than Identity.
    cmap: Option<Cmap>,
    /// `DW`.
    default: Fixed,
    /// `DW2`.
    downward: [Fixed; 2],
    /// `W2`.
    heights: Ranges<[Fixed; 3]>,
    /// `CIDToGIDMap`; none: Identity.
    map: Option<Vec<u8>>,
    /// `W`.
    widths: Ranges<[Fixed; 1]>,
}

/// An embedded program.
struct Embedded {
    /// Its stream.
    number: Option<u32>,
    /// The program.
    program: Program,
}

/// A simple font's encoding (9.6.5).
pub(super) struct Encoding {
    /// The encoding the differences are from.
    base: Option<Base>,
    /// The differences, by code.
    changed: Vec<Option<Vec<u8>>>,
    /// There is an entry.
    present: bool,
}

/// A font as text uses it.
pub(crate) struct Font {
    /// Its program; none is embedded, or it cannot be used.
    program: Option<Rc<Embedded>>,
    /// What its dictionary gives.
    tables: Rc<Tables>,
}

/// Values by object number, the least recently used first.
struct Kept<T> {
    /// The values and their bytes.
    entries: Vec<(u32, T, u64)>,
    /// The bytes in all.
    held: u64,
}

/// What a font dictionary gives, held without the program.
#[derive(Default)]
struct Tables {
    /// Each code of a simple font whose glyph's name marks nothing.
    blanks: Vec<bool>,
    /// Its part as a Type 0 font.
    composite: Option<Composite>,
    /// `FontDescriptor` as given, read only for `Font::extent` (9.8).
    descriptor: Option<Object>,
    /// A program is embedded.
    embedded: bool,
    /// That program's stream.
    file: Option<ObjectId>,
    /// Each code's glyph; none is blank by its name, or a name that cannot be resolved (9.6.5.4).
    glyphs: Vec<Option<u32>>,
    /// Its advances are known.
    measured: bool,
    /// Its part as a Type 3 font.
    procedures: Option<Procedures>,
    /// Each code's advance up, where a Type 3 font's `FontMatrix` turns it.
    rises: Vec<Fixed>,
    /// Each code's advance.
    widths: Vec<Fixed>,
}

/// An outline, in font units.
#[derive(Default)]
pub(crate) struct Traced {
    /// Left, bottom, right and top of its points.
    bounds: Option<[i32; 4]>,
    /// The points.
    points: Vec<[i32; 2]>,
    /// The segments.
    verbs: Vec<Verb>,
}

/// A segment of an outline.
#[derive(Clone, Copy)]
enum Verb {
    /// The line back to the start.
    Close,
    /// A curve of three points.
    Curve,
    /// A line.
    Line,
    /// The start of a subpath.
    Move,
}

impl Cache {
    /// The program in `file`, a stream or a reference to it; one that cannot be used is a
    /// warning and none.
    fn program(
        &mut self,
        job: &Job<'_>,
        file: &Resolved,
    ) -> Result<Option<Rc<Embedded>>, PdfError> {
        let key = file.id().map(|id| id.number);
        if let Some(found) = key.and_then(|known| self.programs.entries_mut().get(known)) {
            return Ok(found);
        }
        let stream = data::followed(job, file)?;
        let message = "an embedded font program cannot be used";
        let read = data::whole(job, &stream, (most(job), 0));
        let mut made = None;
        let mut size = TABLE_SIZE;
        if let Some(bytes) = job.tolerate(read, "9.9", message)? {
            job.spend(u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
            // Its reading's work counts though it fails, as the failure is kept.
            let mut spent = 0;
            let parsed = Program::parse(bytes, &mut spent);
            job.spend(spent)?;
            match parsed {
                Ok(program) => {
                    size = program.held();
                    made = Some(Rc::new(Embedded {
                        number: key,
                        program,
                    }));
                }
                Err(Fault::Limit) if job.objects.strict() => return Err(job.limit()),
                Err(Fault::Broken | Fault::Limit) => job.warn("9.9", message)?,
            }
        }
        if key.is_some() {
            self.programs.entries_mut().trim(size, most(job));
        }
        if let Some(embedded) = made.as_ref() {
            job.lend(embedded, size)?;
        }
        if let Some(known) = key {
            self.programs.offer(job);
            self.programs.entries_mut().put(known, made.clone(), size);
        }
        Ok(made)
    }

    /// The outline of `glyph` in `embedded`; a broken one is a warning and empty.
    fn traced(
        &mut self,
        job: &Job<'_>,
        embedded: &Embedded,
        glyph: u32,
    ) -> Result<Rc<Traced>, PdfError> {
        let key = embedded.number.map(|number| (number, glyph));
        if let Some(found) = key.and_then(|known| self.outlines.get(&known)) {
            return Ok(Rc::clone(found));
        }
        let mut traced = Traced::default();
        match embedded.program.outline(glyph, &mut traced) {
            Ok(steps) => job.spend(steps)?,
            Err((Fault::Limit, _steps)) if job.objects.strict() => return Err(job.limit()),
            Err((Fault::Broken | Fault::Limit, steps)) => {
                job.spend(steps)?;
                job.warn("9.9", "a glyph is broken")?;
                // What came before the damage is dropped.
                traced = Traced::default();
            }
        }
        traced.points.shrink_to_fit();
        traced.verbs.shrink_to_fit();
        let size = traced.bytes();
        let made = Rc::new(traced);
        if let Some(known) = key
            && size <= OUTLINES
        {
            if self.outlines_held.saturating_add(size) > OUTLINES {
                self.outlines.clear();
                self.outlines_held = 0;
            }
            self.outlines_held = self.outlines_held.saturating_add(size);
            let _none: Option<Rc<Traced>> = self.outlines.insert(known, Rc::clone(&made));
        }
        Ok(made)
    }
}

impl Composite {
    /// What the code that starts `bytes` is; `None` when it is cut short.
    fn code(&self, job: &Job<'_>, bytes: &[u8]) -> Result<Option<Code>, PdfError> {
        let Some(cmap) = self.cmap.as_ref() else {
            return Ok(None);
        };
        let Some((cid, length)) = cmap.next(job, bytes)? else {
            return Ok(None);
        };
        let key = u64::from(cid);
        let width = self.widths.find(key).map_or(self.default, |[width]| width);
        let (advance, origin) = if cmap.is_vertical() {
            let [top, step] = self.downward;
            let half = width.times(Fixed::from_bits(1 << 31_u32));
            let [down, across, up] = self.heights.find(key).unwrap_or([step, half, top]);
            (down, [across, up])
        } else {
            (width, [Fixed::default(); 2])
        };
        Ok(Some(Code {
            advance,
            length,
            origin,
            rise: Fixed::default(),
            selector: cid,
            spaced: length == 1 && bytes.first() == Some(&b' '),
        }))
    }

    /// The glyph of `cid`, that of CID 0 where there is none (9.7.4.2, 9.7.6.3).
    fn glyph(&self, program: &Program, cid: u32) -> u32 {
        let mapped = |wanted: u32| match self.map.as_deref() {
            Some(map) => {
                let at = usize::try_from(wanted).ok()?.checked_mul(2)?;
                let pair = map.get(at..at.checked_add(2)?)?;
                Some(number(pair)).filter(|glyph| *glyph < program.glyphs())
            }
            None => program.cid(wanted),
        };
        mapped(cid).or_else(|| mapped(0)).unwrap_or(0)
    }

    /// The Type 0 font `owner` and its `CIDFont`.
    fn read(run: &mut Run<'_, '_>, owner: &Object) -> Result<(Self, Resolved), PdfError> {
        let job = run.job;
        let encoding = data::entry(job, owner, b"Encoding")?;
        let cmap = if let Some(name) = data::name(encoding.value()) {
            Cmap::named(name)
        } else if matches!(*encoding.value(), Object::Stream(..)) {
            Cmap::read(run, &encoding)?
        } else {
            return Err(job.objects.malformed());
        };
        let listed = data::array(job, owner, b"DescendantFonts", 1)?;
        let Some(cid_font) = listed
            .into_iter()
            .next()
            .filter(|found| matches!(*found.value(), Object::Dictionary(_)))
        else {
            return Err(job.objects.malformed());
        };
        let member = cid_font.value();
        let default = data::entry(job, member, b"DW")?;
        let downward = job.tolerate(
            data::numbers(job, member, b"DW2"),
            "9.7.4.3",
            "DW2 is malformed",
        )?;
        let kind = data::entry(job, member, b"Subtype")?;
        // Only a CIDFontType2 has one (Table 115).
        let listed_map = if matches!(kind.value(), Object::Name(name) if name == b"CIDFontType2") {
            data::entry(job, member, b"CIDToGIDMap")?
        } else {
            data::resolved(job, Object::Null)?
        };
        let mut map = None;
        if matches!(*listed_map.value(), Object::Stream(..)) {
            let read = data::start(job, &listed_map, CID_MAP);
            if let Some((bytes, ended)) = job.tolerate(read, "9.7.4.2", "CIDToGIDMap is damaged")? {
                if !ended {
                    job.warn("9.7.4.2", "CIDToGIDMap maps CIDs past 65,535")?;
                }
                map = Some(bytes);
            }
        } else if !matches!(listed_map.value(), Object::Null)
            && !matches!(listed_map.value(), Object::Name(name) if name == b"Identity")
        {
            job.warn("9.7.4.2", "CIDToGIDMap is neither Identity nor a stream")?;
        } else {
            // Identity.
        }
        let width = Fixed::from_object(default.value());
        if width.is_none() && !matches!(*default.value(), Object::Null) {
            job.warn("9.7.4.3", "DW is not a number")?;
        }
        // Each part counts beside those read before it.
        let held = cmap
            .as_ref()
            .map_or(0, Cmap::bytes)
            .saturating_add(u64::try_from(map.as_ref().map_or(0, Vec::len)).unwrap_or(u64::MAX));
        let heights = spans(job, (member, b"W2"), held)?;
        let widths = spans(job, (member, b"W"), held.saturating_add(heights.bytes()))?;
        let composite = Self {
            cmap,
            default: width.unwrap_or(THOUSAND),
            downward: downward
                .flatten()
                .unwrap_or([Fixed::from_i64(880), Fixed::from_i64(-1000)]),
            heights,
            map,
            widths,
        };
        Ok((composite, cid_font))
    }
}

impl Encoding {
    /// The name of `code`.
    pub(super) fn name(&self, code: u8) -> Option<&[u8]> {
        let changed = self
            .changed
            .get(usize::from(code))
            .and_then(Option::as_deref);
        changed.or_else(|| self.base?.glyph_name(code))
    }

    /// The encoding of the font `owner`.
    pub(super) fn read(job: &Job<'_>, owner: &Object) -> Result<Self, PdfError> {
        let listed = data::entry(job, owner, b"Encoding")?;
        let mut encoding = Self {
            base: None,
            changed: Vec::new(),
            present: !matches!(*listed.value(), Object::Null),
        };
        let dictionary = matches!(*listed.value(), Object::Dictionary(_));
        let named = if dictionary {
            data::entry(job, listed.value(), b"BaseEncoding")?
        } else {
            listed.clone()
        };
        if let Some(name) = data::name(named.value()) {
            encoding.base = Base::named(name);
            if encoding.base.is_none() {
                job.warn("9.6.5", "an encoding is unknown")?;
            }
        } else if !matches!(*named.value(), Object::Null) {
            job.warn("9.6.5", "an encoding is neither a name nor a dictionary")?;
        } else {
            // The font's own.
        }
        if dictionary {
            let read = data::entry(job, listed.value(), b"Differences").and_then(|found| {
                if data::items(found.value()).is_some_and(|items| items.len() > DIFFERENCES) {
                    // A cap of this implementation's: 9.6.5.1 lets the array repeat itself.
                    Err(job.capped())
                } else {
                    data::array(job, listed.value(), b"Differences", DIFFERENCES)
                }
            });
            if let Some(items) = job.tolerate(read, "9.6.5.1", "Differences is malformed")? {
                encoding.take(job, &items)?;
            }
        }
        Ok(encoding)
    }

    /// Takes the `Differences` (9.6.5.1).
    fn take(&mut self, job: &Job<'_>, items: &[Resolved]) -> Result<(), PdfError> {
        if items.is_empty() {
            return Ok(());
        }
        self.changed.resize(256, None);
        let mut code: Option<usize> = None;
        for item in items {
            job.spend(1)?;
            if let Object::Integer(value) = *item.value() {
                code = usize::try_from(value).ok();
                continue;
            }
            let slot = code.and_then(|index| self.changed.get_mut(index));
            // No glyph name is longer than a reader takes (Annex C).
            let name = data::name(item.value()).filter(|name| name.len() <= NAME);
            match (name, slot) {
                (Some(found), Some(changed)) => *changed = Some(found.to_vec()),
                (_, _) => job.warn("9.6.5.1", "Differences is malformed")?,
            }
            code = code.map(|index| index.saturating_add(1));
        }
        Ok(())
    }
}

impl Font {
    /// The unit of advances, in text space: thousandths, or one for Type 3.
    pub(crate) fn divisor(&self) -> Fixed {
        if self.tables.procedures.is_some() {
            Fixed::from_i64(1)
        } else {
            THOUSAND
        }
    }

    /// Descent and ascent, in text space units of size 1: the descriptor's, else its `FontBBox`'s
    /// bottom and top, in thousandths; drawing does not read them (9.8.1).
    pub(crate) fn extent(&self, job: &Job<'_>) -> Result<[Fixed; 2], PdfError> {
        let given = self.tables.descriptor.as_ref().unwrap_or(&Object::Null);
        let descriptor = data::resolve(job, given)?;
        let number = |key: &[u8]| -> Result<Option<Fixed>, PdfError> {
            let found = data::entry(job, descriptor.value(), key)?;
            Ok(Fixed::from_object(found.value()).and_then(|value| value.over(THOUSAND)))
        };
        if let (Some(descent), Some(ascent)) = (number(b"Descent")?, number(b"Ascent")?) {
            return Ok([descent, ascent]);
        }
        let [_, bottom, _, top] = bounds(job, descriptor.value())?.unwrap_or_default();
        Ok([bottom, top].map(|value| value.over(THOUSAND).unwrap_or_default()))
    }

    /// Whether the glyph of `code` marks nothing by its name.
    pub(super) fn is_blank(&self, code: &Code) -> bool {
        let index = usize::try_from(code.selector).unwrap_or(usize::MAX);
        self.tables.blanks.get(index).copied().unwrap_or_default()
    }

    /// Whether it needs no program or has one embedded.
    pub(crate) fn is_embedded(&self) -> bool {
        self.tables.embedded || self.tables.procedures.is_some()
    }

    /// Whether how far its text goes is known.
    pub(super) fn is_measured(&self) -> bool {
        self.tables.measured || self.is_embedded()
    }

    /// Whether its codes can be read.
    pub(crate) fn is_readable(&self) -> bool {
        let composite = self.tables.composite.as_ref();
        composite.is_none_or(|found| found.cmap.is_some())
    }

    /// Whether it writes downwards.
    pub(crate) fn is_vertical(&self) -> bool {
        let composite = self.tables.composite.as_ref();
        let cmap = composite.and_then(|found| found.cmap.as_ref());
        cmap.is_some_and(Cmap::is_vertical)
    }

    /// What the code that starts `bytes` is; `None` when it is cut short.
    pub(crate) fn next(&self, job: &Job<'_>, bytes: &[u8]) -> Result<Option<Code>, PdfError> {
        if let Some(composite) = self.tables.composite.as_ref() {
            return composite.code(job, bytes);
        }
        Ok(bytes.first().map(|byte| Code {
            advance: self
                .tables
                .widths
                .get(usize::from(*byte))
                .copied()
                .unwrap_or_default(),
            length: 1,
            origin: [Fixed::default(); 2],
            rise: self
                .tables
                .rises
                .get(usize::from(*byte))
                .copied()
                .unwrap_or_default(),
            selector: u32::from(*byte),
            spaced: *byte == b' ',
        }))
    }

    /// The outline of the glyph `code` selects and its map to text space; none marks nothing.
    pub(crate) fn outline(
        &self,
        job: &Job<'_>,
        cache: &mut Cache,
        code: &Code,
    ) -> Result<Option<(Rc<Traced>, Matrix)>, PdfError> {
        let Some(embedded) = self.program.as_deref() else {
            return Ok(None);
        };
        let program = &embedded.program;
        let glyph = self.tables.composite.as_ref().map_or_else(
            || {
                let index = usize::try_from(code.selector).unwrap_or(usize::MAX);
                self.tables.glyphs.get(index).copied().flatten()
            },
            |composite| Some(composite.glyph(program, code.selector)),
        );
        let Some(chosen) = glyph else {
            if !self.is_blank(code) {
                job.warn("9.6.5.4", "a glyph name cannot be resolved")?;
            }
            return Ok(None);
        };
        let traced = cache.traced(job, embedded, chosen)?;
        if traced.bounds.is_none() {
            return Ok(None);
        }
        let matrix = Matrix {
            terms: program.matrix(chosen).map(Fixed::from_bits),
        };
        Ok(Some((traced, matrix)))
    }

    /// Its procedures, as a Type 3 font.
    pub(crate) fn procedures(&self) -> Option<&Procedures> {
        self.tables.procedures.as_ref()
    }
}

impl<T> Clear for Kept<T> {
    fn clear(&mut self) {
        self.entries.clear();
        self.held = 0;
    }
}

impl<T> Default for Kept<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            held: 0,
        }
    }
}

impl<T: Clone> Kept<T> {
    /// The value of `key`, now the most recently used.
    fn get(&mut self, key: u32) -> Option<T> {
        let at = self.entries.iter().position(|entry| entry.0 == key)?;
        let entry = self.entries.remove(at);
        let value = entry.1.clone();
        self.entries.push(entry);
        Some(value)
    }

    /// Adds `value` of `size` bytes.
    fn put(&mut self, key: u32, value: T, size: u64) {
        self.held = self.held.saturating_add(size);
        self.entries.push((key, value, size));
    }

    /// Drops the least recent until `size` more fit in `most`, and one more entry.
    fn trim(&mut self, size: u64, most: u64) {
        while (self.held.saturating_add(size) > most || self.entries.len() >= ENTRIES)
            && !self.entries.is_empty()
        {
            let dropped = self.entries.remove(0);
            self.held = self.held.saturating_sub(dropped.2);
        }
    }
}

impl Tables {
    /// The bytes it holds.
    fn bytes(&self) -> u64 {
        let composite = self.composite.as_ref().map_or(0, |found| {
            let map = found.map.as_ref().map_or(0, Vec::len);
            found
                .cmap
                .as_ref()
                .map_or(0, Cmap::bytes)
                .saturating_add(found.heights.bytes())
                .saturating_add(found.widths.bytes())
                .saturating_add(u64::try_from(map).unwrap_or(u64::MAX))
        });
        let procedures = self.procedures.as_ref().map_or(0, Procedures::bytes);
        TABLE_SIZE
            .saturating_add(composite)
            .saturating_add(procedures)
            .saturating_add(self.descriptor.as_ref().map_or(0, data::size))
    }
}

impl Traced {
    /// The pixels it touches under `matrix`.
    pub(super) fn area(&self, matrix: &Matrix) -> Area {
        self.bounds
            .map_or_else(Area::default, |corners| matrix.bounds(corners.map(unit)))
    }

    /// The bytes it holds.
    fn bytes(&self) -> u64 {
        let points = self.points.capacity().saturating_mul(size_of::<[i32; 2]>());
        let verbs = self.verbs.capacity().saturating_mul(size_of::<Verb>());
        let total = points.saturating_add(verbs).saturating_add(128);
        u64::try_from(total).unwrap_or(u64::MAX)
    }

    /// Left, bottom, right and top of its points, in font units.
    pub(crate) fn corners(&self) -> Option<[Fixed; 4]> {
        self.bounds.map(|corners| corners.map(unit))
    }

    /// Draws it into `path` under `matrix`.
    pub(super) fn draw(
        &self,
        job: &Job<'_>,
        path: &mut Path,
        matrix: &Matrix,
    ) -> Result<(), PdfError> {
        let mut points = self.points.iter().map(|point| {
            let [across, down] = point.map(unit);
            matrix.apply(Point { across, down })
        });
        let mut next = || points.next().unwrap_or_default();
        for verb in &self.verbs {
            match *verb {
                Verb::Close => path.close(job)?,
                Verb::Curve => path.curve_to(job, [next(), next(), next()])?,
                Verb::Line => path.line_to(job, next())?,
                Verb::Move => path.move_to(job, next())?,
            }
        }
        Ok(())
    }

    /// Adds `point`.
    fn reach(&mut self, point: [i32; 2]) {
        let [across, up] = point;
        let [left, bottom, right, top] = self.bounds.unwrap_or([across, up, across, up]);
        self.bounds = Some([
            left.min(across),
            bottom.min(up),
            right.max(across),
            top.max(up),
        ]);
        self.points.push(point);
    }
}

impl Outline for Traced {
    fn close(&mut self) {
        self.verbs.push(Verb::Close);
    }

    fn curve(&mut self, points: [[i32; 2]; 3]) {
        self.verbs.push(Verb::Curve);
        for point in points {
            self.reach(point);
        }
    }

    fn line(&mut self, to: [i32; 2]) {
        self.verbs.push(Verb::Line);
        self.reach(to);
    }

    fn start(&mut self, at: [i32; 2]) {
        self.verbs.push(Verb::Move);
        self.reach(at);
    }
}

/// The `FontBBox` of `owner`; none where absent or malformed, which drawing does not need, so
/// it is not warned of (9.6.4, 9.8.1).
pub(super) fn bounds(job: &Job<'_>, owner: &Object) -> Result<Option<[Fixed; 4]>, PdfError> {
    match data::numbers(job, owner, b"FontBBox") {
        Err(error) if error.code() == ErrorCode::Malformed => Ok(None),
        found => found,
    }
}

/// The program's stream the descriptor of `owner` lists, and whether as Type 1's.
fn file(job: &Job<'_>, owner: &Object) -> Result<Option<(Resolved, bool)>, PdfError> {
    let descriptor = data::entry(job, owner, b"FontDescriptor")?;
    for key in [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"] {
        let found = data::entry(job, descriptor.value(), key)?;
        if !matches!(*found.value(), Object::Null) {
            return Ok(Some((found, key == b"FontFile")));
        }
    }
    Ok(None)
}

/// Each code's glyph (9.6.5.2, 9.6.5.4); `symbolic` is given for TrueType's rules only.
fn glyphs(program: &Program, encoding: &Encoding, symbolic: Option<bool>) -> Vec<Option<u32>> {
    let named = |code| {
        let name = encoding.name(code)?;
        program.named(name)
    };
    let Some(flagged) = symbolic else {
        return (0..=u8::MAX)
            .map(|code| {
                let found = if encoding.name(code).is_some() || encoding.base.is_some() {
                    named(code)
                } else {
                    program.built_in(code)
                };
                // What is missing is .notdef.
                Some(found.unwrap_or(0))
            })
            .collect();
    };
    let own = |code| program.built_in(code);
    if flagged && !encoding.present {
        return (0..=u8::MAX).map(own).collect();
    }
    // A nonsymbolic font without one names its codes by StandardEncoding (9.6.5.4).
    let filled = |code| {
        let name = encoding
            .name(code)
            .or_else(|| Base::Standard.glyph_name(code))?;
        program.named(name)
    };
    if flagged {
        return (0..=u8::MAX)
            .map(|code| own(code).or_else(|| filled(code)))
            .collect();
    }
    let found: Vec<Option<u32>> = (0..=u8::MAX).map(filled).collect();
    // A program that maps no name at all is read as a symbolic one.
    if found.iter().flatten().all(|glyph| *glyph == 0) {
        return (0..=u8::MAX).map(own).collect();
    }
    // What is left is mapped as 9.6.5.4 leaves open: by a symbol table, else blank.
    let symbol = |code: u8| program.mapped(Table::Symbol, 0xF000 | u32::from(code));
    (0..=u8::MAX)
        .zip(found)
        .map(|(code, glyph)| glyph.or_else(|| symbol(code)))
        .collect()
}

/// The font `font` (9.6, 9.7).
pub(super) fn load(run: &mut Run<'_, '_>, font: &Resolved) -> Result<Rc<Font>, PdfError> {
    let job = run.job;
    let key = font.id().map(|id| id.number);
    if let Some(tables) =
        key.and_then(|number| run.caches.fonts.loaded.tables.entries_mut().get(number))
    {
        let program = match tables.file {
            Some(id) => {
                let reference = data::reference(id);
                run.caches.fonts.loaded.program(job, &reference)?
            }
            None => None,
        };
        return Ok(Rc::new(Font { program, tables }));
    }
    let owner = font.value();
    if !matches!(owner, Object::Dictionary(_)) {
        return Err(job.objects.malformed());
    }
    let (made, program, lasting) = made(run, owner)?;
    let size = made.bytes();
    job.reserve(size)?;
    let cache = &mut run.caches.fonts.loaded;
    let held = key.filter(|_number| lasting);
    if held.is_some() {
        cache.tables.entries_mut().trim(size, most(job));
    }
    let tables = Rc::new(made);
    job.lend(&tables, size)?;
    if let Some(number) = held {
        cache.tables.offer(job);
        cache
            .tables
            .entries_mut()
            .put(number, Rc::clone(&tables), size);
    }
    Ok(Rc::new(Font { program, tables }))
}

/// What the font `owner` gives, its program, and whether that can be found again.
fn made(
    run: &mut Run<'_, '_>,
    owner: &Object,
) -> Result<(Tables, Option<Rc<Embedded>>, bool), PdfError> {
    let job = run.job;
    let kind = data::entry(job, owner, b"Subtype")?;
    let is = |name: &[u8]| matches!(kind.value(), Object::Name(found) if found.as_slice() == name);
    let mut tables = Tables::default();
    if is(b"Type3") {
        let procedures = Procedures::read(job, owner)?;
        if let Some(found) = widths(job, owner, Fixed::default())? {
            tables.rises = found.iter().map(|width| procedures.rise(*width)).collect();
            tables.widths = found
                .into_iter()
                .map(|width| procedures.width(width))
                .collect();
        } else {
            job.warn("9.6.4", "a Type 3 font has no Widths")?;
        }
        tables.procedures = Some(procedures);
        return Ok((tables, None, true));
    }
    if is(b"CIDFontType0") || is(b"CIDFontType2") {
        return Err(job.objects.malformed());
    }
    let composed = if is(b"Type0") {
        let (composite, cid_font) = Composite::read(run, owner)?;
        tables.composite = Some(composite);
        tables.measured = true;
        Some(cid_font)
    } else {
        None
    };
    let described = composed.as_ref().map_or(owner, |cid_font| cid_font.value());
    tables.descriptor = described.get(b"FontDescriptor");
    let listed = file(job, described)?;
    tables.embedded = listed.is_some();
    let program = if let Some(found) = listed.as_ref() {
        let stream = &found.0;
        tables.file = stream.id();
        run.caches.fonts.loaded.program(job, stream)?
    } else {
        None
    };
    if composed.is_none() {
        let truetype = is(b"TrueType") && !listed.as_ref().is_some_and(|found| found.1);
        simple(job, owner, &mut tables, program.as_deref(), truetype)?;
    }
    let lasting = !tables.embedded || tables.file.is_some();
    Ok((tables, program, lasting))
}

/// Takes the work of looking up the names of 256 codes, at eight bytes scanned a unit.
pub(super) fn looked_up(job: &Job<'_>) -> Result<(), PdfError> {
    job.spend(encoding::scanned().saturating_mul(256) >> 3_u32)
}

/// The most bytes fonts may hold.
fn most(job: &Job<'_>) -> u64 {
    job.objects.decoded_limit().min(HELD)
}

/// Whether what `code` stands for is known without the program: it has no name, or one of Annex D's,
/// `uniXXXX` or `uXXXX` (9.6.5.4).
fn resolves(encoding: &Encoding, code: u8) -> bool {
    if !encoding.present {
        return true;
    }
    let named = encoding
        .name(code)
        .or_else(|| Base::Standard.glyph_name(code));
    named.is_none_or(|name| {
        name == b".notdef"
            || !encoding::characters(name).is_empty()
            || encoding::roman_code(name).is_some()
    })
}

/// Takes a simple font's widths and glyphs.
fn simple(
    job: &Job<'_>,
    owner: &Object,
    tables: &mut Tables,
    embedded: Option<&Embedded>,
    truetype: bool,
) -> Result<(), PdfError> {
    let descriptor = data::entry(job, owner, b"FontDescriptor")?;
    let missing = data::entry(job, descriptor.value(), b"MissingWidth")?;
    let fallback = Fixed::from_object(missing.value()).unwrap_or_default();
    if let Some(found) = widths(job, owner, fallback)? {
        tables.measured = true;
        tables.widths = found;
    } else {
        if tables.embedded {
            job.warn("9.6.2.1", "a font has no Widths")?;
        }
        tables.widths = vec![fallback; 256];
    }
    let encoding = Encoding::read(job, owner)?;
    looked_up(job)?;
    tables.blanks = (0..=u8::MAX)
        .map(|code| {
            let name = encoding
                .name(code)
                .or_else(|| Base::Standard.glyph_name(code));
            matches!(name, Some(b"space" | b"nonbreakingspace"))
        })
        .collect();
    let Some(held) = embedded else {
        return Ok(());
    };
    let symbolic = if truetype {
        let listed = data::entry(job, descriptor.value(), b"Flags")?;
        let flags = match *listed.value() {
            Object::Integer(value) => value,
            Object::Null => 0,
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Name(_)
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => {
                job.warn("9.8.2", "a font's Flags are not an integer")?;
                0
            }
        };
        // Symbolic, and not Nonsymbolic as well (9.8.2).
        Some(flags & 0x24 == 4)
    } else {
        None
    };
    looked_up(job)?;
    tables.glyphs = glyphs(&held.program, &encoding, symbolic);
    if symbolic.is_some() {
        looked_up(job)?;
        let notdef = held.program.named(b".notdef");
        for ((code, slot), blank) in (0..=u8::MAX)
            .zip(tables.glyphs.iter_mut())
            .zip(&tables.blanks)
        {
            // A character without a glyph shows .notdef, which is no deviation.
            if slot.is_none() && !*blank && resolves(&encoding, code) {
                *slot = notdef;
            }
        }
    }
    Ok(())
}

/// The ranges of `W` or `W2` of `owner`, `COUNT` numbers to a CID, counted beside `held`
/// bytes (9.7.4.3).
fn spans<const COUNT: usize>(
    job: &Job<'_>,
    (owner, key): (&Object, &[u8]),
    held: u64,
) -> Result<Ranges<[Fixed; COUNT]>, PdfError> {
    let mut ranges = Ranges::default();
    let filled = spread(job, (owner, key), held, &mut ranges);
    let _all: Option<()> = job.tolerate(filled, "9.7.4.3", "a CIDFont's widths are malformed")?;
    Ok(ranges)
}

/// Fills `ranges` from `key` of `owner` up to what is malformed.
fn spread<const COUNT: usize>(
    job: &Job<'_>,
    (owner, key): (&Object, &[u8]),
    held: u64,
    ranges: &mut Ranges<[Fixed; COUNT]>,
) -> Result<(), PdfError> {
    let malformed = || job.objects.malformed();
    let cid = |value: &Object| {
        Fixed::from_object(value)
            .map(Fixed::floor)
            .and_then(|found| u64::try_from(found.min(LAST_CID.saturating_add(1))).ok())
            .ok_or_else(malformed)
    };
    let figure = |value: &Object| {
        let found = data::resolve(job, value)?;
        Fixed::from_object(found.value()).ok_or_else(malformed)
    };
    let last_cid = u64::try_from(LAST_CID).unwrap_or(0);
    let mut items = data::array(job, owner, key, usize::MAX)?.into_iter();
    while let Some(head) = items.next() {
        job.spend(16)?;
        let first = cid(head.value())?;
        let second = items.next().ok_or_else(malformed)?;
        let mut value = [Fixed::default(); COUNT];
        if let Some(listed) = data::items(second.value()) {
            let mut rest = listed.iter();
            let mut at = first;
            while rest.len() > 0 {
                for slot in &mut value {
                    *slot = figure(rest.next().ok_or_else(malformed)?)?;
                }
                if at <= last_cid {
                    job.fits(
                        held.saturating_add(ranges.bytes())
                            .saturating_add(RANGE.saturating_mul(2)),
                    )?;
                    ranges.put(at, at, value);
                }
                at = at.saturating_add(1);
            }
        } else {
            let last = cid(second.value())?;
            for slot in &mut value {
                *slot = figure(items.next().ok_or_else(malformed)?.value())?;
            }
            job.fits(
                held.saturating_add(ranges.bytes())
                    .saturating_add(RANGE.saturating_mul(2)),
            )?;
            ranges.put(first, last.min(last_cid), value);
        }
    }
    Ok(())
}

/// A coordinate of an outline.
fn unit(value: i32) -> Fixed {
    Fixed::from_bits(i64::from(value) << 16_u32)
}

/// Each code's width from `Widths` and `FirstChar`, `fallback` outside them; none without
/// `Widths` (9.6.2.1).
fn widths(job: &Job<'_>, owner: &Object, fallback: Fixed) -> Result<Option<Vec<Fixed>>, PdfError> {
    let message = "a font's Widths are malformed";
    let read = data::array(job, owner, b"Widths", usize::MAX);
    let items = match job.tolerate(read, "9.6.2.1", message)? {
        Some(items) if items.is_empty() => return Ok(None),
        Some(items) => items,
        None => return Ok(Some(vec![fallback; 256])),
    };
    let listed = data::entry(job, owner, b"FirstChar")?;
    let first = match *listed.value() {
        Object::Integer(value) => value,
        Object::Null => 0,
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Name(_)
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => {
            job.warn("9.6.2.1", "a font's FirstChar is not an integer")?;
            0
        }
    };
    let mut found = vec![fallback; 256];
    let mut bad = false;
    for (index, item) in items.iter().enumerate() {
        job.spend(1)?;
        let code = first.saturating_add(i64::try_from(index).unwrap_or(i64::MAX));
        let Some(slot) = usize::try_from(code)
            .ok()
            .and_then(|place| found.get_mut(place))
        else {
            if code > 255 {
                break;
            }
            continue;
        };
        match Fixed::from_object(item.value()) {
            Some(width) => *slot = width,
            None => bad = true,
        }
    }
    if bad {
        job.warn("9.6.2.1", message)?;
    }
    Ok(Some(found))
}
