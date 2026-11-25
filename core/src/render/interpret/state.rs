//! The graphics state and its operators (ISO 32000-2:2020, 8.4).

use alloc::rc::Rc;
use core::slice::from_ref;

use crate::PdfError;
use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::render::colour::{Intent, Space};
use crate::render::data::{self, Category, Resolved};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Area, Matrix};
use crate::render::interpret::{Run, mask, name_operand, numbers, paint};
use crate::render::job::{self, Job};
use crate::render::stroke::{Cap, Join, Style};
use crate::render::{blend, surface, text};

/// What fills or strokes.
#[derive(Clone)]
pub(crate) enum Fill {
    /// Nothing.
    None,
    /// A pattern.
    Pattern(Rc<Patterned>),
    /// A colour.
    Solid([u16; 3]),
}

/// What a pattern takes from how the stream that selects it began (8.7.2, 8.7.3.1).
pub(crate) struct Parent {
    /// How many streams were being run, it among them.
    pub active: usize,
    /// `AIS`.
    pub alpha_is_shape: bool,
    /// What fills.
    pub fill: Fill,
    /// The space `sc` fills in.
    pub fill_space: Selected,
    /// The rendering intent.
    pub intent: Intent,
    /// The map from pattern space's base to pixels.
    pub matrix: Matrix,
    /// The stream's number in the render.
    pub serial: u64,
    /// What strokes.
    pub stroke: Fill,
    /// The space `SC` strokes in.
    pub stroke_space: Selected,
    /// How lines are stroked.
    pub style: Style,
    /// The text state.
    pub text: text::Settings,
}

/// A pattern as the colour operators set it.
pub(crate) struct Patterned {
    /// The colour of an uncoloured pattern.
    pub color: Option<[u16; 3]>,
    /// How the stream that selected it began.
    pub parent: Option<Rc<Parent>>,
    /// The pattern, or a reference to it.
    pub pattern: Resolved,
}

/// A colour space as the colour operators selected it.
#[derive(Clone, Default)]
pub(crate) enum Selected {
    /// `DeviceGray`, as content begins.
    #[default]
    Initial,
    /// One that cannot be used, which paints nothing.
    Lost,
    /// `Pattern`, and the colour in the space below it.
    Pattern(Option<Rc<Tint>>),
    /// A colour in a space.
    Space(Rc<Tint>),
}

/// The graphics state.
#[derive(Clone)]
pub(crate) struct State {
    /// `AIS`.
    pub alpha_is_shape: bool,
    /// The blend mode.
    pub blend: blend::Mode,
    /// The clip.
    pub clip: surface::Clip,
    /// What fills.
    pub fill: Fill,
    /// `ca`.
    pub fill_alpha: u16,
    /// The space `sc` fills in.
    pub fill_space: Selected,
    /// The rendering intent.
    pub intent: Intent,
    /// The map from user space to pixels.
    pub matrix: Matrix,
    /// How the stream being run began; none before one does.
    pub parent: Option<Rc<Parent>>,
    /// The soft mask.
    pub soft: Option<surface::Soft>,
    /// What set it and under which map.
    pub soft_source: Option<(ObjectId, [Fixed; 6])>,
    /// What strokes.
    pub stroke: Fill,
    /// `CA`.
    pub stroke_alpha: u16,
    /// The space `SC` strokes in.
    pub stroke_space: Selected,
    /// How lines are stroked.
    pub style: Style,
    /// The text state.
    pub text: text::Settings,
}

/// A colour as set, converted with the intent in force (8.6.5.8).
pub(crate) struct Tint {
    /// Its space.
    pub space: Rc<Space>,
    /// Its values; none before a pattern's is set.
    pub values: Vec<Fixed>,
}

impl Parent {
    /// Gives `state` what the pattern begins with, transparency aside (8.7.3.1, 8.7.4.2, 11.6.7).
    pub(crate) fn hand_down(&self, state: &mut State) {
        state.alpha_is_shape = self.alpha_is_shape;
        state.fill.clone_from(&self.fill);
        state.fill_space.clone_from(&self.fill_space);
        state.intent = self.intent;
        state.stroke.clone_from(&self.stroke);
        state.stroke_space.clone_from(&self.stroke_space);
        state.style.clone_from(&self.style);
        state.text.clone_from(&self.text);
    }
}

impl State {
    /// Makes this state the one stream `serial` began with, `active` streams being run.
    pub(crate) fn begin(&mut self, active: usize, serial: u64) {
        self.parent = Some(Rc::new(Parent {
            active,
            alpha_is_shape: self.alpha_is_shape,
            fill: self.fill.clone(),
            fill_space: self.fill_space.clone(),
            intent: self.intent,
            matrix: self.matrix,
            serial,
            stroke: self.stroke.clone(),
            stroke_space: self.stroke_space.clone(),
            style: self.style.clone(),
            text: self.text.clone(),
        }));
    }

    /// The initial state under `matrix`, clipped to `area` (8.4.1).
    pub(crate) fn new(matrix: Matrix, area: Area) -> Self {
        let (none, one) = (Fixed::default(), Fixed::from_i64(1));
        Self {
            alpha_is_shape: false,
            blend: blend::Mode::default(),
            clip: surface::Clip { area, mask: None },
            fill: Fill::Solid([0; 3]),
            fill_alpha: u16::MAX,
            fill_space: Selected::default(),
            intent: Intent::default(),
            matrix,
            parent: None,
            soft: None,
            soft_source: None,
            stroke: Fill::Solid([0; 3]),
            stroke_alpha: u16::MAX,
            stroke_space: Selected::default(),
            style: Style {
                cap: Cap::Butt,
                dash: Rc::from(Vec::new()),
                join: Join::Miter,
                miter: Fixed::from_i64(10),
                phase: none,
                width: one,
            },
            text: text::Settings {
                character: none,
                font: None,
                knockout: true,
                leading: none,
                mode: 0,
                rise: none,
                scale: one,
                size: none,
                word: none,
            },
        }
    }
}

/// The line cap `value` (8.4.3.3).
fn cap(job: &Job<'_>, value: &Object) -> Result<Cap, PdfError> {
    match Fixed::from_object(value).map(Fixed::floor) {
        Some(0) => Ok(Cap::Butt),
        Some(1) => Ok(Cap::Round),
        Some(2) => Ok(Cap::Square),
        Some(_) | None => Err(job.objects.malformed()),
    }
}

/// Sets the dashes `dashes` from `phase` (8.4.3.6).
fn dash(run: &mut Run<'_, '_>, dashes: &Object, phase: &Object) -> Result<(), PdfError> {
    let malformed = || run.job.objects.malformed();
    let items = data::items(dashes).ok_or_else(malformed)?;
    // Each saved state keeps its own.
    if items.len() > job::DASHES {
        return Err(run.job.capped());
    }
    let lengths = items
        .iter()
        .map(|item| Fixed::from_object(item).ok_or_else(malformed))
        .collect::<Result<Vec<_>, _>>()?;
    let start = Fixed::from_object(phase).ok_or_else(malformed)?;
    run.state.style.dash = Rc::from(lengths);
    run.state.style.phase = start;
    Ok(())
}

/// The rendering intent `name`, the default for an unknown one (8.6.5.8).
pub(crate) const fn intent(name: &[u8]) -> Intent {
    match name {
        b"AbsoluteColorimetric" => Intent::Absolute,
        b"Perceptual" => Intent::Perceptual,
        b"Saturation" => Intent::Saturation,
        _ => Intent::Relative,
    }
}

/// The line join `value` (8.4.3.4).
fn join(job: &Job<'_>, value: &Object) -> Result<Join, PdfError> {
    match Fixed::from_object(value).map(Fixed::floor) {
        Some(0) => Ok(Join::Miter),
        Some(1) => Ok(Join::Round),
        Some(2) => Ok(Join::Bevel),
        Some(_) | None => Err(job.objects.malformed()),
    }
}

/// The blend mode `value` names, of an array the first known; else Normal (11.3.5).
pub(crate) fn mode(value: &Object) -> blend::Mode {
    let known = |item: &Object| data::name(item).and_then(blend::named);
    let found =
        data::items(value).map_or_else(|| known(value), |items| items.iter().find_map(known));
    found.unwrap_or_default()
}

/// Runs an operator of the graphics or text state.
pub(crate) fn operator(
    run: &mut Run<'_, '_>,
    saved: &mut Vec<State>,
    name: &[u8],
    operands: &[Object],
) -> Result<(), PdfError> {
    let job = run.job;
    let first = operands.first().unwrap_or(&Object::Null);
    let last = operands.last().unwrap_or(&Object::Null);
    let number = || numbers::<1>(job, operands).map(|[value]| value);
    match name {
        b"q" => {
            // Fewer deeper, so nesting does not multiply them.
            if saved.len() >= job::SAVED >> run.depth.min(6) {
                return Err(job.capped());
            }
            saved.push(run.state.clone());
        }
        b"Q" => match saved.pop() {
            Some(state) => run.state = state,
            None => job.warn("8.4.2", "Q without q")?,
        },
        b"cm" => {
            let terms = numbers(job, operands)?;
            run.state.matrix = Matrix { terms }.then(&run.state.matrix);
        }
        b"d" => dash(run, first, last)?,
        b"gs" => set(run, name_operand(job, operands, 0)?)?,
        b"J" => run.state.style.cap = cap(job, first)?,
        b"j" => run.state.style.join = join(job, first)?,
        b"M" => run.state.style.miter = number()?,
        b"ri" => {
            run.state.intent = intent(name_operand(job, operands, 0)?);
            paint::retint(run)?;
        }
        b"w" => run.state.style.width = number()?,
        _ => text_state(run, name, operands)?,
    }
    Ok(())
}

/// Sets the parameter `key` of the dictionary `source` to `listed` (8.4.5).
fn parameter(
    run: &mut Run<'_, '_>,
    source: Option<ObjectId>,
    key: &[u8],
    listed: &Object,
) -> Result<(), PdfError> {
    if !matches!(
        key,
        b"AIS"
            | b"BM"
            | b"ca"
            | b"CA"
            | b"D"
            | b"Font"
            | b"LC"
            | b"LJ"
            | b"LW"
            | b"ML"
            | b"RI"
            | b"SMask"
            | b"TK"
    ) {
        return Ok(());
    }
    let job = run.job;
    // Only now, once the key is known, is it copied.
    let found = data::resolve(job, listed)?;
    let value = found.value();
    if matches!(value, Object::Null) {
        return Ok(());
    }
    let number = || Fixed::from_object(value).ok_or_else(|| job.objects.malformed());
    let flag = || {
        if matches!(value, Object::Boolean(_)) {
            Ok(matches!(value, Object::Boolean(true)))
        } else {
            Err(job.objects.malformed())
        }
    };
    match key {
        b"AIS" => run.state.alpha_is_shape = flag()?,
        b"BM" => run.state.blend = mode(value),
        b"ca" => run.state.fill_alpha = number()?.unit(),
        b"CA" => run.state.stroke_alpha = number()?.unit(),
        b"LC" => run.state.style.cap = cap(job, value)?,
        b"LJ" => run.state.style.join = join(job, value)?,
        b"LW" => run.state.style.width = number()?,
        b"ML" => run.state.style.miter = number()?,
        b"RI" => {
            run.state.intent = intent(name_operand(job, from_ref(value), 0)?);
            paint::retint(run)?;
        }
        b"SMask" => soft(run, found.id().or(source), &found)?,
        b"TK" => run.state.text.knockout = flag()?,
        // D and Font.
        _ => {
            let (first, second) = pair(job, value)?;
            if key == b"D" {
                dash(run, first.value(), second.value())?;
            } else {
                let size =
                    Fixed::from_object(second.value()).ok_or_else(|| job.objects.malformed())?;
                run.state.text.font = None;
                text::set_font(run, (&first, None), size)?;
            }
        }
    }
    Ok(())
}

/// The two items of the array `value`.
fn pair(job: &Job<'_>, value: &Object) -> Result<(Resolved, Resolved), PdfError> {
    let items = data::items(value).ok_or_else(|| job.objects.malformed())?;
    let (Some(first), Some(second)) = (items.first(), items.get(1)) else {
        return Err(job.objects.malformed());
    };
    Ok((data::resolve(job, first)?, data::resolve(job, second)?))
}

/// Sets the parameters of the `ExtGState` resource `key`, each alone (8.4.5).
fn set(run: &mut Run<'_, '_>, key: &[u8]) -> Result<(), PdfError> {
    let job = run.job;
    let Some(found) = data::named(job, &run.scope, Category::ExtGState, key)? else {
        return job.warn("7.8.3", "a graphics state dictionary is missing");
    };
    applied(run, &found)
}

/// Sets the parameters of the `ExtGState` dictionary `found`, each alone (8.4.5).
pub(crate) fn applied(run: &mut Run<'_, '_>, found: &Resolved) -> Result<(), PdfError> {
    let job = run.job;
    let source = found.id();
    // Held, and counted, while its parameters are set.
    let entries = data::pairs(found.value()).ok_or_else(|| job.objects.malformed())?;
    for pair in entries {
        job.spend(1)?;
        let done = parameter(run, source, &pair.0, &pair.1);
        let _set: Option<()> =
            job.tolerate(done, "8.4.5", "a graphics state parameter is malformed")?;
    }
    Ok(())
}

/// Sets the soft mask `value` from `source`; the one the state has from there is kept.
fn soft(run: &mut Run<'_, '_>, source: Option<ObjectId>, value: &Resolved) -> Result<(), PdfError> {
    let origin = source.map(|id| (id, run.state.matrix.terms));
    if run.state.soft.is_some() && origin.is_some() && origin == run.state.soft_source {
        return Ok(());
    }
    (run.state.soft, run.state.soft_source) = (None, None);
    run.state.soft = mask::set(run, value)?;
    run.state.soft_source = origin;
    Ok(())
}

/// Runs an operator of the text state (9.3).
fn text_state(run: &mut Run<'_, '_>, name: &[u8], operands: &[Object]) -> Result<(), PdfError> {
    let job = run.job;
    let number = || numbers::<1>(job, operands).map(|[value]| value);
    match name {
        b"Tc" => run.state.text.character = number()?,
        b"TL" => run.state.text.leading = number()?,
        b"Tr" => {
            run.state.text.mode = u8::try_from(number()?.floor())
                .ok()
                .filter(|mode| *mode < 8)
                .ok_or_else(|| job.objects.malformed())?;
        }
        b"Ts" => run.state.text.rise = number()?,
        b"Tw" => run.state.text.word = number()?,
        b"Tz" => {
            let percent = number()?.over(Fixed::from_i64(100));
            run.state.text.scale = percent.ok_or_else(|| job.objects.malformed())?;
        }
        b"Tf" => {
            let size = operands
                .last()
                .and_then(Fixed::from_object)
                .ok_or_else(|| job.objects.malformed())?;
            let key = name_operand(job, operands, 0)?;
            // Text must not go on in the font before.
            run.state.text.font = None;
            run.state.text.size = size;
            match data::named(job, &run.scope, Category::Font, key)? {
                Some(font) => text::set_font(run, (&font, Some(key)), size)?,
                None => job.warn("7.8.3", "a font is missing")?,
            }
        }
        // `i`: flatness is the renderer's.
        _ => {}
    }
    Ok(())
}
