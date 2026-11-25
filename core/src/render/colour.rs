//! Colour spaces and conversion to RGB (ISO 32000-2:2020, 8.6, 10.4).

mod cie;
mod icc;

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::slice;

use crate::overlay::Object;
use crate::render::data::{self, Category, Resolved, Scope};
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::function::{self, Function, VALUES, room};
use crate::render::job::Job;
use crate::render::lent::{Clear, Yielding};
use crate::{PdfError, pull};

/// The bytes the caches hold.
const CACHE: u64 = 0x10_0000;

/// The entries of a converter.
const CONVERTED: usize = 4096;

/// The inputs a converter holds.
const CONVERTER_INPUTS: usize = 0x4000;

/// How deep spaces nest.
const DEPTH: u8 = 8;

/// Two to the 64 over the golden ratio, to spread a converter's inputs.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// Work of looking a colour up in a converter: hashing and comparing.
const HIT_WORK: u64 = 2;

/// Work of a conversion that a converter does not hold.
const MISS_WORK: u64 = 3;

/// The most bytes of a profile.
const PROFILE: u64 = 0x40_0000;

/// Bytes a space holds besides its table and ranges.
const SPACE_SIZE: u64 = 256;

/// A space whose profile is used.
struct Based {
    /// The profile.
    profile: Rc<icc::Profile>,
    /// What its inputs go from and to.
    ranges: Vec<[Fixed; 2]>,
}

/// `DeviceCMYK` (10.4.2).
struct Cmyk;

/// Converts many colours of one space.
pub(crate) struct Converter {
    /// The colours converted, by slot; empty until the first.
    colours: Vec<Option<[u16; 3]>>,
    /// The intent.
    intent: Intent,
    /// What gave each slot's colour, a colour after the other.
    keys: Vec<Fixed>,
    /// The space.
    space: Rc<Space>,
}

/// `DeviceGray`.
struct Gray;

/// An Indexed space (8.6.6.3).
struct Indexed {
    /// The base.
    base: Rc<Space>,
    /// The base's components, a colour after the other; what is missing is 0.
    table: Vec<u8>,
}

/// A rendering intent (8.6.5.8).
#[derive(Clone, Copy, Default)]
pub(crate) enum Intent {
    /// `AbsoluteColorimetric`.
    Absolute,
    /// `Perceptual`.
    Perceptual,
    /// `RelativeColorimetric`.
    #[default]
    Relative,
    /// `Saturation`.
    Saturation,
}

/// What resolving one space needs.
struct Resolver<'run, 'doc> {
    /// The job.
    job: &'run Job<'doc>,
    /// The resources the space is used with.
    scope: &'run Scope,
    /// The caches.
    spaces: &'run mut Spaces,
}

/// `DeviceRGB`.
struct Rgb;

/// A colour space.
#[derive(Clone)]
pub(crate) struct Space {
    /// Whether converting costs less than looking up.
    direct: bool,
    /// Whether its samples are indices.
    indexed: bool,
    /// Its conversion.
    kind: Rc<dyn Convert>,
    /// Whether painting in it marks the page.
    marks: bool,
    /// The range of each component.
    ranges: Vec<[Fixed; 2]>,
    /// The initial value of each component, before clamping.
    start: Vec<Fixed>,
}

/// A count of bytes held.
#[derive(Default)]
struct Held(u64);

/// Profiles by number and components; `None` when unusable.
type Profiles = BTreeMap<(u32, usize), Option<Rc<icc::Profile>>>;

/// What a render read of spaces, by object number, and what of it is alive.
#[derive(Default)]
pub(crate) struct Spaces {
    /// Gamma curves, by the bits of gamma.
    curves: Yielding<BTreeMap<i64, Rc<cie::Curve>>>,
    /// Tint transformations.
    functions: Yielding<BTreeMap<u32, Rc<Function>>>,
    /// The bytes the caches hold, reset as they give way.
    held: Yielding<Held>,
    /// Profiles by number and components; `None` when unusable.
    profiles: Yielding<Profiles>,
}

/// A Separation or `DeviceN` space through its tint transformation (8.6.6.4, 8.6.6.5).
struct Tinted {
    /// The alternate space.
    alternate: Rc<Space>,
    /// The tint transformation.
    tint: Rc<Function>,
}

/// `Separation` of `All`, as on RGB: one less the tint.
struct Whole;

/// What turns a colour within its ranges into RGB.
trait Convert {
    /// The colour `values` in RGB with `intent`.
    fn rgb(&self, job: &Job<'_>, values: &[Fixed], intent: Intent) -> Result<[u16; 3], PdfError>;
}

impl Convert for Based {
    fn rgb(&self, job: &Job<'_>, values: &[Fixed], intent: Intent) -> Result<[u16; 3], PdfError> {
        job.spend(self.profile.work())?;
        let plain = [Fixed::default(), Fixed::from_i64(1)];
        let mut given = [0_i64; 4];
        for ((slot, &value), &range) in given.iter_mut().zip(values).zip(&self.ranges) {
            let part = if range == plain {
                value
            } else {
                function::interpolate(value, range, plain)
            };
            *slot = cie::sixteenths(part);
        }
        Ok(self.profile.rgb(given, intent))
    }
}

impl Convert for Cmyk {
    fn rgb(&self, job: &Job<'_>, values: &[Fixed], _intent: Intent) -> Result<[u16; 3], PdfError> {
        let Some(&[cyan, magenta, yellow, black]) = values.first_chunk::<4>() else {
            return Err(job.objects.malformed());
        };
        let one = Fixed::from_i64(1);
        Ok([cyan, magenta, yellow].map(|ink| one.minus(ink.plus(black)).unit()))
    }
}

impl Convert for Gray {
    fn rgb(&self, _job: &Job<'_>, values: &[Fixed], _intent: Intent) -> Result<[u16; 3], PdfError> {
        Ok([values.first().copied().unwrap_or_default().unit(); 3])
    }
}

impl Convert for Indexed {
    fn rgb(&self, job: &Job<'_>, values: &[Fixed], intent: Intent) -> Result<[u16; 3], PdfError> {
        let half = Fixed::from_bits(0x8000_0000);
        let index = values
            .first()
            .copied()
            .unwrap_or_default()
            .plus(half)
            .floor();
        let count = self.base.components();
        let start = usize::try_from(index).unwrap_or(0).saturating_mul(count);
        let mut held = [Fixed::default(); VALUES];
        let ranges = self.base.ranges.iter().enumerate();
        for (slot, (at, &[low, high])) in held.iter_mut().zip(ranges) {
            let byte = self
                .table
                .get(start.saturating_add(at))
                .copied()
                .unwrap_or(0);
            let span = i128::from(high.bits()).saturating_sub(i128::from(low.bits()));
            let way = quotient(span.saturating_mul(i128::from(byte)), 255).unwrap_or(0);
            let bits = i128::from(low.bits()).saturating_add(way);
            *slot = Fixed::from_bits(saturated(bits));
        }
        self.base.rgb(job, &held, intent)
    }
}

impl Convert for Rgb {
    fn rgb(&self, job: &Job<'_>, values: &[Fixed], _intent: Intent) -> Result<[u16; 3], PdfError> {
        let found = values
            .first_chunk::<3>()
            .ok_or_else(|| job.objects.malformed())?;
        Ok(found.map(Fixed::unit))
    }
}

impl Convert for Tinted {
    fn rgb(&self, job: &Job<'_>, values: &[Fixed], intent: Intent) -> Result<[u16; 3], PdfError> {
        let mut held = [Fixed::default(); VALUES];
        self.tint.evaluate(job, values, &mut held)?;
        self.alternate.rgb(job, &held, intent)
    }
}

impl Convert for Whole {
    fn rgb(&self, _job: &Job<'_>, values: &[Fixed], _intent: Intent) -> Result<[u16; 3], PdfError> {
        let tint = values.first().copied().unwrap_or_default();
        Ok([Fixed::from_i64(1).minus(tint).unit(); 3])
    }
}

impl Converter {
    /// Converts colours of `space` with `intent`.
    pub(crate) fn new(job: &Job<'_>, space: &Rc<Space>, intent: Intent) -> Result<Self, PdfError> {
        job.spend(1)?;
        Ok(Self {
            colours: Vec::new(),
            intent,
            keys: Vec::new(),
            space: Rc::clone(space),
        })
    }

    /// The colour `values` in RGB.
    pub(crate) fn rgb(&mut self, job: &Job<'_>, values: &[Fixed]) -> Result<[u16; 3], PdfError> {
        if self.space.direct {
            return self.space.rgb(job, values, self.intent);
        }
        let count = self.space.components();
        let given = values.get(..count).ok_or_else(|| job.objects.malformed())?;
        if self.colours.is_empty() {
            let most = CONVERTER_INPUTS
                .checked_div(count)
                .unwrap_or(1)
                .min(CONVERTED);
            let inputs = most.saturating_mul(count);
            self.keys = room(inputs)?;
            self.keys.resize(inputs, Fixed::default());
            self.colours = room(most)?;
            self.colours.resize(most, None);
        }
        let hash = given.iter().fold(0_u64, |hash, value| {
            let bits = value.bits();
            // The magnitude and, apart from it, the sign.
            let part = bits.unsigned_abs() ^ u64::from(bits < 0).rotate_right(1);
            (hash.rotate_left(29) ^ part).wrapping_mul(GOLDEN)
        });
        let slots = u64::try_from(self.colours.len()).unwrap_or(0);
        let slot = usize::try_from((hash >> 32_u32).saturating_mul(slots) >> 32_u32).unwrap_or(0);
        let start = slot.saturating_mul(count);
        let place = start..start.saturating_add(count);
        let known = self.colours.get(slot).copied().flatten();
        // Hashing and comparing take each component.
        let compared = u64::try_from(count >> 2_u32).unwrap_or(u64::MAX);
        if let Some(found) = known
            && self.keys.get(place.clone()) == Some(given)
        {
            job.spend(HIT_WORK.saturating_add(compared))?;
            return Ok(found);
        }
        job.spend(MISS_WORK.saturating_add(compared))?;
        let found = self.space.rgb(job, given, self.intent)?;
        if let (Some(key), Some(colour)) = (self.keys.get_mut(place), self.colours.get_mut(slot)) {
            key.copy_from_slice(given);
            *colour = Some(found);
        }
        Ok(found)
    }
}

impl Resolver<'_, '_> {
    /// `CalGray` or `CalRGB`, the latter when `colours`, of the dictionary `owner` (8.6.5.2, 8.6.5.3).
    fn calibrated(&mut self, owner: &Object, colours: bool) -> Result<Rc<Space>, PdfError> {
        let job = self.job;
        let broken = || job.objects.malformed();
        let white = data::numbers::<3>(job, owner, b"WhitePoint")?.ok_or_else(broken)?;
        let one = Fixed::from_i64(1);
        let nought = Fixed::default();
        let (gammas, columns) = if colours {
            let gammas = data::numbers::<3>(job, owner, b"Gamma")?.unwrap_or([one; 3]);
            let columns = data::numbers::<9>(job, owner, b"Matrix")?.map_or_else(
                || cie::diagonal([one; 3]),
                |[
                    red_x,
                    red_y,
                    red_z,
                    green_x,
                    green_y,
                    green_z,
                    blue_x,
                    blue_y,
                    blue_z,
                ]| {
                    [
                        [red_x, red_y, red_z],
                        [green_x, green_y, green_z],
                        [blue_x, blue_y, blue_z],
                    ]
                },
            );
            (gammas.to_vec(), columns)
        } else {
            let found = data::entry(job, owner, b"Gamma")?;
            let gamma = if matches!(*found.value(), Object::Null) {
                one
            } else {
                Fixed::from_object(found.value()).ok_or_else(broken)?
            };
            (vec![gamma], [white, [nought; 3], [nought; 3]])
        };
        let linear = cie::Linear::new(white, &columns).ok_or_else(broken)?;
        let mut curves = Vec::with_capacity(gammas.len());
        for &gamma in &gammas {
            if gamma <= nought {
                return Err(broken());
            }
            curves.push(self.spaces.curve(job, gamma)?);
        }
        let kind = Rc::new(cie::Calibrated { curves, linear });
        self.kept(Space::new(vec![[nought, one]; gammas.len()], kind), 0)
    }

    /// A Separation space, or with `many` a `DeviceN` space, of the array `items`.
    fn colorants(
        &mut self,
        items: &[Object],
        many: bool,
        depth: u8,
        substitute: bool,
    ) -> Result<Rc<Space>, PdfError> {
        let job = self.job;
        let broken = || job.objects.malformed();
        let listed = data::resolve(job, items.get(1).ok_or_else(broken)?)?;
        let names = match (many, data::items(listed.value())) {
            (true, Some(names)) if !names.is_empty() => names,
            (false, None) if data::name(listed.value()).is_some() => {
                slice::from_ref(listed.value())
            }
            _ => return Err(broken()),
        };
        let count = names.len();
        if count > VALUES {
            return Err(job.capped());
        }
        let (mut blank, mut whole) = (true, !many);
        for name in names {
            let found = data::resolve(job, name)?;
            let word = data::name(found.value()).ok_or_else(broken)?;
            blank &= word == b"None";
            whole &= word == b"All";
        }
        let one = Fixed::from_i64(1);
        let kind: Rc<dyn Convert> = if blank || whole {
            Rc::new(Whole)
        } else {
            let base = data::space(job, items.get(2).ok_or_else(broken)?)?;
            let alternate = self.space(&base, depth.saturating_add(1), substitute)?;
            let value = data::resolve(job, items.get(3).ok_or_else(broken)?)?;
            let tint = self.spaces.function(job, &value)?;
            let fits = tint.inputs() <= count && tint.outputs() >= alternate.components();
            if !fits || alternate.indexed {
                return Err(broken());
            }
            Rc::new(Tinted { alternate, tint })
        };
        let mut space = Space::new(vec![[Fixed::default(), one]; count], kind);
        space.marks = !blank;
        space.start.fill(one);
        self.kept(space, 0)
    }

    /// The device space of `components`, or with `substitute` the scope's default for it
    /// (8.6.5.6).
    fn device(
        &mut self,
        components: usize,
        depth: u8,
        substitute: bool,
    ) -> Result<Rc<Space>, PdfError> {
        let job = self.job;
        let key: &[u8] = match components {
            1 => b"DefaultGray",
            3 => b"DefaultRGB",
            4 => b"DefaultCMYK",
            _ => return Err(job.objects.malformed()),
        };
        if substitute && let Some(value) = data::named(job, self.scope, Category::ColorSpace, key)?
        {
            let found = self.space(&value, depth.saturating_add(1), false);
            let message = "a default colour space cannot be used";
            if let Some(space) = job.tolerate(found, "8.6.5.6", message)? {
                if space.components() == components && !space.indexed {
                    let start = plain(components).start;
                    return self.standing(space, &start);
                }
                job.warn("8.6.5.6", message)?;
            }
        }
        Ok(Rc::new(plain(components)))
    }

    /// An `ICCBased` space of the stream `stream` (8.6.5.5).
    fn icc_based(
        &mut self,
        stream: &Resolved,
        depth: u8,
        substitute: bool,
    ) -> Result<Rc<Space>, PdfError> {
        let job = self.job;
        let owner = stream.value();
        let components = Fixed::from_object(data::entry(job, owner, b"N")?.value())
            .and_then(|count| usize::try_from(count.floor()).ok())
            .filter(|count| matches!(*count, 1 | 3 | 4))
            .ok_or_else(|| job.objects.malformed())?;
        if let Some(profile) = self.spaces.profile(job, stream, components)? {
            let mut ranges = function::pairs(job, &function::numbers(job, owner, b"Range")?)?;
            if ranges.is_empty() {
                ranges = vec![[Fixed::default(), Fixed::from_i64(1)]; components];
            }
            if ranges.len() != components {
                return Err(job.objects.malformed());
            }
            let kind = Rc::new(Based {
                profile,
                ranges: ranges.clone(),
            });
            return self.kept(Space::new(ranges, kind), 0);
        }
        let zeros = vec![Fixed::default(); components];
        let value = data::space(job, data::stored(owner, b"Alternate"))?;
        if !matches!(*value.value(), Object::Null) {
            let found = self.space(&value, depth.saturating_add(1), substitute);
            let message = "an alternate colour space cannot be used";
            if let Some(space) = job.tolerate(found, "8.6.5.5", message)? {
                if space.components() == components && !space.indexed {
                    return self.standing(space, &zeros);
                }
                job.warn("8.6.5.5", message)?;
            }
        }
        let space = self.device(components, depth, substitute)?;
        self.standing(space, &zeros)
    }

    /// An Indexed space of the array `items` (8.6.6.3).
    fn indexed(
        &mut self,
        items: &[Object],
        depth: u8,
        substitute: bool,
    ) -> Result<Rc<Space>, PdfError> {
        let job = self.job;
        let broken = || job.objects.malformed();
        let item = |at: usize| data::resolve(job, items.get(at).ok_or_else(broken)?);
        let listed = data::space(job, items.get(1).ok_or_else(broken)?)?;
        let base = self.space(&listed, depth.saturating_add(1), substitute)?;
        let declared = Fixed::from_object(item(2)?.value())
            .ok_or_else(broken)?
            .floor();
        if base.indexed || declared < 0 {
            return Err(broken());
        }
        let highest = declared.min(255);
        if highest < declared {
            job.warn("8.6.6.3", "an indexed space has over 256 colours")?;
        }
        let colours = usize::try_from(highest).map_err(|_error| broken())?;
        let length = colours.saturating_add(1).saturating_mul(base.components());
        let mut table = vec![0; length];
        let lookup = item(3)?;
        let count = if let Some(bytes) = data::text(lookup.value()) {
            job.spend(u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
            for (slot, byte) in table.iter_mut().zip(bytes) {
                *slot = *byte;
            }
            bytes.len()
        } else if matches!(*lookup.value(), Object::Stream(..)) {
            let mut opened = data::coded(job, &lookup)?;
            if opened.image.is_some() {
                return Err(broken());
            }
            pull::fill(&mut opened.data, &mut table)?
        } else {
            return Err(broken());
        };
        if count < length {
            job.warn("8.6.6.3", "an indexed space's table is short")?;
        }
        let range = [Fixed::default(), Fixed::from_i64(highest)];
        let mut space = Space::new(vec![range], Rc::new(Indexed { base, table }));
        space.indexed = true;
        self.kept(space, u64::try_from(length).unwrap_or(u64::MAX))
    }

    /// Counts `space`, whose table has `bytes`.
    fn kept(&self, space: Space, bytes: u64) -> Result<Rc<Space>, PdfError> {
        let shared = Rc::new(space);
        let size = SPACE_SIZE.saturating_add(bytes).saturating_add(
            u64::try_from(shared.ranges.len())
                .unwrap_or(0)
                .saturating_mul(24),
        );
        self.job.lend(&shared, size)?;
        Ok(shared)
    }

    /// A Lab space of the dictionary `owner` (8.6.5.4).
    fn lab(&self, owner: &Object) -> Result<Rc<Space>, PdfError> {
        let job = self.job;
        let broken = || job.objects.malformed();
        let white = data::numbers::<3>(job, owner, b"WhitePoint")?.ok_or_else(broken)?;
        let hundred = Fixed::from_i64(100);
        let below = Fixed::from_i64(-100);
        let given = data::numbers::<4>(job, owner, b"Range")?;
        let bounds = function::pairs(job, &given.unwrap_or([below, hundred, below, hundred]))?;
        let mut ranges = vec![[Fixed::default(), hundred]];
        ranges.extend(bounds);
        let linear = cie::Linear::new(white, &cie::diagonal(white)).ok_or_else(broken)?;
        self.kept(Space::new(ranges, Rc::new(cie::Lab { linear })), 0)
    }

    /// The space `value` names or holds, `depth` spaces down.
    fn space(
        &mut self,
        value: &Resolved,
        depth: u8,
        substitute: bool,
    ) -> Result<Rc<Space>, PdfError> {
        let job = self.job;
        let broken = || job.objects.malformed();
        if depth > DEPTH {
            return Err(job.capped());
        }
        // No family takes more than five items: a longer array is not copied.
        let items = data::items(value.value()).unwrap_or_default();
        if items.len() > 5 {
            return Err(broken());
        }
        job.spend(u64::try_from(items.len()).unwrap_or(u64::MAX))?;
        let first = items
            .first()
            .map(|item| data::resolve(job, item))
            .transpose()?;
        let family = data::name(
            first
                .as_ref()
                .map_or_else(|| value.value(), Resolved::value),
        )
        .ok_or_else(broken)?;
        let second = || data::resolve(job, items.get(1).ok_or_else(broken)?);
        match family {
            b"CalGray" => self.calibrated(second()?.value(), false),
            b"CalRGB" => self.calibrated(second()?.value(), true),
            b"DeviceCMYK" => self.device(4, depth, substitute),
            b"DeviceGray" => self.device(1, depth, substitute),
            b"DeviceN" => self.colorants(items, true, depth, substitute),
            b"DeviceRGB" => self.device(3, depth, substitute),
            b"ICCBased" => self.icc_based(&second()?, depth, substitute),
            b"Indexed" => self.indexed(items, depth, substitute),
            b"Lab" => self.lab(second()?.value()),
            b"Separation" => self.colorants(items, false, depth, substitute),
            _ => Err(broken()),
        }
    }

    /// `space` with `start`, the initial colour of the space it stands in for (8.6.8).
    fn standing(&self, space: Rc<Space>, start: &[Fixed]) -> Result<Rc<Space>, PdfError> {
        if space.start == start {
            return Ok(space);
        }
        let mut stand = Space::clone(&space);
        stand.start = start.to_vec();
        self.kept(stand, 0)
    }
}

impl Space {
    /// Its components.
    pub(crate) const fn components(&self) -> usize {
        self.ranges.len()
    }

    /// The `Decode` of `component` by default, of samples up to `most` (8.9.5.2).
    pub(crate) fn default_decode(&self, component: usize, most: u32) -> [Fixed; 2] {
        if self.indexed {
            return [Fixed::default(), Fixed::from_i64(i64::from(most))];
        }
        self.ranges.get(component).copied().unwrap_or_default()
    }

    /// The initial colour, into `out` (8.6.8).
    pub(crate) fn initial(&self, out: &mut [Fixed]) {
        out.fill(Fixed::default());
        for ((slot, &value), &[low, high]) in out.iter_mut().zip(&self.start).zip(&self.ranges) {
            *slot = value.clamp(low, high);
        }
    }

    /// Whether painting in it marks the page (8.6.6.4).
    pub(crate) const fn marks(&self) -> bool {
        self.marks
    }

    /// A space of `ranges` converted by `kind`, zeros its initial colour.
    fn new(ranges: Vec<[Fixed; 2]>, kind: Rc<dyn Convert>) -> Self {
        Self {
            direct: false,
            indexed: false,
            kind,
            marks: true,
            start: vec![Fixed::default(); ranges.len()],
            ranges,
        }
    }

    /// The colour `values` in RGB with `intent`.
    pub(crate) fn rgb(
        &self,
        job: &Job<'_>,
        values: &[Fixed],
        intent: Intent,
    ) -> Result<[u16; 3], PdfError> {
        let count = self.components();
        let mut held = [Fixed::default(); VALUES];
        let (Some(within), Some(given)) = (held.get_mut(..count), values.get(..count)) else {
            return Err(job.objects.malformed());
        };
        for ((slot, &value), &[low, high]) in within.iter_mut().zip(given).zip(&self.ranges) {
            *slot = value.clamp(low, high);
        }
        job.spend(1)?;
        self.kind.rgb(job, within, intent)
    }
}

impl Clear for Held {
    fn clear(&mut self) {
        self.0 = 0;
    }
}

impl Spaces {
    /// The curve of `gamma`.
    fn curve(&mut self, job: &Job<'_>, gamma: Fixed) -> Result<Rc<cie::Curve>, PdfError> {
        if let Some(found) = self.curves.entries().get(&gamma.bits()) {
            return Ok(Rc::clone(found));
        }
        let made = Rc::new(cie::Curve::power(job, gamma)?);
        let size = made.bytes().saturating_add(SPACE_SIZE);
        job.lend(&made, size)?;
        self.make_room(job, size);
        self.curves.offer(job);
        let _none: Option<Rc<cie::Curve>> = self
            .curves
            .entries_mut()
            .insert(gamma.bits(), Rc::clone(&made));
        Ok(made)
    }

    /// The tint transformation `value`.
    fn function(&mut self, job: &Job<'_>, value: &Resolved) -> Result<Rc<Function>, PdfError> {
        let key = value.id().map(|id| id.number);
        if let Some(found) = key.and_then(|number| self.functions.entries().get(&number).cloned()) {
            return Ok(found);
        }
        let made = Function::read(job, value)?;
        job.lend(&made, made.bytes())?;
        if let Some(number) = key {
            self.make_room(job, made.bytes());
            self.functions.offer(job);
            let _none: Option<Rc<Function>> = self
                .functions
                .entries_mut()
                .insert(number, Rc::clone(&made));
        }
        Ok(made)
    }

    /// Makes room in the caches for `size`; one item alone may be larger.
    fn make_room(&mut self, job: &Job<'_>, size: u64) {
        self.held.offer(job);
        let mut held = self.held.entries_mut();
        if held.0.saturating_add(size) > CACHE {
            self.curves.entries_mut().clear();
            self.functions.entries_mut().clear();
            self.profiles.entries_mut().clear();
            held.0 = 0;
        }
        held.0 = held.0.saturating_add(size);
    }

    /// The profile of `components` in `stream`; `None` when unusable, which is warned.
    fn profile(
        &mut self,
        job: &Job<'_>,
        stream: &Resolved,
        components: usize,
    ) -> Result<Option<Rc<icc::Profile>>, PdfError> {
        let key = stream.id().map(|id| (id.number, components));
        if let Some(found) = key.and_then(|known| self.profiles.entries().get(&known).cloned()) {
            return Ok(found);
        }
        let read = data::start(job, stream, PROFILE).and_then(|(bytes, ended)| {
            if ended {
                icc::Profile::read(job, &bytes, components)
            } else {
                Err(job.capped())
            }
        });
        let made = unusable(job, read)?.map(Rc::new);
        let size = made
            .as_ref()
            .map_or(0, |found| found.bytes())
            .saturating_add(SPACE_SIZE);
        if let Some(found) = made.as_ref() {
            job.lend(found, size)?;
        }
        if let Some(known) = key {
            self.make_room(job, size);
            self.profiles.offer(job);
            let _none: Option<Option<Rc<icc::Profile>>> =
                self.profiles.entries_mut().insert(known, made.clone());
        }
        Ok(made)
    }
}

/// The device space of `components`.
fn plain(components: usize) -> Space {
    let one = Fixed::from_i64(1);
    let kind: Rc<dyn Convert> = match components {
        1 => Rc::new(Gray),
        4 => Rc::new(Cmyk),
        _ => Rc::new(Rgb),
    };
    let mut space = Space::new(vec![[Fixed::default(), one]; components], kind);
    space.direct = true;
    if let Some(black) = space.start.get_mut(3) {
        *black = one;
    }
    space
}

/// What `read` gives; a profile that cannot be used is a warning and `None`.
fn unusable<T>(job: &Job<'_>, read: Result<T, PdfError>) -> Result<Option<T>, PdfError> {
    job.tolerate(read, "8.6.5.5", "an ICC profile cannot be used")
}

/// The device space of `components`, or the `scope`'s default for it (8.6.5.6).
pub(crate) fn device(
    job: &Job<'_>,
    spaces: &mut Spaces,
    scope: &Scope,
    components: usize,
) -> Result<Rc<Space>, PdfError> {
    Resolver { job, scope, spaces }.device(components, 0, true)
}

/// The space of the ICC profile `bytes` of `components`, counted while alive, and the bytes it
/// holds; `None` when it is unusable, which is warned.
pub(crate) fn profile(
    job: &Job<'_>,
    bytes: &[u8],
    components: usize,
) -> Result<Option<(Rc<Space>, u64)>, PdfError> {
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let parsed = if size > PROFILE {
        Err(job.capped())
    } else {
        job.reserve(size)?;
        icc::Profile::read(job, bytes, components)
    };
    let Some(read) = unusable(job, parsed)? else {
        return Ok(None);
    };
    let held = read.bytes().saturating_add(SPACE_SIZE);
    let profile = Rc::new(read);
    job.lend(&profile, held)?;
    let ranges = vec![[Fixed::default(), Fixed::from_i64(1)]; components];
    let kind = Rc::new(Based {
        profile,
        ranges: ranges.clone(),
    });
    Ok(Some((Rc::new(Space::new(ranges, kind)), held)))
}

/// The space `value` names or holds; never `Pattern`.
pub(crate) fn resolve(
    job: &Job<'_>,
    spaces: &mut Spaces,
    scope: &Scope,
    value: &Resolved,
) -> Result<Rc<Space>, PdfError> {
    Resolver { job, scope, spaces }.space(value, 0, true)
}
