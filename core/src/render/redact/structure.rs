//! What structure says of content a mark touches: its words for it removed
//! (ISO 32000-2:2020, 14.6, 14.7, 14.9.3 to 14.9.5).

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
    let (objects, overlay) = (job.objects, &*job.overlay);
    let trailer = Object::Dictionary(overlay.trailer(objects)?);
    let catalog = overlay.entry(objects, &trailer, b"Root")?;
    let root = overlay.entry(objects, &catalog, b"StructTreeRoot")?;
    let tree = overlay.entry(objects, &root, b"ParentTree")?;
    let mut elements = Vec::new();
    for &(key, id) in struck {
        let Object::Array(items) = overlay.direct(objects, number(job, &tree, key)?)? else {
            continue;
        };
        if let Some(&Object::Reference(element)) =
            usize::try_from(id).ok().and_then(|at| items.get(at))
        {
            elements.push((element, None));
        }
    }
    for &(key, annotation) in annotations {
        if let Object::Reference(element) = number(job, &tree, key)? {
            elements.push((element, annotation));
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
    let (objects, overlay) = (job.objects, &*job.overlay);
    let mut at = element;
    let mut gone = annotation;
    for _ in 0..=objects.depth_limit() {
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
            let refers = |item: &Object| -> Result<bool, PdfError> {
                let found = overlay.direct(objects, item.clone())?;
                Ok(matches!(found.get(b"Obj"), Some(Object::Reference(id)) if id == removed))
            };
            match overlay.direct(objects, value.get(b"K").unwrap_or(Object::Null))? {
                Object::Array(items) => {
                    let count = items.len();
                    let mut kept = Vec::new();
                    for item in items {
                        if !refers(&item)? {
                            kept.push(item);
                        }
                    }
                    if kept.len() != count {
                        value.set(b"K", Some(Object::Array(kept)));
                        changed = true;
                    }
                }
                single @ Object::Dictionary(_) => {
                    if refers(&single)? {
                        value.set(b"K", None);
                        changed = true;
                    }
                }
                Object::Boolean(_)
                | Object::Integer(_)
                | Object::Name(_)
                | Object::Null
                | Object::Real(_)
                | Object::Reference(_)
                | Object::Stream(..)
                | Object::String(_) => {}
            }
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
        data::named(job, &run.scope, Category::Properties, key)?
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
    if WORDS
        .iter()
        .any(|key| !matches!(*data::stored(list.value(), key), Object::Null))
    {
        level.properties = Some(list);
    }
    Ok(level)
}

/// The value of `key` in the number tree `tree`, as stored; null where absent (7.9.7).
fn number(job: &Job<'_>, tree: &Object, key: i64) -> Result<Object, PdfError> {
    let (objects, overlay) = (job.objects, &*job.overlay);
    let mut node = tree.clone();
    for _ in 0..=objects.depth_limit() {
        job.spend(NODE_WORK)?;
        if let Object::Array(pairs) = overlay.entry(objects, &node, b"Nums")? {
            let mut items = pairs.into_iter();
            while let (Some(found), Some(value)) = (items.next(), items.next()) {
                if matches!(overlay.direct(objects, found)?, Object::Integer(at) if at == key) {
                    return Ok(value);
                }
            }
            return Ok(Object::Null);
        }
        let Object::Array(kids) = overlay.entry(objects, &node, b"Kids")? else {
            return Ok(Object::Null);
        };
        let mut next = None;
        for kid in kids {
            let child = overlay.direct(objects, kid)?;
            let Object::Array(limits) = overlay.entry(objects, &child, b"Limits")? else {
                continue;
            };
            let mut bounds = Vec::new();
            for limit in limits {
                bounds.push(overlay.direct(objects, limit)?);
            }
            if let [Object::Integer(low), Object::Integer(high)] = *bounds.as_slice()
                && low <= key
                && key <= high
            {
                next = Some(child);
                break;
            }
        }
        let Some(child) = next else {
            return Ok(Object::Null);
        };
        node = child;
    }
    Err(job.limit())
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
pub(super) fn stripped(list: &Resolved) -> Object {
    let mut made = list.value().clone();
    for key in WORDS {
        made.set(key, None);
    }
    made
}
