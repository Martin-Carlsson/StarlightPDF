//! Optional content (ISO 32000-2:2020, 8.11).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use core::slice;

use crate::PdfError;
use crate::overlay::Object;
use crate::render::data::{self, Category, Resolved};
use crate::render::interpret::Run;
use crate::render::job::Job;
use crate::syntax::Value;

/// How deep a visibility expression nests, the policy permitting.
const DEPTH: u32 = 64;

/// The longest name a reader takes (Annex C): no intent is longer.
const NAME: usize = 127;

/// The most cached answers.
const KNOWN: usize = 4096;

/// The most parts of a visibility expression.
const PARTS: u32 = 4096;

/// The bytes a node of a set of numbers takes at most.
const NODE: u64 = 16;

/// The default configuration (8.11.4.3); by default everything is visible.
#[derive(Default)]
struct Configuration {
    /// `BaseState` is `OFF`.
    base_off: bool,
    /// Its intents; no group without one of them has an effect.
    intents: Vec<Vec<u8>>,
    /// `OFF`, which wins over `ON`.
    off: BTreeSet<u32>,
    /// `ON`.
    on: BTreeSet<u32>,
}

/// The visibility found, by object number.
#[derive(Default)]
pub(crate) struct Layers {
    /// The configuration, once read.
    configuration: Option<Rc<Configuration>>,
    /// Whether each is visible.
    known: BTreeMap<u32, bool>,
}

/// One stream's marked-content levels, counted, as content alone bounds them.
#[derive(Default)]
pub(crate) struct Marked {
    /// How many are open.
    depth: u64,
    /// The level that hides.
    hiding: Option<u64>,
}

impl Configuration {
    /// The bytes it holds.
    fn bytes(&self) -> u64 {
        let intents = self.intents.iter().fold(0_u64, |total, intent| {
            let size = intent.len().saturating_add(size_of::<Vec<u8>>());
            total.saturating_add(u64::try_from(size).unwrap_or(u64::MAX))
        });
        let numbers = self.on.len().saturating_add(self.off.len());
        u64::try_from(numbers)
            .unwrap_or(u64::MAX)
            .saturating_mul(NODE)
            .saturating_add(intents)
    }
}

/// Whether the `OC` of `owner` lets it show (8.11.3.3).
pub(crate) fn allows(run: &mut Run<'_, '_>, owner: &Object) -> Result<bool, PdfError> {
    let member = data::entry(run.job, owner, b"OC")?;
    if matches!(*member.value(), Object::Null) {
        return Ok(true);
    }
    visible(run, &member)
}

/// The document's default configuration.
fn configuration(job: &Job<'_>) -> Result<Configuration, PdfError> {
    let properties = if job.overlay.is_changed() {
        let trailer = Object::Dictionary(job.overlay.trailer(job.objects)?);
        let catalog = data::entry(job, &trailer, b"Root")?;
        data::entry(job, catalog.value(), b"OCProperties")?
    } else {
        // Only that entry of the stored catalog is read.
        let objects = job.objects;
        let catalog = objects.catalog()?;
        let at = objects.dictionary(Some(Value::Reference(catalog)))?;
        let found = objects.owned_entry(catalog, at, b"OCProperties")?;
        data::resolved(job, found.unwrap_or(Object::Null))?
    };
    let chosen = data::entry(job, properties.value(), b"D")?;
    if matches!(*chosen.value(), Object::Null) {
        return Ok(Configuration::default());
    }
    let base = data::entry(job, chosen.value(), b"BaseState")?;
    Ok(Configuration {
        base_off: matches!(base.value(), Object::Name(name) if name == b"OFF"),
        intents: intents(job, chosen.value())?,
        off: numbered(job, chosen.value(), b"OFF")?,
        on: numbered(job, chosen.value(), b"ON")?,
    })
}

/// The value of the visibility expression `given`, `depth` more levels allowed (8.11.2.2).
fn expressed(
    job: &Job<'_>,
    configuration: &Configuration,
    given: &Object,
    depth: u32,
    left: &mut u32,
) -> Result<bool, PdfError> {
    if depth == 0 || *left == 0 {
        return Err(job.capped());
    }
    *left = left.saturating_sub(1);
    // Each operand takes a part: one too long for those left is not read.
    let most = usize::try_from(*left)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    if let Object::Reference(id) = *given
        && !job.overlay.is_changed()
        && job.objects.longer(id, most)?
    {
        return Err(job.capped());
    }
    let value = &data::resolve(job, given)?;
    if !matches!(*value.value(), Object::Array(_)) {
        return group(job, configuration, value);
    }
    let items = data::items(value.value()).ok_or_else(|| job.objects.malformed())?;
    if items.len() > most {
        return Err(job.capped());
    }
    let (first, parts) = items.split_first().ok_or_else(|| job.objects.malformed())?;
    let head = data::resolve(job, first)?;
    let operator = data::name(head.value()).ok_or_else(|| job.objects.malformed())?;
    let (mut count, mut all, mut any) = (0_u32, true, false);
    for part in parts {
        let shown = expressed(job, configuration, part, depth.saturating_sub(1), left)?;
        count = count.saturating_add(1);
        all &= shown;
        any |= shown;
    }
    match (operator, count) {
        (b"Not", 1) => Ok(!any),
        (b"And", 1..) => Ok(all),
        (b"Or", 1..) => Ok(any),
        _ => Err(job.objects.malformed()),
    }
}

/// Whether the group `found` is visible.
fn group(job: &Job<'_>, configuration: &Configuration, found: &Resolved) -> Result<bool, PdfError> {
    if !matches!(*found.value(), Object::Dictionary(_)) {
        return Err(job.objects.malformed());
    }
    let Some(number) = found.id().map(|id| id.number) else {
        return Ok(true);
    };
    let off = configuration.off.contains(&number)
        || (configuration.base_off && !configuration.on.contains(&number));
    if !off {
        return Ok(true);
    }
    let own = intents(job, found.value())?;
    let compared = configuration.intents.len().saturating_mul(own.len());
    job.spend(u64::try_from(compared).unwrap_or(u64::MAX))?;
    // A group of another intent has no effect (8.11.2.3).
    let concerns = configuration
        .intents
        .iter()
        .any(|intent| intent == b"All" || own.contains(intent));
    Ok(!concerns)
}

/// The `Intent` of `owner`, `View` by default; names past [`NAME`] bytes, which no intent has,
/// are left out, and those kept are checked beside all that is held.
fn intents(job: &Job<'_>, owner: &Object) -> Result<Vec<Vec<u8>>, PdfError> {
    let listed = data::entry(job, owner, b"Intent")?;
    if matches!(*listed.value(), Object::Null) {
        return Ok(vec![b"View".to_vec()]);
    }
    let items = if let Some(items) = data::items(listed.value()) {
        items
    } else if data::name(listed.value()).is_some() {
        slice::from_ref(listed.value())
    } else {
        return Err(job.objects.malformed());
    };
    let mut found = Vec::new();
    let mut total = 0_u64;
    for item in items {
        job.spend(1)?;
        let one = data::resolve(job, item)?;
        if let Some(name) = data::name(one.value()).filter(|name| name.len() <= NAME) {
            let size = name.len().saturating_add(size_of::<Vec<u8>>());
            total = total.saturating_add(u64::try_from(size).unwrap_or(u64::MAX));
            job.fits(total)?;
            found.push(name.to_vec());
        }
    }
    Ok(found)
}

/// Runs a marked-content operator (8.11.3.2, 14.6).
pub(crate) fn marked(
    run: &mut Run<'_, '_>,
    levels: &mut Marked,
    name: &[u8],
    operands: &[Object],
) -> Result<(), PdfError> {
    let job = run.job;
    if name == b"EMC" {
        if levels.depth == 0 {
            return job.warn("14.6", "EMC without BMC or BDC");
        }
        if levels.hiding == Some(levels.depth) {
            levels.hiding = None;
            run.hidden = run.hidden.saturating_sub(1);
        }
        levels.depth = levels.depth.saturating_sub(1);
        return Ok(());
    }
    if name != b"BDC" && name != b"BMC" {
        return Ok(());
    }
    levels.depth = levels.depth.saturating_add(1);
    if name == b"BMC" || !matches!(operands.first(), Some(Object::Name(tag)) if tag == b"OC") {
        return Ok(());
    }
    let found = match operands.get(1).cloned() {
        Some(Object::Name(key)) => data::named(job, &run.scope, Category::Properties, &key)?,
        Some(listed @ Object::Dictionary(_)) => Some(data::resolved(job, listed)?),
        Some(_) | None => return Err(job.objects.malformed()),
    };
    let Some(membership) = found else {
        return job.warn("7.8.3", "optional content is missing");
    };
    if levels.hiding.is_none() && !visible(run, &membership)? {
        levels.hiding = Some(levels.depth);
        run.hidden = run.hidden.saturating_add(1);
    }
    Ok(())
}

/// Whether the membership dictionary `owner` lets content show (8.11.2.2).
fn member(job: &Job<'_>, configuration: &Configuration, owner: &Object) -> Result<bool, PdfError> {
    let expression = data::stored(owner, b"VE");
    if !matches!(expression, Object::Null) {
        let depth = job.objects.depth_limit().min(DEPTH);
        let mut left = PARTS;
        return expressed(job, configuration, expression, depth, &mut left);
    }
    let listed = data::entry(job, owner, b"OCGs")?;
    let (mut any_on, mut any_off) = (false, false);
    let mut count = |found: &Resolved| -> Result<(), PdfError> {
        if !matches!(*found.value(), Object::Null) {
            let shown = group(job, configuration, found)?;
            any_on |= shown;
            any_off |= !shown;
        }
        Ok(())
    };
    if let Some(items) = data::items(listed.value()) {
        // One at a time, as there can be many.
        for item in items {
            job.spend(1)?;
            count(&data::resolve(job, item)?)?;
        }
    } else {
        count(&listed)?;
    }
    if !any_on && !any_off {
        return Ok(true);
    }
    let policy = data::entry(job, owner, b"P")?;
    let is = |wanted: &[u8]| data::name(policy.value()) == Some(wanted);
    Ok(if is(b"AllOn") {
        !any_off
    } else if is(b"AllOff") {
        !any_on
    } else if is(b"AnyOff") {
        any_off
    } else {
        any_on
    })
}

/// The object numbers of the groups the array `key` of `owner` lists.
fn numbered(job: &Job<'_>, owner: &Object, key: &[u8]) -> Result<BTreeSet<u32>, PdfError> {
    let mut numbers = BTreeSet::new();
    let listed = data::entry(job, owner, key)?;
    if matches!(*listed.value(), Object::Null) {
        return Ok(numbers);
    }
    let items = data::items(listed.value()).ok_or_else(|| job.objects.malformed())?;
    job.fits(
        u64::try_from(items.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(NODE),
    )?;
    for item in items {
        job.spend(1)?;
        numbers.extend(data::resolve(job, item)?.id().map(|id| id.number));
    }
    Ok(numbers)
}

/// Whether `membership`, a group or a membership dictionary, is visible by default (8.11.4).
pub(crate) fn visible(run: &mut Run<'_, '_>, membership: &Resolved) -> Result<bool, PdfError> {
    let job = run.job;
    let number = membership.id().map(|id| id.number);
    if let Some(known) = number.and_then(|key| run.caches.layers.known.get(&key)) {
        return Ok(*known);
    }
    let configured = if let Some(read) = run.caches.layers.configuration.clone() {
        read
    } else {
        let read = job.tolerate(
            configuration(job),
            "8.11.4.2",
            "the optional content configuration is malformed",
        )?;
        let shared = Rc::new(read.unwrap_or_default());
        job.lend(&shared, shared.bytes())?;
        run.caches.layers.configuration = Some(Rc::clone(&shared));
        shared
    };
    let kind = data::entry(job, membership.value(), b"Type")?;
    let shown = if matches!(kind.value(), Object::Name(name) if name == b"OCMD") {
        member(job, &configured, membership.value())?
    } else {
        group(job, &configured, membership)?
    };
    if let Some(key) = number {
        let known = &mut run.caches.layers.known;
        if known.len() >= KNOWN {
            known.clear();
        }
        let _none: Option<bool> = known.insert(key, shown);
    }
    Ok(shown)
}
