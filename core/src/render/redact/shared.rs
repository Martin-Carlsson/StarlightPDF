//! Objects a redaction removed or replaced by copies: those nothing left draws become null, so
//! that what still names them finds nothing (ISO 32000-2:2020, 7.3.10, 7.8.3).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use core::mem;

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
use crate::render::redact::walk::Found;
use crate::{ErrorCode, PdfError};

/// The dictionaries between an object holding a property list directly and the list, by how
/// deep it holds it.
const INSIDE: [&[u8]; 2] = [b"Resources", b"Properties"];

/// Work of a page or of content it runs.
const NODE_WORK: u64 = 64;

/// Names.
type Names = BTreeSet<Vec<u8>>;

/// Objects by number and generation.
pub(super) type Ids = BTreeSet<(u32, u16)>;

/// An object, or a property list one holds directly: that one, how deep (0: it is the
/// `Properties`, 1: the `Resources`, 2: it holds the `Resources`) and the list's name.
pub(super) type Key = ((u32, u16), Option<(u8, Vec<u8>)>);

/// Objects and property lists.
pub(super) type Keys = BTreeSet<Key>;

/// Where resources are: the object they are, 1 deep, or that holds them directly, 2 deep.
pub(super) type Place = Option<(ObjectId, u8)>;

/// Names of resources and the objects of those it draws.
type Naming = BTreeMap<Vec<u8>, Keys>;

/// A resource's category and name.
type Named = (&'static [u8], Vec<u8>);

/// Content opened, or why it could not be.
type Opened<'data> = Result<Box<dyn Pull + 'data>, PdfError>;

/// An appearance stream and its own resources, where it has them.
pub(super) type Shown = (Resolved, Option<Resolved>);

/// The forms, images and property lists a mark touched and the content streams replaced, and
/// what still draws them.
#[derive(Default)]
pub(super) struct Released {
    /// Of them, those drawn.
    drawn: Keys,
    /// What each of them with resources of its own draws of them.
    edges: BTreeMap<Key, Keys>,
    /// They, soft mask groups included.
    objects: Keys,
    /// The redacted pages.
    pages: Ids,
    /// The content with resources of its own already read.
    seen: Ids,
}

impl Released {
    /// Adds the redacted page `page`, the originals of the forms and images and the property
    /// lists a mark touched there, as `found`, its content streams `replaced`, and `drawn`,
    /// what it draws as it is.
    pub(super) fn add(
        &mut self,
        page: ObjectId,
        (found, replaced): (&Found, &[Object]),
        drawn: &Keys,
    ) {
        let _new: bool = self.pages.insert(held(page));
        self.objects
            .extend(found.moved.iter().map(|entry| key(entry.0)));
        self.objects.extend(found.lists.iter().cloned());
        self.objects.extend(references(replaced));
        self.drawn.extend(drawn.iter().cloned());
    }

    /// Of the objects, the names `resources`, at `place`, gives those, or what uses them as it
    /// is drawn.
    fn naming(
        &self,
        job: &Job<'_>,
        (resources, place): (&Resolved, Place),
    ) -> Result<Naming, PdfError> {
        let mut named = Naming::new();
        for category in [
            &b"XObject"[..],
            b"ExtGState",
            b"Font",
            b"Pattern",
            b"Properties",
        ] {
            let listed = match data::entry(job, resources.value(), category) {
                Err(error) if error.code() == ErrorCode::Malformed => continue,
                other => other?,
            };
            for pair in data::pairs(listed.value()).map_or(&[][..], Vec::as_slice) {
                job.spend(1)?;
                let at = if let Object::Reference(id) = pair.1 {
                    Some(id)
                } else {
                    None
                };
                let mut found = if category == b"Properties" {
                    list(&listed, place, (&pair.0, at)).into_iter().collect()
                } else {
                    match data::resolve(job, &pair.1).and_then(|given| referenced(job, &given)) {
                        Err(error) if error.code() == ErrorCode::Malformed => continue,
                        other => other?,
                    }
                };
                found.retain(|id| self.objects.contains(id));
                if !found.is_empty() {
                    named.entry(pair.0.clone()).or_default().extend(found);
                }
            }
        }
        Ok(named)
    }

    /// Reads what `resources`, at `place`, names that runs content: where it has resources of its
    /// own, or where `all`, through `resources`, each once, what it draws of the objects is
    /// drawn, or drawn where it is drawn when one of them; and so on within them, `depth` deep;
    /// past the deepest, all are drawn (7.8.3, 8.10.1).
    fn owners(
        &mut self,
        run: &mut Run<'_, '_>,
        (resources, place): (&Resolved, Place),
        (all, depth): (bool, u8),
    ) -> Result<(), PdfError> {
        let job = run.job;
        if depth >= job::NESTING {
            self.drawn.clone_from(&self.objects);
            return Ok(());
        }
        for category in [&b"XObject"[..], b"Pattern", b"Font", b"ExtGState"] {
            let listed = match data::entry(job, resources.value(), category) {
                Err(error) if error.code() == ErrorCode::Malformed => continue,
                other => other?,
            };
            for pair in data::pairs(listed.value()).map_or(&[][..], Vec::as_slice) {
                job.spend(1)?;
                // What cannot be read draws nothing.
                let made = match data::resolve(job, &pair.1).and_then(|found| runs(job, &found)) {
                    Err(error) if error.code() == ErrorCode::Malformed => continue,
                    other => other?,
                };
                for (owner, streams) in made {
                    let own = match data::entry(job, owner.value(), b"Resources") {
                        Err(error) if error.code() == ErrorCode::Malformed => continue,
                        other => other?,
                    };
                    let inherits = !matches!(*own.value(), Object::Dictionary(_));
                    if (inherits && !all)
                        || owner.id().is_some_and(|id| !self.seen.insert(held(id)))
                    {
                        continue;
                    }
                    job.spend(NODE_WORK)?;
                    let at = own_place(&own, owner.id());
                    let given = if inherits {
                        (resources, place)
                    } else {
                        (&own, at)
                    };
                    let named = self.naming(job, given)?;
                    let mut found = referenced(job, &owner)?;
                    found.retain(|id| self.objects.contains(id));
                    for stream in streams.iter().filter(|_| !named.is_empty()) {
                        let opened = data::open(job, &mut run.caches.streams, stream);
                        let scope = Rc::new(Scope::new(given.0.clone()));
                        found.extend(scanned(run, (opened, scope, 1), &named)?);
                    }
                    match owner.id().map(key).filter(|id| self.objects.contains(id)) {
                        Some(id) => self.edges.entry(id).or_default().extend(found),
                        None => self.drawn.extend(found),
                    }
                    if !inherits {
                        self.owners(run, (&own, at), (all, depth.saturating_add(1)))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Reads the page `node`, as staged, object `id`, for what it, its annotations' appearances
    /// and content with resources of its own draw; a page that cannot be drawn draws nothing
    /// (7.7.3.3, 7.7.3.4, 12.5.5).
    fn page(
        &mut self,
        run: &mut Run<'_, '_>,
        node: &Object,
        (id, redacted): (Option<ObjectId>, bool),
    ) -> Result<(), PdfError> {
        let job = run.job;
        let (objects, overlay) = (job.objects, &*job.overlay);
        job.spend(NODE_WORK)?;
        let shown = match data::entry(job, node, b"Annots") {
            Err(error) if error.code() == ErrorCode::Malformed => Vec::new(),
            other => appearances(job, data::items(other?.value()).unwrap_or_default())?,
        };
        let inherited = job.tolerate(
            display::inherited(objects, overlay, node, b"Resources"),
            "7.7.3.4",
            "the page's Resources are damaged",
        )?;
        let made = data::made(job, inherited.unwrap_or(Object::Null), None)?;
        let within = Rc::new(Scope::new(made));
        let place = job
            .tolerate(
                located(job, node, id),
                "7.7.3.4",
                "the page's Resources are damaged",
            )?
            .flatten();
        let named = self.naming(job, (&within.resources, place))?;
        let listed = job.tolerate(
            display::streams(objects, overlay, node.get(b"Contents")),
            "7.7.3.3",
            "Contents is neither a stream nor an array",
        )?;
        let streams = listed.unwrap_or_default();
        for found in references(&streams) {
            if self.objects.contains(&found) {
                let _new: bool = self.drawn.insert(found);
            }
        }
        // A redacted page's content is known: what it draws as it is was added.
        if !named.is_empty() && !redacted {
            let listing = data::made(job, Object::Array(streams), None)?;
            let joined: Box<dyn Pull + '_> = Box::new(Joined::new(job, listing));
            let found = scanned(run, (Ok(joined), Rc::clone(&within), 0), &named)?;
            self.drawn.extend(found);
        }
        for entry in &shown {
            let (stream, own) = (&entry.0, &entry.1);
            let target = stream
                .id()
                .map(key)
                .filter(|found| self.objects.contains(found));
            self.drawn.extend(target);
            let (scope, given) = match *own {
                Some(ref resources) => (
                    Rc::new(Scope::new(resources.clone())),
                    self.naming(job, (resources, own_place(resources, stream.id())))?,
                ),
                None => (Rc::clone(&within), named.clone()),
            };
            if !given.is_empty() {
                job.spend(NODE_WORK)?;
                let opened = data::open(job, &mut run.caches.streams, stream);
                self.drawn.extend(scanned(run, (opened, scope, 1), &given)?);
            }
        }
        self.owners(run, (&within.resources, place), (redacted, 0))?;
        for entry in &shown {
            if let Some(ref resources) = entry.1 {
                let at = own_place(resources, entry.0.id());
                self.owners(run, (resources, at), (redacted, 0))?;
            }
        }
        Ok(())
    }

    /// Stages as null the objects that no page, template page, appearance or icon of an
    /// annotation kept or content with resources of its own draws, and removes such property
    /// lists from what holds them, reading each page once; all are drawn where the templates
    /// cannot be read (7.3.10, 7.7.4, 12.5.5).
    pub(super) fn release(
        mut self,
        run: &mut Run<'_, '_>,
        stage: &mut Stage,
    ) -> Result<(), PdfError> {
        let job = run.job;
        let (objects, overlay) = (job.objects, &*job.overlay);
        let damaged = job.damaged.get();
        let wanted = mem::take(&mut self.objects);
        self.drawn.retain(|id| wanted.contains(id));
        self.objects = wanted;
        let mut pages = Leaves::new(objects, overlay)?;
        while self.drawn.len() < self.objects.len()
            && let Some(id) = pages.next(objects, overlay)?
        {
            let node = stage.get(job, id)?;
            self.page(run, &node, (Some(id), self.pages.contains(&held(id))))?;
        }
        match templates(job) {
            Ok(found) => {
                for item in found {
                    if self.drawn.len() >= self.objects.len() {
                        break;
                    }
                    let (node, given) = if let Object::Reference(id) = item {
                        (
                            stage.get(job, id)?,
                            (Some(id), self.pages.contains(&held(id))),
                        )
                    } else {
                        (item, (None, false))
                    };
                    self.page(run, &node, given)?;
                }
            }
            Err(error) if error.code() == ErrorCode::Malformed => {
                self.drawn.clone_from(&self.objects);
            }
            Err(error) => return Err(error),
        }
        job.damaged.set(damaged);
        let mut next: Vec<Key> = self.drawn.iter().cloned().collect();
        while let Some(found) = next.pop() {
            for reached in self.edges.remove(&found).unwrap_or_default() {
                job.spend(1)?;
                if self.drawn.insert(reached.clone()) {
                    next.push(reached);
                }
            }
        }
        // An object comes before the lists it holds: one made null stays so.
        for &((number, generation), ref list) in self.objects.difference(&self.drawn) {
            let id = ObjectId { generation, number };
            let Some((depth, ref name)) = *list else {
                stage.set(job, id, Object::Null)?;
                continue;
            };
            job.spend(NODE_WORK)?;
            let mut value = stage.get(job, id)?;
            let inside = INSIDE.get(usize::from(2_u8.saturating_sub(depth))..);
            if without(&mut value, inside.unwrap_or_default(), name) {
                stage.set(job, id, value)?;
            }
        }
        Ok(())
    }
}

/// The appearance streams of the annotations `listed`, normal, rollover and down, of each state,
/// and the icons of widgets, with their own resources; those that cannot be read draw nothing
/// (12.5.3, 12.5.5, 12.5.6.19).
pub(super) fn appearances(job: &Job<'_>, listed: &[Object]) -> Result<Vec<Shown>, PdfError> {
    let mut streams = Vec::new();
    let read: [(&[u8], [&[u8]; 3]); 2] =
        [(b"AP", [b"N", b"R", b"D"]), (b"MK", [b"I", b"RI", b"IX"])];
    for (item, (owner, keys)) in listed.iter().flat_map(|item| read.map(|pair| (item, pair))) {
        job.spend(NODE_WORK)?;
        let given = data::resolve(job, item)
            .and_then(|annotation| data::entry(job, annotation.value(), owner));
        let appearances = match given {
            Err(error) if error.code() == ErrorCode::Malformed => continue,
            other => other?,
        };
        for key in keys {
            let found = match data::entry(job, appearances.value(), key) {
                Err(error) if error.code() == ErrorCode::Malformed => continue,
                other => other?,
            };
            if matches!(*found.value(), Object::Stream(..)) {
                streams.push(found);
                continue;
            }
            for pair in data::pairs(found.value()).map_or(&[][..], Vec::as_slice) {
                job.spend(1)?;
                match data::resolve(job, &pair.1) {
                    Ok(state) if matches!(*state.value(), Object::Stream(..)) => {
                        streams.push(state);
                    }
                    Err(error) if error.code() != ErrorCode::Malformed => return Err(error),
                    Ok(_) | Err(_) => {}
                }
            }
        }
    }
    let mut shown = Vec::new();
    for stream in streams {
        let resources = match data::entry(job, stream.value(), b"Resources") {
            Err(error) if error.code() == ErrorCode::Malformed => continue,
            other => other?,
        };
        let own = matches!(*resources.value(), Object::Dictionary(_)).then_some(resources);
        shown.push((stream, own));
    }
    Ok(shown)
}

/// Of `removed`, by category and name, those that the streams `shown` without resources of
/// their own name, run within the resources of their page; all where one cannot be read (12.5.5).
pub(super) fn drawn(
    run: &mut Run<'_, '_>,
    (shown, removed): (&[Shown], &[Named]),
) -> Result<Vec<Named>, PdfError> {
    let job = run.job;
    let mut names: Names = removed.iter().map(|entry| entry.1.clone()).collect();
    for entry in shown.iter().filter(|entry| entry.1.is_none()) {
        if names.is_empty() {
            break;
        }
        job.spend(NODE_WORK)?;
        let result = data::open(job, &mut run.caches.streams, &entry.0)
            .and_then(|opened| mentioned(run, opened, (&mut names, 1)));
        match result {
            Err(error) if error.code() == ErrorCode::Malformed => names.clear(),
            other => other?,
        }
    }
    Ok(removed
        .iter()
        .filter(|entry| !names.contains(&entry.1))
        .cloned()
        .collect())
}

/// The streams that the tiling patterns, Type 3 fonts and shading patterns' soft mask groups of
/// `resources` without resources of their own run within it; those that cannot be read draw
/// nothing (7.8.3, 8.7.3.1, 9.6.4, 11.6.5.2).
pub(super) fn inheriting(job: &Job<'_>, resources: &Object) -> Result<Vec<Shown>, PdfError> {
    let mut shown = Vec::new();
    for category in [&b"Pattern"[..], b"Font"] {
        let listed = match data::entry(job, resources, category) {
            Err(error) if error.code() == ErrorCode::Malformed => continue,
            other => other?,
        };
        for pair in data::pairs(listed.value()).map_or(&[][..], Vec::as_slice) {
            job.spend(1)?;
            let made = match data::resolve(job, &pair.1).and_then(|found| runs(job, &found)) {
                Err(error) if error.code() == ErrorCode::Malformed => continue,
                other => other?,
            };
            for (owner, streams) in made {
                let own = match data::entry(job, owner.value(), b"Resources") {
                    Err(error) if error.code() == ErrorCode::Malformed => continue,
                    other => other?,
                };
                if !matches!(*own.value(), Object::Dictionary(_)) {
                    shown.extend(streams.into_iter().map(|stream| (stream, None)));
                }
            }
        }
    }
    Ok(shown)
}

/// The template pages the catalog's `Templates` name tree holds, each node read once (7.7.4,
/// 7.9.6).
fn templates(job: &Job<'_>) -> Result<Vec<Object>, PdfError> {
    let (objects, overlay) = (job.objects, &*job.overlay);
    let trailer = Object::Dictionary(overlay.trailer(objects)?);
    let catalog = overlay.entry(objects, &trailer, b"Root")?;
    let names = overlay.entry(objects, &catalog, b"Names")?;
    let mut stack = vec![overlay.entry(objects, &names, b"Templates")?];
    let (mut seen, mut pages) = (Ids::new(), Vec::new());
    while let Some(node) = stack.pop() {
        job.spend(NODE_WORK)?;
        if let Object::Array(pairs) = overlay.entry(objects, &node, b"Names")? {
            job.spend(u64::try_from(pairs.len()).unwrap_or(u64::MAX))?;
            pages.extend(pairs.into_iter().skip(1).step_by(2));
        }
        if let Object::Array(kids) = overlay.entry(objects, &node, b"Kids")? {
            for kid in kids {
                job.spend(1)?;
                let (child, reached) = overlay.ended(objects, kid)?;
                if reached.is_none_or(|id| seen.insert(held(id))) {
                    stack.push(child);
                }
            }
        }
    }
    Ok(pages)
}

/// `id` as [`Ids`] hold it.
pub(super) const fn held(id: ObjectId) -> (u32, u16) {
    (id.number, id.generation)
}

/// `id` as [`Keys`] hold it.
pub(super) const fn key(id: ObjectId) -> Key {
    (held(id), None)
}

/// The property list `name` of `lists`, the `Properties` of resources at `place`: object `id`
/// where one, else as `lists` or `place` holds it.
pub(super) fn list(
    lists: &Resolved,
    place: Place,
    (name, id): (&[u8], Option<ObjectId>),
) -> Option<Key> {
    id.map(key).or_else(|| {
        let holder = lists.id().map(|found| (found, 0)).or(place);
        holder.map(|(found, depth)| (held(found), Some((depth, name.to_vec()))))
    })
}

/// Where the page `node`, object `id`, has its resources or inherits them from; none where not
/// known (7.7.3.4).
pub(super) fn located(
    job: &Job<'_>,
    node: &Object,
    id: Option<ObjectId>,
) -> Result<Place, PdfError> {
    let (objects, overlay) = (job.objects, &*job.overlay);
    let (mut current, mut at) = (node.clone(), id);
    for _ in 0..=objects.depth_limit() {
        job.spend(1)?;
        match current.get(b"Resources") {
            Some(Object::Reference(found)) => {
                if !matches!(overlay.resolve(objects, found)?, Object::Null) {
                    return Ok(Some((found, 1)));
                }
            }
            Some(Object::Null) | None => {}
            Some(_) => return Ok(at.map(|holder| (holder, 2))),
        }
        let Some(Object::Reference(parent)) = current.get(b"Parent") else {
            return Ok(None);
        };
        current = overlay.resolve(objects, parent)?;
        at = Some(parent);
    }
    Ok(None)
}

/// Where `resources`, those of the object `owner` where its own, are.
pub(super) fn own_place(resources: &Resolved, owner: Option<ObjectId>) -> Place {
    resources
        .id()
        .map(|found| (found, 1))
        .or_else(|| owner.map(|found| (found, 2)))
}

/// The objects `items` refer to.
fn references(items: &[Object]) -> impl Iterator<Item = Key> + '_ {
    items.iter().filter_map(|item| {
        if let Object::Reference(found) = *item {
            Some(key(found))
        } else {
            None
        }
    })
}

/// Removes the entry `name` from the dictionary `keys` lead to within `value`, each direct;
/// whether there was one.
fn without(value: &mut Object, keys: &[&[u8]], name: &[u8]) -> bool {
    let Some((first, rest)) = keys.split_first() else {
        let had = value.get(name).is_some();
        value.set(name, None);
        return had;
    };
    let Some(mut inner) = value.get(first) else {
        return false;
    };
    let had = without(&mut inner, rest, name);
    if had {
        value.set(first, Some(inner));
    }
    had
}

/// Removes from `names` those that `data` names, `depth` deep, or content it runs through the
/// same resources names: forms, Type 3 glyph procedures, soft mask groups and tiling pattern
/// cells that have none of their own; all past the deepest (7.8.2, 8.7.3.1, 8.10.1, 9.6.4,
/// 11.6.5.2).
fn mentioned(
    run: &mut Run<'_, '_>,
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
            if matches!(*own.value(), Object::Dictionary(_)) {
                continue;
            }
            for stream in streams {
                let opened = data::open(job, &mut run.caches.streams, &stream)?;
                mentioned(run, opened, (&mut *names, depth.saturating_add(1)))?;
            }
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
/// glyph procedures, a soft mask's group or font of a graphics state, a tiling pattern, a shading
/// pattern's soft mask group (8.4.5, 8.7.3.1, 8.7.4.2, 8.10.1, 9.6.4, 11.6.5.2).
pub(super) fn runs(
    job: &Job<'_>,
    found: &Resolved,
) -> Result<Vec<(Resolved, Vec<Resolved>)>, PdfError> {
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
    let pattern = data::entry(job, value, b"PatternType")?;
    let mut owners = vec![found.clone()];
    if matches!(*pattern.value(), Object::Integer(2)) {
        owners.push(data::entry(job, value, b"ExtGState")?);
    }
    for owner in owners {
        let mask = data::entry(job, owner.value(), b"SMask")?;
        let group = data::entry(job, mask.value(), b"G")?;
        if matches!(*group.value(), Object::Stream(..)) {
            made.push((group.clone(), vec![group]));
        }
    }
    let font = data::entry(job, value, b"Font")?;
    if let Some(first) = data::items(font.value()).and_then(<[Object]>::first) {
        made.extend(procedures(job, &data::resolve(job, first)?)?);
    }
    Ok(made)
}

/// `found`, a resource, and what it uses as it is drawn: an image's soft mask and mask, the soft
/// mask group of a graphics state or a shading pattern's, a Type 3 font's glyph procedures, which
/// may be forms too (8.7.4.2, 8.9.6, 9.6.4, 11.6.5.2).
pub(super) fn referenced(job: &Job<'_>, found: &Resolved) -> Result<Keys, PdfError> {
    let mut ids: Keys = found.id().map(key).into_iter().collect();
    let value = found.value();
    let group = |owner: &Object| -> Result<Resolved, PdfError> {
        let mask = data::entry(job, owner, b"SMask")?;
        data::entry(job, mask.value(), b"G")
    };
    let used = [
        data::entry(job, value, b"SMask"),
        data::entry(job, value, b"Mask"),
        group(value),
        data::entry(job, value, b"ExtGState").and_then(|parameters| group(parameters.value())),
    ];
    for given in used {
        match given {
            Ok(entry) => ids.extend(entry.id().map(key)),
            Err(error) if error.code() != ErrorCode::Malformed => return Err(error),
            Err(_) => {}
        }
    }
    let listed = match data::entry(job, value, b"CharProcs") {
        Err(error) if error.code() == ErrorCode::Malformed => return Ok(ids),
        other => other?,
    };
    for pair in data::pairs(listed.value()).map_or(&[][..], Vec::as_slice) {
        job.spend(1)?;
        if let Object::Reference(id) = pair.1 {
            let _new: bool = ids.insert(key(id));
        }
    }
    Ok(ids)
}

/// Of `named`, the objects of the names that `data`, run `depth` deep within `scope`, draws;
/// all where it cannot be read.
fn scanned(
    run: &mut Run<'_, '_>,
    (data, scope, depth): (Opened<'_>, Rc<Scope>, u8),
    named: &Naming,
) -> Result<Keys, PdfError> {
    let mut left: Names = named.keys().cloned().collect();
    let outer = mem::replace(&mut run.scope, scope);
    let result = data.and_then(|opened| mentioned(run, opened, (&mut left, depth)));
    run.scope = outer;
    match result {
        Err(error) if error.code() == ErrorCode::Malformed => left.clear(),
        other => other?,
    }
    Ok(named
        .iter()
        .filter(|entry| !left.contains(entry.0))
        .flat_map(|entry| entry.1.iter().cloned())
        .collect())
}
