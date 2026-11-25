//! What structure says of content a mark touches: its words for it removed
//! (ISO 32000-2:2020, 14.6, 14.7, 14.9.3 to 14.9.5).

use alloc::collections::{BTreeMap, BTreeSet};

use crate::PdfError;
use crate::objects::ObjectId;
use crate::overlay::Object;
use crate::render::data::{self, Category, Resolved};
use crate::render::interpret::Run;
use crate::render::job::Job;
use crate::render::redact::stage::Stage;

/// Work of a node of the parent tree, or an element.
const NODE_WORK: u64 = 64;

/// The keys that say in words what content is: replacement text, alternate description and
/// expansion.
const WORDS: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];

/// A marked-content level open.
pub(super) struct Level {
    /// Its `MCID`.
    pub id: Option<i64>,
    /// Its property list's name in `Properties`; none where inline or absent.
    pub name: Option<Vec<u8>>,
    /// Its property list, where it says in words what the content is.
    pub properties: Option<Resolved>,
    /// Where its operator is.
    pub span: (u64, u64),
    /// Its tag.
    pub tag: Vec<u8>,
    /// Content a mark meets is in it.
    pub touched: bool,
}

impl Level {
    /// Whether structure reads it.
    pub(super) const fn kept(&self) -> bool {
        self.id.is_some() || self.properties.is_some()
    }
}

/// Removes the words of the structure elements `struck`, parent tree keys and `MCID`s, reaches,
/// and those of `annotations`, keys and the annotations removed, with each annotation from its
/// element's kids; and the words of their ancestors (14.7.4.4, 14.7.5).
///
/// # Errors
///
/// As reading and staging objects.
pub(super) fn clear(
    job: &Job<'_>,
    stage: &mut Stage,
    struck: &[(i64, i64)],
    annotations: &[(i64, Option<ObjectId>)],
) -> Result<(), PdfError> {
    if struck.is_empty() && annotations.is_empty() {
        return Ok(());
    }
    let keys = struck
        .iter()
        .map(|entry| entry.0)
        .chain(annotations.iter().map(|entry| entry.0));
    let found = numbers(job, &parent_tree(job)?, &keys.collect())?;
    let mut elements = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |element: ObjectId, annotation: Option<ObjectId>| {
        let key = (element.number, element.generation);
        if seen.insert((key, annotation.map(|id| (id.number, id.generation)))) {
            elements.push((element, annotation));
        }
    };
    for &(key, id) in struck {
        if let Some(&(_, Object::Array(ref items))) = found.get(&key)
            && let Some(&Object::Reference(element)) =
                usize::try_from(id).ok().and_then(|at| items.get(at))
        {
            add(element, None);
        }
    }
    for &(key, annotation) in annotations {
        if let Some(&(Object::Reference(element), _)) = found.get(&key) {
            add(element, annotation);
        }
    }
    for (element, annotation) in elements {
        cleared(job, stage, element, annotation)?;
    }
    Ok(())
}

/// Removes the words of `element` and its ancestors, and `annotation` from its kids.
fn cleared(
    job: &Job<'_>,
    stage: &mut Stage,
    element: ObjectId,
    annotation: Option<ObjectId>,
) -> Result<(), PdfError> {
    let mut at = element;
    let mut gone = annotation;
    for _ in 0..=job.objects.depth_limit() {
        job.spend(NODE_WORK)?;
        let mut value = stage.get(job, at)?;
        if !matches!(value, Object::Dictionary(_)) {
            return Ok(());
        }
        let mut changed = false;
        for key in WORDS {
            if value.get(key).is_some() {
                value.set(key, None);
                changed = true;
            }
        }
        if let Some(removed) = gone.take() {
            changed |= retargeted(job, stage, &mut value, (removed, None))?;
        }
        let parent = value.get(b"P");
        if changed {
            stage.set(job, at, value)?;
        }
        let Some(Object::Reference(next)) = parent else {
            return Ok(());
        };
        at = next;
    }
    Err(job.limit())
}

/// Points the structure elements of the parent tree keys of each of `moved`, an object, its copy
/// or none where it is left out, and the keys, at the copy, or no longer at it (14.7.4.3, 14.7.5).
///
/// # Errors
///
/// As reading and staging objects.
pub(super) fn moved(
    job: &Job<'_>,
    stage: &mut Stage,
    moved: &[(ObjectId, Option<ObjectId>, Vec<i64>)],
) -> Result<(), PdfError> {
    if moved.is_empty() {
        return Ok(());
    }
    let wanted = moved.iter().flat_map(|entry| entry.2.iter().copied());
    let found = numbers(job, &parent_tree(job)?, &wanted.collect())?;
    for &(from, to, ref keys) in moved {
        for key in keys {
            let elements = found.get(key).map_or_else(Vec::new, |entry| {
                if let Object::Array(ref items) = entry.1 {
                    items.clone()
                } else {
                    vec![entry.0.clone()]
                }
            });
            for item in elements {
                let Object::Reference(element) = item else {
                    continue;
                };
                job.spend(NODE_WORK)?;
                let mut value = stage.get(job, element)?;
                if retargeted(job, stage, &mut value, (from, to))? {
                    stage.set(job, element, value)?;
                }
            }
        }
    }
    Ok(())
}

/// The parent tree, as stored; null where none (14.7.5.4).
fn parent_tree(job: &Job<'_>) -> Result<Object, PdfError> {
    let (objects, overlay) = (job.objects, &*job.overlay);
    let trailer = Object::Dictionary(overlay.trailer(objects)?);
    let catalog = overlay.entry(objects, &trailer, b"Root")?;
    let root = overlay.entry(objects, &catalog, b"StructTreeRoot")?;
    overlay.entry(objects, &root, b"ParentTree")
}

/// Makes the kids of `element` that refer by `Obj` or `Stm` to `from` refer to `to`, staged where
/// their own objects, or leaves them out where none; whether any did (14.7.2).
fn retargeted(
    job: &Job<'_>,
    stage: &mut Stage,
    element: &mut Object,
    (from, to): (ObjectId, Option<ObjectId>),
) -> Result<bool, PdfError> {
    let (objects, overlay) = (job.objects, &*job.overlay);
    let found = element.get(b"K").unwrap_or(Object::Null);
    let (items, single) = match overlay.direct(objects, found.clone())? {
        Object::Array(items) => (items, false),
        Object::Dictionary(_) => (vec![found], true),
        Object::Boolean(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => return Ok(false),
    };
    let mut changed = false;
    let mut kept = Vec::new();
    for item in items {
        job.spend(NODE_WORK)?;
        let own = match item {
            Object::Reference(id) => Some(id),
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Stream(..)
            | Object::String(_) => None,
        };
        let mut kid = match own {
            Some(id) => stage.get(job, id)?,
            None => item.clone(),
        };
        let keys = [&b"Obj"[..], b"Stm"];
        let Some(key) = keys
            .into_iter()
            .find(|key| matches!(kid.get(key), Some(Object::Reference(id)) if id == from))
        else {
            kept.push(item);
            continue;
        };
        changed = true;
        let Some(copy) = to else {
            continue;
        };
        kid.set(key, Some(Object::Reference(copy)));
        match own {
            Some(id) => {
                stage.set(job, id, kid)?;
                kept.push(item);
            }
            None => kept.push(kid),
        }
    }
    if changed {
        element.set(
            b"K",
            if single {
                kept.pop()
            } else {
                Some(Object::Array(kept))
            },
        );
    }
    Ok(changed)
}

/// The level `BDC` or `BMC`, `name`, with `operands` opens, its operator at `span`.
///
/// # Errors
///
/// As reading its property list.
pub(super) fn level(
    run: &Run<'_, '_>,
    name: &[u8],
    operands: &[Object],
    span: (u64, u64),
) -> Result<Level, PdfError> {
    let job = run.job;
    let tag = operands.first().and_then(data::name).unwrap_or_default();
    let mut level = Level {
        id: None,
        name: None,
        properties: None,
        span,
        tag: tag.to_vec(),
        touched: false,
    };
    if name != b"BDC" {
        return Ok(level);
    }
    let given = operands.get(1).unwrap_or(&Object::Null);
    let found = if let Some(key) = data::name(given) {
        level.name = Some(key.to_vec());
        named(run, key)?
    } else if matches!(*given, Object::Dictionary(_)) {
        Some(data::resolve(job, given)?)
    } else {
        None
    };
    let Some(list) = found else {
        return Ok(level);
    };
    if let Object::Integer(id) = *data::entry(job, list.value(), b"MCID")?.value() {
        level.id = Some(id);
    }
    if worded(list.value()) {
        level.properties = Some(list);
    }
    Ok(level)
}

/// The property list `name` of `run`'s resources; none where it is not a dictionary (14.6.2).
///
/// # Errors
///
/// As reading it.
pub(super) fn named(run: &Run<'_, '_>, name: &[u8]) -> Result<Option<Resolved>, PdfError> {
    let found = data::named(run.job, &run.scope, Category::Properties, name)?;
    Ok(found.filter(|list| matches!(*list.value(), Object::Dictionary(_))))
}

/// Whether `list` says in words what content is.
pub(super) fn worded(list: &Object) -> bool {
    WORDS
        .iter()
        .any(|key| !matches!(*data::stored(list, key), Object::Null))
}

/// The values of `keys` in the number tree `tree`, as stored and followed, the first where one is
/// held twice: each node read once, and none whose limits hold none of them (7.9.7).
fn numbers(
    job: &Job<'_>,
    tree: &Object,
    keys: &BTreeSet<i64>,
) -> Result<BTreeMap<i64, (Object, Object)>, PdfError> {
    let (objects, overlay) = (job.objects, &*job.overlay);
    let mut found = BTreeMap::new();
    if keys.is_empty() {
        return Ok(found);
    }
    let mut pending = vec![(tree.clone(), 0_u32)];
    while let Some((node, depth)) = pending.pop() {
        job.spend(NODE_WORK)?;
        if depth > objects.depth_limit() {
            return Err(job.limit());
        }
        if let Object::Array(pairs) = overlay.entry(objects, &node, b"Nums")? {
            let mut items = pairs.into_iter();
            while let (Some(key), Some(value)) = (items.next(), items.next()) {
                job.spend(1)?;
                if let Object::Integer(at) = overlay.direct(objects, key)?
                    && keys.contains(&at)
                    && !found.contains_key(&at)
                {
                    let followed = overlay.direct(objects, value.clone())?;
                    let _new: Option<(Object, Object)> = found.insert(at, (value, followed));
                }
            }
            continue;
        }
        let Object::Array(kids) = overlay.entry(objects, &node, b"Kids")? else {
            continue;
        };
        let mut taken = Vec::new();
        for kid in kids {
            job.spend(1)?;
            let child = overlay.direct(objects, kid)?;
            let mut bounds = Vec::new();
            if let Object::Array(limits) = overlay.entry(objects, &child, b"Limits")? {
                for limit in limits {
                    bounds.push(overlay.direct(objects, limit)?);
                }
            }
            let held = if let [Object::Integer(low), Object::Integer(high)] = *bounds.as_slice() {
                low <= high && keys.range(low..=high).next().is_some()
            } else {
                // Required, yet searched without (7.9.7).
                job.warn("7.9.7", "a number tree node lacks Limits")?;
                true
            };
            if held {
                taken.push((child, depth.saturating_add(1)));
            }
        }
        pending.extend(taken.into_iter().rev());
    }
    Ok(found)
}

/// The parent tree key `key` of `owner`, a page or a form: `StructParents` or `StructParent`
/// (14.7.4.4).
///
/// # Errors
///
/// As reading it.
pub(super) fn key(job: &Job<'_>, owner: &Object, key: &[u8]) -> Result<Option<i64>, PdfError> {
    let found = data::entry(job, owner, key)?;
    Ok(if let Object::Integer(number) = *found.value() {
        Some(number)
    } else {
        None
    })
}

/// `list` without its words.
pub(super) fn stripped(list: &Object) -> Object {
    let mut made = list.clone();
    for key in WORDS {
        made.set(key, None);
    }
    made
}
