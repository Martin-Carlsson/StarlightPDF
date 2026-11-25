//! Redacting pages: their content, annotations and thumbnails, then the marks' fill, all staged
//! and set at once (ISO 32000-2:2020, 12.3.4, 12.5.6.23).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};

use crate::PdfError;
use crate::objects::ObjectId;
use crate::overlay::{Data, Object, Overlay, Reading};
use crate::page;
use crate::page::display;
use crate::part::Leaves;
use crate::progress::Progress;
use crate::render::data::{self, Scope};
use crate::render::fixed::Fixed;
use crate::render::geometry::{Area, Matrix};
use crate::render::interpret::content::Joined;
use crate::render::interpret::state::State;
use crate::render::interpret::{Caches, Run};
use crate::render::job::{Job, Pace, WORK_STEP, work_limit};
use crate::render::plane::Planes;
use crate::render::redact::marks::{self, Mark, Marks};
use crate::render::redact::shared::Released;
use crate::render::redact::stage::Stage;
use crate::render::redact::structure;
use crate::render::redact::walk::{self, Context, Found, Mode};
use crate::render::surface::Surface;
use crate::rendering::RenderUsage;
use crate::store::{Bounds, Objects, Store};

/// The operation errors name.
const OPERATION: &str = "Document::apply_redactions";

/// What walking a page's content found, the content rewritten when changed, and its streams.
type Walked = (Found, Option<Vec<u8>>, Vec<Object>);

/// What the annotations of a page become.
struct Kept {
    /// Those left.
    left: Vec<Object>,
    /// The `StructParent` keys of those removed, and them; then of forms a mark touched.
    parents: Vec<(i64, Option<ObjectId>)>,
    /// The widgets removed, and their dictionaries.
    widgets: Vec<(ObjectId, Object)>,
}

/// Redacts the pages of `pages`, of `count`, none being all, and removes their marks; all or
/// nothing.
///
/// # Errors
///
/// As [`crate::document::Document::apply_redactions`].
pub(crate) fn apply(
    store: &Arc<Store>,
    overlay: &Arc<Overlay>,
    progress: Option<&dyn Progress>,
    (pages, count): (Option<&[u32]>, u32),
) -> Result<(), PdfError> {
    let unbounded = store.objects(OPERATION);
    let bounds = Bounds::default();
    // A page's content may be read twice: walked, then copied.
    let decoded = unbounded.decoded_limit();
    let work = work_limit(decoded, 0).saturating_add(decoded.saturating_mul(2));
    bounds.work.set(work);
    let pace = Pace {
        checkpoint: Cell::new(work.saturating_sub(WORK_STEP)),
        done: Cell::new(0),
        height: Cell::new(pages.map_or_else(
            || u64::from(count),
            |listed| u64::try_from(listed.len()).unwrap_or(u64::MAX),
        )),
        overlay: Reading::new(overlay),
        progress,
        work: &bounds.work,
    };
    let objects = unbounded.within(&bounds).paced(&pace);
    pace.report()?;
    let (mut stage, mut released) = (Stage::default(), Released::default());
    let mut listed = pages.map(<[u32]>::iter);
    let mut all = if listed.is_none() {
        Some(Leaves::new(objects, overlay)?)
    } else {
        None
    };
    let mut changed = false;
    loop {
        bounds.work.set(work);
        pace.checkpoint.set(work.saturating_sub(WORK_STEP));
        let next = match (listed.as_mut(), all.as_mut()) {
            (Some(indices), _) => indices
                .next()
                .map(|&index| page::find(store, overlay, index).map(|found| found.object()))
                .transpose()?,
            (None, Some(leaves)) => leaves.next(objects, overlay)?,
            (None, None) => None,
        };
        let Some(id) = next else {
            break;
        };
        let job = job(objects, (overlay, &pace));
        changed |= redact(&job, (&mut stage, &mut released), id)?;
        pace.done.set(pace.done.get().saturating_add(1));
        pace.report()?;
    }
    if !changed {
        return Ok(());
    }
    bounds.work.set(work);
    pace.checkpoint.set(work.saturating_sub(WORK_STEP));
    let job = job(objects, (overlay, &pace));
    let mut caches = Caches::default();
    let mut surface = Surface::new(&job, Area::default(), None)?;
    let mut run = Run {
        active: Vec::new(),
        caches: &mut caches,
        depth: 0,
        hidden: 0,
        job: &job,
        locked_color: false,
        scope: Rc::new(Scope::new(data::made(&job, Object::Null, None)?)),
        state: State::new(Matrix::identity(), Area::default()),
        surface: &mut surface,
    };
    released.release(&mut run, &mut stage)?;
    stage.commit(&pace.overlay, OPERATION)
}

/// A job of `objects` and `overlay` that paints nothing, paced by `pace`.
fn job<'job>(
    objects: Objects<'job>,
    (overlay, pace): (&Arc<Overlay>, &'job Pace<'job>),
) -> Job<'job> {
    Job {
        area: Area::default(),
        damaged: Cell::new(false),
        device: Matrix::identity(),
        known: RefCell::default(),
        lent: RefCell::default(),
        objects,
        overlay: Reading::new(overlay),
        pace,
        page: Area::default(),
        planes: Planes::new(true),
        refused: Cell::new(false),
        usage: RenderUsage::Display,
        work: pace.work,
    }
}

/// The annotations `listed` of a page less the marks, those whose `Rect` meets one and their
/// pop-ups, and those left without their `IRT` and `Popup` where these are gone (12.5.2,
/// 12.5.6.14).
fn annotations(
    (job, stage): (&Job<'_>, &mut Stage),
    marks: &Marks,
    listed: Vec<Object>,
) -> Result<Kept, PdfError> {
    let (objects, overlay) = (job.objects, &*job.overlay);
    let mut read = Vec::new();
    let mut gone = Vec::new();
    let mut parents = Vec::new();
    for item in listed {
        let (value, id) = overlay.ended(objects, item.clone())?;
        let kind = overlay.entry(objects, &value, b"Subtype")?;
        let named = |name: &[u8]| matches!(kind, Object::Name(ref found) if found == name);
        let rect = rectangle(objects, overlay, &value)?;
        let meets = named(b"Redact")
            || rect
                .map(|found| marks.meets(job, &marks::rectangle(found)))
                .transpose()?
                .unwrap_or(false);
        if meets {
            if let Object::Integer(key) = overlay.entry(objects, &value, b"StructParent")? {
                parents.push((key, id));
            }
            gone.extend(id);
            if let Some(Object::Reference(popup)) = value.get(b"Popup") {
                gone.push(popup);
            }
        }
        read.push((item, value, id, meets, meets && named(b"Widget")));
    }
    let mut kept = Kept {
        left: Vec::new(),
        parents,
        widgets: Vec::new(),
    };
    for (item, value, id, meets, widget) in read {
        let parent = match value.get(b"Parent") {
            Some(Object::Reference(found)) => Some(found),
            Some(_) | None => None,
        };
        let removed = id.map_or(meets, |found| gone.contains(&found))
            || parent.is_some_and(|found| gone.contains(&found));
        if !removed {
            kept.left
                .push(unlinked((job, &mut *stage), (item, value, id), &gone)?);
            continue;
        }
        if widget && let Some(found) = id {
            kept.widgets.push((found, value));
        }
    }
    Ok(kept)
}

/// The annotation `item`, `value` read from object `id` where one, without its `IRT` and `Popup`
/// where they refer to one of `gone`; staged where it is its own object (12.5.2, 12.5.6.14).
fn unlinked(
    (job, stage): (&Job<'_>, &mut Stage),
    (item, mut value, id): (Object, Object, Option<ObjectId>),
    gone: &[ObjectId],
) -> Result<Object, PdfError> {
    let mut changed = false;
    for key in [&b"IRT"[..], b"Popup"] {
        if let Some(Object::Reference(found)) = value.get(key)
            && gone.contains(&found)
        {
            value.set(key, None);
            changed = true;
        }
    }
    match id {
        Some(found) if changed => {
            stage.set(job, found, value)?;
            Ok(item)
        }
        Some(_) | None => Ok(if changed { value } else { item }),
    }
}

/// Removes `kid` from the array `key` of `owner`, staging the array where it is its own object;
/// whether it is empty after.
fn detached(
    job: &Job<'_>,
    stage: &mut Stage,
    owner: &mut Object,
    (key, kid): (&[u8], ObjectId),
) -> Result<bool, PdfError> {
    let keep = |items: &mut Vec<Object>| {
        items.retain(|item| !matches!(*item, Object::Reference(found) if found == kid));
        items.is_empty()
    };
    match owner.get(key) {
        Some(Object::Reference(array)) => {
            let Object::Array(mut items) = stage.get(job, array)? else {
                return Ok(false);
            };
            let empty = keep(&mut items);
            stage.set(job, array, Object::Array(items))?;
            Ok(empty)
        }
        Some(Object::Array(mut items)) => {
            let empty = keep(&mut items);
            owner.set(key, Some(Object::Array(items)));
            Ok(empty)
        }
        Some(_) | None => Ok(false),
    }
}

/// Removes the widget `widget` from its field, and a field left without widgets from its parent
/// or the form's `Fields`; what is removed leaves the form's `CO` (12.7.3.3, 12.7.4).
fn detach(job: &Job<'_>, stage: &mut Stage, widget: (ObjectId, Object)) -> Result<(), PdfError> {
    let (mut kid, mut value) = widget;
    let mut gone = vec![kid];
    let mut top = None;
    let mut ended = false;
    for _ in 0..=job.objects.depth_limit() {
        let Some(Object::Reference(parent)) = value.get(b"Parent") else {
            (top, ended) = (Some(kid), true);
            break;
        };
        let mut node = stage.get(job, parent)?;
        let empty = detached(job, stage, &mut node, (b"Kids", kid))?;
        stage.set(job, parent, node.clone())?;
        if !empty {
            ended = true;
            break;
        }
        (kid, value) = (parent, node);
        gone.push(kid);
    }
    if !ended {
        return Err(job.limit());
    }
    let trailer = Object::Dictionary(job.overlay.trailer(job.objects)?);
    let Some(Object::Reference(root)) = trailer.get(b"Root") else {
        return Ok(());
    };
    let mut catalog = stage.get(job, root)?;
    let (mut form, place) = match catalog.get(b"AcroForm") {
        Some(Object::Reference(found)) => (stage.get(job, found)?, Some(found)),
        Some(found @ Object::Dictionary(_)) => (found, None),
        Some(_) | None => return Ok(()),
    };
    for (key, removed) in top
        .map(|field| (&b"Fields"[..], field))
        .into_iter()
        .chain(gone.into_iter().map(|field| (&b"CO"[..], field)))
    {
        let _empty: bool = detached(job, stage, &mut form, (key, removed))?;
    }
    if let Some(found) = place {
        stage.set(job, found, form)
    } else {
        catalog.set(b"AcroForm", Some(form));
        stage.set(job, root, catalog)
    }
}

/// The normalized `Rect` of `annotation`; none when it has none (7.9.5).
fn rectangle(
    objects: Objects<'_>,
    overlay: &Overlay,
    annotation: &Object,
) -> Result<Option<[Fixed; 4]>, PdfError> {
    let Object::Array(items) = overlay.entry(objects, annotation, b"Rect")? else {
        return Ok(None);
    };
    let mut found = Vec::new();
    for item in items {
        let Some(number) = Fixed::from_object(&overlay.direct(objects, item)?) else {
            return Ok(None);
        };
        found.push(number);
    }
    Ok(match *found.as_slice() {
        [left, bottom, right, top] => Some([
            left.min(right),
            bottom.min(top),
            left.max(right),
            bottom.max(top),
        ]),
        _ => None,
    })
}

/// Walks the content of the page `id`, `dictionary`, for what `marks` meet: what it found, the
/// content rewritten when changed, and its streams; sets the resources a copy adds, and adds
/// those it no longer uses to `released`.
fn content(
    job: &Job<'_>,
    (marks, stage, released): (&Marks, &mut Stage, &mut Released),
    (id, dictionary): (ObjectId, &mut Object),
) -> Result<Walked, PdfError> {
    let (objects, reading) = (job.objects, &*job.overlay);
    let inherited = job.tolerate(
        display::inherited(objects, reading, dictionary, b"Resources"),
        "7.7.3.4",
        "the page's Resources are damaged",
    )?;
    let resources = data::made(job, inherited.unwrap_or(Object::Null), None)?;
    let contents = job.tolerate(
        display::streams(objects, reading, dictionary.get(b"Contents")),
        "7.7.3.3",
        "Contents is neither a stream nor an array",
    )?;
    let items = contents.unwrap_or_default();
    let listing = data::made(job, Object::Array(items.clone()), None)?;
    let mut caches = Caches::default();
    let mut surface = Surface::new(job, Area::default(), None)?;
    let mut run = Run {
        active: Vec::new(),
        caches: &mut caches,
        depth: 0,
        hidden: 0,
        job,
        locked_color: false,
        scope: Rc::new(Scope::new(resources)),
        state: State::new(Matrix::identity(), Area::default()),
        surface: &mut surface,
    };
    let mut cx = Context {
        marks,
        measured: BTreeSet::new(),
        parents: structure::key(job, dictionary, b"StructParents")?,
        procedures: BTreeMap::new(),
        stage,
    };
    let joined = Box::new(Joined::new(job, listing.clone()));
    let found = walk::stream(&mut run, &mut cx, joined, Mode::Rewrite, false)?;
    let rewritten = if found.changed() {
        let again = Box::new(Joined::new(job, listing));
        Some(walk::emit(job, again, &found, cx.stage.room(job))?)
    } else {
        None
    };
    if !found.added.is_empty() || !found.removed.is_empty() {
        let made = walk::resources_with(job, run.scope.resources.value(), &found, &cx.measured)?;
        released.add(job, cx.stage, (id, dictionary), &found.removed)?;
        dictionary.set(b"Resources", Some(made));
    }
    Ok((found, rewritten, items))
}

/// Sets the `Contents` of the page `dictionary`: `rewritten` where changed, else its `items`,
/// then the fill of `marks` in default user space where one has a colour (12.5.6.23).
fn filled(
    (job, stage): (&Job<'_>, &mut Stage),
    dictionary: &mut Object,
    (found, rewritten, items): (&Found, Option<Vec<u8>>, Vec<Object>),
    marks: &[Mark],
) -> Result<(), PdfError> {
    let fill = marks::fill(marks);
    let stream = |bytes: Vec<u8>| Object::Stream(Vec::new(), Data::Bytes(Arc::from(bytes)));
    let mut middle = match rewritten {
        Some(bytes) => {
            let mut coded = stream(bytes);
            coded.set(b"Filter", Some(Object::Name(b"FlateDecode".to_vec())));
            vec![Object::Reference(stage.add(job, coded)?)]
        }
        None if fill.is_empty() => return Ok(()),
        None => items,
    };
    if fill.is_empty() {
        dictionary.set(b"Contents", middle.pop());
        return Ok(());
    }
    // What the content leaves open is closed first.
    let before = "q\n".repeat(found.stray.saturating_add(1));
    let mut after = String::from(if found.texting { "\nET" } else { "" });
    after.push_str(&"\nEMC".repeat(found.marked));
    after.push_str(&"\nQ".repeat(found.open.saturating_add(1)));
    after.push('\n');
    after.push_str(&fill);
    let mut all = vec![Object::Reference(
        stage.add(job, stream(before.into_bytes()))?,
    )];
    all.append(&mut middle);
    all.push(Object::Reference(
        stage.add(job, stream(after.into_bytes()))?,
    ));
    dictionary.set(b"Contents", Some(Object::Array(all)));
    Ok(())
}

/// Redacts the page `id` where it has a mark, staging what changes; whether it had one.
fn redact(
    job: &Job<'_>,
    (stage, released): (&mut Stage, &mut Released),
    id: ObjectId,
) -> Result<bool, PdfError> {
    let (objects, reading) = (job.objects, &*job.overlay);
    let mut dictionary = stage.get(job, id)?;
    if !matches!(dictionary, Object::Dictionary(_)) {
        return Err(objects.malformed());
    }
    let listed = match reading.entry(objects, &dictionary, b"Annots")? {
        Object::Array(items) => items,
        Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => Vec::new(),
    };
    let mut read: Vec<Mark> = Vec::new();
    for item in &listed {
        let value = reading.direct(objects, item.clone())?;
        read.extend(marks::read(objects, reading, &value)?);
    }
    if read.is_empty() {
        return Ok(false);
    }
    let marks = Marks::new(read);
    let walked = content(job, (&marks, stage, released), (id, &mut dictionary))?;
    let mut kept = annotations((job, &mut *stage), &marks, listed)?;
    // A form a mark touched is one content item, as an annotation is (14.7.5.3).
    kept.parents
        .extend(walked.0.elements.iter().map(|&key| (key, None)));
    structure::clear(job, stage, &walked.0.struck, &kept.parents)?;
    structure::moved(job, stage, &walked.0.moved)?;
    for widget in kept.widgets {
        detach(job, stage, widget)?;
    }
    dictionary.set(
        b"Annots",
        (!kept.left.is_empty()).then_some(Object::Array(kept.left)),
    );
    dictionary.set(b"Thumb", None);
    let (found, rewritten, items) = walked;
    filled(
        (job, stage),
        &mut dictionary,
        (&found, rewritten, items),
        marks.list(),
    )?;
    stage.set(job, id, dictionary)?;
    Ok(true)
}
