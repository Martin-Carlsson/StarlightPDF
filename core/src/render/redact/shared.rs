//! Resources redacted pages share with other pages: those they no longer use leave them where no
//! other page uses them either (ISO 32000-2:2020, 7.7.3.4, 7.8.3).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;

use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::page::display;
use crate::part::Leaves;
use crate::pull::Pull;
use crate::render::data::{self, Category, Resolved, Scope};
use crate::render::interpret::content::{self, Joined};
use crate::render::interpret::{Run, inline};
use crate::render::job::{self, Job};
use crate::render::redact::stage::Stage;
use crate::{ErrorCode, PdfError};

/// Work of a page or of content it runs.
const NODE_WORK: u64 = 64;

/// What holds resources of a category.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Holder {
    /// A category dictionary that is its own object.
    Category(ObjectId),
    /// A page or page tree node whose resources are a dictionary within it.
    Node(ObjectId),
    /// A resource dictionary that is its own object.
    Resources(ObjectId),
}

/// The names of a category.
type Names = BTreeSet<Vec<u8>>;

/// What holds names, ordered, and their category.
type Place = ((u8, u32, u16), &'static [u8]);

/// Names redacted pages no longer use, with what holds them and their category, by both.
#[derive(Default)]
pub(super) struct Released {
    /// The names, what holds them and their category.
    entries: BTreeMap<Place, (Holder, Names)>,
    /// What holds each category for the parent of the page asked last: siblings come in turn.
    last: Vec<(ObjectId, &'static [u8], Option<Holder>)>,
}

impl Holder {
    /// It, ordered.
    const fn key(self) -> (u8, u32, u16) {
        match self {
            Self::Category(id) => (0, id.number, id.generation),
            Self::Node(id) => (1, id.number, id.generation),
            Self::Resources(id) => (2, id.number, id.generation),
        }
    }
}

impl Released {
    /// Adds `removed`, the resources the page `page`, read as `dictionary`, no longer uses,
    /// where it shares them.
    pub(super) fn add(
        &mut self,
        job: &Job<'_>,
        stage: &Stage,
        (page, dictionary): (ObjectId, &Object),
        removed: &[(&'static [u8], Vec<u8>)],
    ) -> Result<(), PdfError> {
        let keys: BTreeSet<&'static [u8]> = removed.iter().map(|entry| entry.0).collect();
        for key in keys {
            let Some(held) = self.holder(job, stage, (page, dictionary), key)? else {
                continue;
            };
            if held == Holder::Node(page) {
                continue;
            }
            let names = &mut self
                .entries
                .entry((held.key(), key))
                .or_insert_with(|| (held, Names::new()))
                .1;
            names.extend(
                removed
                    .iter()
                    .filter(|entry| entry.0 == key)
                    .map(|entry| entry.1.clone()),
            );
        }
        Ok(())
    }

    /// What holds the resources of the category `key` of the page `id`, `node`, through its
    /// parents; none where it has none.
    fn holder(
        &mut self,
        job: &Job<'_>,
        stage: &Stage,
        (id, node): (ObjectId, &Object),
        key: &'static [u8],
    ) -> Result<Option<Holder>, PdfError> {
        if let Some(found @ (Object::Reference(_) | Object::Dictionary(_))) = node.get(b"Resources")
        {
            return holding(job, stage, found, Some(Holder::Node(id)), key);
        }
        let Some(Object::Reference(parent)) = node.get(b"Parent") else {
            return Ok(None);
        };
        if let Some(entry) = self
            .last
            .iter()
            .find(|entry| entry.0 == parent && entry.1 == key)
        {
            return Ok(entry.2);
        }
        let found = holder(job, stage, (parent, stage.get(job, parent)?), key)?;
        self.last.retain(|entry| entry.1 != key);
        self.last.push((parent, key, found));
        Ok(found)
    }

    /// Removes the names from what holds them where no page uses them, reading each page once.
    pub(super) fn release(
        mut self,
        run: &mut Run<'_, '_>,
        stage: &mut Stage,
    ) -> Result<(), PdfError> {
        let keys: BTreeSet<&'static [u8]> = self.entries.keys().map(|place| place.1).collect();
        let mut left = self.entries.len();
        let job = run.job;
        let (objects, overlay) = (job.objects, &*job.overlay);
        let (scope, damaged) = (Rc::clone(&run.scope), job.damaged.get());
        let mut pages = Leaves::new(objects, overlay)?;
        while left > 0
            && let Some(id) = pages.next(objects, overlay)?
        {
            job.spend(NODE_WORK)?;
            let node = stage.get(job, id)?;
            for &key in &keys {
                let Some(held) = self.holder(job, stage, (id, &node), key)? else {
                    continue;
                };
                let Some(&mut (_, ref mut names)) = self.entries.get_mut(&(held.key(), key)) else {
                    continue;
                };
                if names.is_empty() {
                    continue;
                }
                // As stored: a redacted page's content uses no more than it did.
                let stored = overlay.resolve(objects, id)?;
                let resources = display::inherited(objects, overlay, &stored, b"Resources")?;
                let streams = display::streams(objects, overlay, stored.get(b"Contents"))?;
                let listing = data::made(job, Object::Array(streams), None)?;
                run.scope = Rc::new(Scope::new(data::made(job, resources, None)?));
                let joined = Box::new(Joined::new(job, listing));
                let result = mentioned(run, stage, (held, key), joined, (&mut *names, 0));
                run.scope = Rc::clone(&scope);
                match result {
                    // Content that cannot be read may use any of them.
                    Err(error) if error.code() == ErrorCode::Malformed => names.clear(),
                    other => other?,
                }
                if names.is_empty() {
                    left = left.saturating_sub(1);
                }
            }
        }
        job.damaged.set(damaged);
        for ((_, key), (held, names)) in self.entries {
            if !names.is_empty() {
                dropped(job, stage, (held, key), &names)?;
            }
        }
        Ok(())
    }
}

/// Removes `names` from the category `key` that `held` holds.
fn dropped(
    job: &Job<'_>,
    stage: &mut Stage,
    (held, key): (Holder, &[u8]),
    names: &Names,
) -> Result<(), PdfError> {
    let without = |mut category: Object| {
        for name in names {
            category.set(name, None);
        }
        category
    };
    let within = |mut resources: Object| {
        let category = resources.get(key).map(without);
        if category.is_some() {
            resources.set(key, category);
        }
        resources
    };
    match held {
        Holder::Category(id) => {
            let category = stage.get(job, id)?;
            stage.set(job, id, without(category))
        }
        Holder::Node(id) => {
            let mut node = stage.get(job, id)?;
            node.set(b"Resources", node.get(b"Resources").map(within));
            stage.set(job, id, node)
        }
        Holder::Resources(id) => {
            let resources = stage.get(job, id)?;
            stage.set(job, id, within(resources))
        }
    }
}

/// What holds the resources of the category `key` of the page tree node `id`, `node`, through
/// its parents; none where it has none.
fn holder(
    job: &Job<'_>,
    stage: &Stage,
    (mut id, mut node): (ObjectId, Object),
    key: &[u8],
) -> Result<Option<Holder>, PdfError> {
    for _ in 0..=job.objects.depth_limit() {
        if let Some(found @ (Object::Reference(_) | Object::Dictionary(_))) = node.get(b"Resources")
        {
            return holding(job, stage, found, Some(Holder::Node(id)), key);
        }
        let Some(Object::Reference(parent)) = node.get(b"Parent") else {
            return Ok(None);
        };
        (id, node) = (parent, stage.get(job, parent)?);
    }
    Err(job.limit())
}

/// What holds the category `key` of `resources`, as stored: `within` where it is a dictionary
/// that holds it within; none where nothing that may be shared does.
fn holding(
    job: &Job<'_>,
    stage: &Stage,
    resources: Object,
    within: Option<Holder>,
    key: &[u8],
) -> Result<Option<Holder>, PdfError> {
    let (found, held) = if let Object::Reference(id) = resources {
        (stage.get(job, id)?, Some(Holder::Resources(id)))
    } else {
        (resources, within)
    };
    Ok(match found.get(key) {
        Some(Object::Reference(id)) => Some(Holder::Category(id)),
        Some(_) | None => held,
    })
}

/// Removes from `names` those that `data` names, `depth` deep, or content it runs through the
/// same resources names: forms, Type 3 glyph procedures, soft mask groups and tiling pattern
/// cells that have none of their own or those `held` holds; all past the deepest (7.8.2, 8.7.3.1,
/// 8.10.1, 9.6.4, 11.6.5.2).
fn mentioned(
    run: &mut Run<'_, '_>,
    stage: &Stage,
    (held, key): (Holder, &[u8]),
    data: Box<dyn Pull + '_>,
    (names, depth): (&mut Names, u8),
) -> Result<(), PdfError> {
    let job = run.job;
    if depth >= job::NESTING {
        names.clear();
        return Ok(());
    }
    let ending: Box<dyn Pull + '_> = Box::new(content::Ending::new(job, data));
    let mut operations = Box::new(content::Operations::new(ending));
    let mut operands = Vec::new();
    while let Some(name) = operations.next(job, &mut operands)? {
        for operand in &operands {
            if let Some(found) = data::name(operand) {
                let _named: bool = names.remove(found);
            }
        }
        if names.is_empty() {
            return Ok(());
        }
        let category = match name.bytes() {
            b"BI" => {
                inline::pass(run, &mut operations)?;
                continue;
            }
            b"Do" => Category::XObject,
            b"Tf" => Category::Font,
            b"gs" => Category::ExtGState,
            b"SCN" | b"scn" => Category::Pattern,
            _ => continue,
        };
        let Some(label) = operands.iter().find_map(data::name) else {
            continue;
        };
        let Some(found) = data::named(job, &run.scope, category, label)? else {
            continue;
        };
        for (owner, streams) in runs(job, &found)? {
            job.spend(NODE_WORK)?;
            let own = data::entry(job, owner.value(), b"Resources")?;
            let inherits = !matches!(*own.value(), Object::Dictionary(_));
            if !inherits {
                let stored = owner.value().get(b"Resources").unwrap_or(Object::Null);
                if holding(job, stage, stored, None, key)? != Some(held) {
                    continue;
                }
            }
            let scope = Rc::clone(&run.scope);
            if !inherits {
                run.scope = Rc::new(Scope::new(own));
            }
            let mut result = Ok(());
            for stream in streams {
                let opened = data::open(job, &mut run.caches.streams, &stream)?;
                result = mentioned(
                    run,
                    stage,
                    (held, key),
                    opened,
                    (&mut *names, depth.saturating_add(1)),
                );
                if result.is_err() {
                    break;
                }
            }
            run.scope = scope;
            result?;
        }
    }
    Ok(())
}

/// `found` and its glyph procedures where it is a Type 3 font (9.6.4).
fn procedures(
    job: &Job<'_>,
    found: &Resolved,
) -> Result<Option<(Resolved, Vec<Resolved>)>, PdfError> {
    let kind = data::entry(job, found.value(), b"Subtype")?;
    if data::name(kind.value()) != Some(b"Type3") {
        return Ok(None);
    }
    let listed = data::entry(job, found.value(), b"CharProcs")?;
    let mut streams = Vec::new();
    for pair in data::pairs(listed.value()).map_or(&[][..], Vec::as_slice) {
        job.spend(1)?;
        streams.push(data::resolve(job, &pair.1)?);
    }
    Ok(Some((found.clone(), streams)))
}

/// What runs content that `found`, a resource, names, and its streams: a form, a Type 3 font's
/// glyph procedures, a soft mask's group or font of a graphics state, a tiling pattern
/// (8.4.5, 8.7.3.1, 8.10.1, 9.6.4, 11.6.5.2).
fn runs(job: &Job<'_>, found: &Resolved) -> Result<Vec<(Resolved, Vec<Resolved>)>, PdfError> {
    let value = found.value();
    let kind = data::entry(job, value, b"Subtype")?;
    let mut made: Vec<(Resolved, Vec<Resolved>)> = procedures(job, found)?.into_iter().collect();
    if data::name(kind.value()) == Some(b"Form") {
        made.push((found.clone(), vec![found.clone()]));
    }
    if matches!(
        *data::entry(job, value, b"PatternType")?.value(),
        Object::Integer(1)
    ) {
        made.push((found.clone(), vec![found.clone()]));
    }
    let mask = data::entry(job, value, b"SMask")?;
    let group = data::entry(job, mask.value(), b"G")?;
    if matches!(*group.value(), Object::Stream(..)) {
        made.push((group.clone(), vec![group]));
    }
    let font = data::entry(job, value, b"Font")?;
    if let Some(first) = data::items(font.value()).and_then(<[Object]>::first) {
        made.extend(procedures(job, &data::resolve(job, first)?)?);
    }
    Ok(made)
}
