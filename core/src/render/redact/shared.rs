//! Resources a redacted page shares with other pages: those it no longer uses leave them where no
//! other page uses them either (ISO 32000-2:2020, 7.7.3.4, 7.8.3).

use alloc::collections::BTreeSet;
use alloc::rc::Rc;

use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::page::display;
use crate::pull::Pull;
use crate::render::data::{self, Category, Scope};
use crate::render::interpret::content::{self, Joined};
use crate::render::interpret::{Run, inline};
use crate::render::job;
use crate::render::redact::stage::Stage;
use crate::{ErrorCode, PdfError};

/// Work of a node of the page tree.
const NODE_WORK: u64 = 64;

/// What holds resources of a category.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Holder {
    /// A category dictionary that is its own object.
    Category(ObjectId),
    /// A page, page tree node or form whose resources are a dictionary within it.
    Node(ObjectId),
    /// A resource dictionary that is its own object.
    Resources(ObjectId),
}

/// The names of the category `key`.
type Names = BTreeSet<Vec<u8>>;

/// Removes those of `names` from the category `key` that `held` holds.
fn dropped(
    run: &Run<'_, '_>,
    stage: &mut Stage,
    (held, key): (Holder, &[u8]),
    names: &Names,
) -> Result<(), PdfError> {
    let job = run.job;
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

/// What holds the resources of the category `key` of `node`, object `id`, through its parents;
/// none where it has none.
fn holder(
    run: &Run<'_, '_>,
    stage: &Stage,
    (mut id, mut node): (ObjectId, Object),
    key: &[u8],
) -> Result<Option<Holder>, PdfError> {
    let job = run.job;
    for _ in 0..=job.objects.depth_limit() {
        let (resources, held) = match node.get(b"Resources") {
            Some(Object::Reference(found)) => (stage.get(job, found)?, Holder::Resources(found)),
            Some(found @ Object::Dictionary(_)) => (found, Holder::Node(id)),
            Some(_) | None => {
                let Some(Object::Reference(parent)) = node.get(b"Parent") else {
                    return Ok(None);
                };
                (id, node) = (parent, stage.get(job, parent)?);
                continue;
            }
        };
        return Ok(Some(match resources.get(key) {
            Some(Object::Reference(found)) => Holder::Category(found),
            Some(_) | None => held,
        }));
    }
    Err(job.limit())
}

/// Removes from `names` those that `data` names, `depth` forms deep, or a form it draws whose
/// resources `held` holds or that has none; all past the deepest (7.8.2, 8.10.1).
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
        if name.bytes() == b"BI" {
            inline::pass(run, &mut operations)?;
            continue;
        }
        let Some(label) = operands
            .last()
            .and_then(data::name)
            .filter(|_| name.bytes() == b"Do")
        else {
            continue;
        };
        let Some(form) = data::named(job, &run.scope, Category::XObject, label)? else {
            continue;
        };
        let kind = data::entry(job, form.value(), b"Subtype")?;
        let Some(id) = form
            .id()
            .filter(|_| data::name(kind.value()) == Some(b"Form"))
        else {
            continue;
        };
        let own = holder(run, stage, (id, form.value().clone()), key)?;
        if own.is_some_and(|found| found != held) {
            continue;
        }
        let resources = data::entry(job, form.value(), b"Resources")?;
        let opened = data::open(job, &mut run.caches.streams, &form)?;
        let scope = Rc::clone(&run.scope);
        if own.is_some() {
            run.scope = Rc::new(Scope::new(resources));
        }
        let result = mentioned(
            run,
            stage,
            (held, key),
            opened,
            (names, depth.saturating_add(1)),
        );
        run.scope = scope;
        result?;
    }
    Ok(())
}

/// Removes `removed`, the resources the page `page`, read as `dictionary`, no longer uses, from
/// what it shares them with other pages through, where no other page uses them.
pub(super) fn release(
    run: &mut Run<'_, '_>,
    stage: &mut Stage,
    (page, dictionary): (ObjectId, &Object),
    removed: &[(&'static [u8], Vec<u8>)],
) -> Result<(), PdfError> {
    let keys: BTreeSet<&[u8]> = removed.iter().map(|entry| entry.0).collect();
    for key in keys {
        let Some(held) = holder(run, stage, (page, dictionary.clone()), key)? else {
            continue;
        };
        if held == Holder::Node(page) {
            continue;
        }
        let mut names: Names = removed
            .iter()
            .filter(|entry| entry.0 == key)
            .map(|entry| entry.1.clone())
            .collect();
        unused(run, stage, (page, held, key), &mut names)?;
        if !names.is_empty() {
            dropped(run, stage, (held, key), &names)?;
        }
    }
    Ok(())
}

/// Removes from `names` those that a page but `page` whose resources of the category `key`
/// `held` holds uses.
fn unused(
    run: &mut Run<'_, '_>,
    stage: &Stage,
    (page, held, key): (ObjectId, Holder, &[u8]),
    names: &mut Names,
) -> Result<(), PdfError> {
    let job = run.job;
    let (objects, overlay) = (job.objects, &*job.overlay);
    let trailer = Object::Dictionary(overlay.trailer(objects)?);
    let catalog = overlay.entry(objects, &trailer, b"Root")?;
    let mut pending: Vec<Object> = catalog.get(b"Pages").into_iter().collect();
    let mut seen = BTreeSet::new();
    let (scope, damaged) = (Rc::clone(&run.scope), job.damaged.get());
    while let Some(next) = pending.pop() {
        job.spend(NODE_WORK)?;
        let Object::Reference(id) = next else {
            continue;
        };
        if names.is_empty() {
            break;
        }
        if id == page || !seen.insert((id.number, id.generation)) {
            continue;
        }
        let node = stage.get(job, id)?;
        if let Object::Array(kids) = overlay.entry(objects, &node, b"Kids")? {
            pending.extend(kids);
            continue;
        }
        if holder(run, stage, (id, node.clone()), key)? != Some(held) {
            continue;
        }
        let resources = display::inherited(objects, overlay, &node, b"Resources")?;
        let streams = display::streams(objects, overlay, node.get(b"Contents"))?;
        let listing = data::made(job, Object::Array(streams), None)?;
        run.scope = Rc::new(Scope::new(data::made(job, resources, None)?));
        let joined = Box::new(Joined::new(job, listing));
        let result = mentioned(run, stage, (held, key), joined, (names, 0));
        run.scope = Rc::clone(&scope);
        match result {
            // Content that cannot be read may use any of them.
            Err(error) if error.code() == ErrorCode::Malformed => names.clear(),
            other => other?,
        }
    }
    job.damaged.set(damaged);
    Ok(())
}
