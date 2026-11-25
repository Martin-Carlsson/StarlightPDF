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
use crate::{ErrorCode, PdfError};

/// Work of a page or of content it runs.
const NODE_WORK: u64 = 64;

/// Names.
type Names = BTreeSet<Vec<u8>>;

/// Objects by number and generation.
pub(super) type Ids = BTreeSet<(u32, u16)>;

/// Names of resources and the objects of those it draws.
type Naming = BTreeMap<Vec<u8>, Ids>;

/// A resource's category and name.
type Named = (&'static [u8], Vec<u8>);

/// Content opened, or why it could not be.
type Opened<'data> = Result<Box<dyn Pull + 'data>, PdfError>;

/// An appearance stream and its own resources, where it has them.
type Shown = (Resolved, Option<Resolved>);

/// The forms and images a mark touched, and what still draws them.
#[derive(Default)]
pub(super) struct Released {
    /// Of them, those drawn.
    drawn: Ids,
    /// What each of them with resources of its own draws of them.
    edges: BTreeMap<(u32, u16), Ids>,
    /// They, soft mask groups included.
    objects: Ids,
    /// The redacted pages.
    pages: Ids,
    /// The content with resources of its own already read.
    seen: Ids,
}

impl Released {
    /// Adds the redacted page `page`, the originals of `moved`, which a mark touched there, and
    /// `drawn`, what it draws as it is.
    pub(super) fn add(
        &mut self,
        page: ObjectId,
        moved: &[(ObjectId, Option<ObjectId>, Vec<i64>)],
        drawn: &Ids,
    ) {
        let _new: bool = self.pages.insert(held(page));
        self.objects.extend(moved.iter().map(|entry| held(entry.0)));
        self.drawn.extend(drawn.iter().copied());
    }

    /// Of the objects, the names `resources` gives those, or what uses them as it is drawn.
    fn naming(&self, job: &Job<'_>, resources: &Resolved) -> Result<Naming, PdfError> {
        let mut named = Naming::new();
        for category in [&b"XObject"[..], b"ExtGState", b"Font", b"Pattern"] {
            let listed = match data::entry(job, resources.value(), category) {
                Err(error) if error.code() == ErrorCode::Malformed => continue,
                other => other?,
            };
            for pair in data::pairs(listed.value()).map_or(&[][..], Vec::as_slice) {
                job.spend(1)?;
                let mut found =
                    match data::resolve(job, &pair.1).and_then(|given| referenced(job, &given)) {
                        Err(error) if error.code() == ErrorCode::Malformed => continue,
                        other => other?,
                    };
                found.retain(|id| self.objects.contains(id));
                if !found.is_empty() {
                    named.entry(pair.0.clone()).or_default().extend(found);
                }
            }
        }
        Ok(named)
    }

    /// Reads what `resources` names that runs content: where it has resources of its own, or
    /// where `all`, through `resources`, each once, what it draws of the objects is drawn, or
    /// drawn where it is drawn when one of them; and so on within them, `depth` deep; past the
    /// deepest, all are drawn (7.8.3, 8.10.1).
    fn owners(
        &mut self,
        run: &mut Run<'_, '_>,
        resources: &Resolved,
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
                    let given = if inherits { resources } else { &own };
                    let named = self.naming(job, given)?;
                    let mut found = referenced(job, &owner)?;
                    found.retain(|id| self.objects.contains(id));
                    for stream in streams.iter().filter(|_| !named.is_empty()) {
                        let opened = data::open(job, &mut run.caches.streams, stream);
                        let scope = Rc::new(Scope::new(given.clone()));
                        found.extend(scanned(run, (opened, scope, 1), &named)?);
                    }
                    match owner.id().map(held).filter(|id| self.objects.contains(id)) {
                        Some(id) => self.edges.entry(id).or_default().extend(found),
                        None => self.drawn.extend(found),
                    }
                    if !inherits {
                        self.owners(run, &own, (all, depth.saturating_add(1)))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Reads the page `node`, as staged, for what it, its annotations' appearances and content
    /// with resources of its own draw; a page that cannot be drawn draws nothing (7.7.3.3,
    /// 7.7.3.4, 12.5.5).
    fn page(
        &mut self,
        run: &mut Run<'_, '_>,
        node: &Object,
        redacted: bool,
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
        let named = self.naming(job, &within.resources)?;
        // A redacted page's content is known: what it draws as it is was added.
        if !named.is_empty() && !redacted {
            let streams = job.tolerate(
                display::streams(objects, overlay, node.get(b"Contents")),
                "7.7.3.3",
                "Contents is neither a stream nor an array",
            )?;
            let listing = data::made(job, Object::Array(streams.unwrap_or_default()), None)?;
            let joined: Box<dyn Pull + '_> = Box::new(Joined::new(job, listing));
            let found = scanned(run, (Ok(joined), Rc::clone(&within), 0), &named)?;
            self.drawn.extend(found);
        }
        for entry in &shown {
            let (stream, own) = (&entry.0, &entry.1);
            let target = stream
                .id()
                .map(held)
                .filter(|found| self.objects.contains(found));
            self.drawn.extend(target);
            let (scope, given) = match *own {
                Some(ref resources) => (
                    Rc::new(Scope::new(resources.clone())),
                    self.naming(job, resources)?,
                ),
                None => (Rc::clone(&within), named.clone()),
            };
            if !given.is_empty() {
                job.spend(NODE_WORK)?;
                let opened = data::open(job, &mut run.caches.streams, stream);
                self.drawn.extend(scanned(run, (opened, scope, 1), &given)?);
            }
        }
        self.owners(run, &within.resources, (redacted, 0))?;
        for resources in shown.iter().filter_map(|entry| entry.1.as_ref()) {
            self.owners(run, resources, (redacted, 0))?;
        }
        Ok(())
    }

    /// Stages as null the objects that no page, template page, appearance or icon of an
    /// annotation kept or content with resources of its own draws, reading each page once; all
    /// are drawn where the templates cannot be read (7.3.10, 7.7.4, 12.5.5).
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
            self.page(run, &node, self.pages.contains(&held(id)))?;
        }
        match templates(job) {
            Ok(found) => {
                for item in found {
                    if self.drawn.len() >= self.objects.len() {
                        break;
                    }
                    let (node, redacted) = if let Object::Reference(id) = item {
                        (stage.get(job, id)?, self.pages.contains(&held(id)))
                    } else {
                        (item, false)
                    };
                    self.page(run, &node, redacted)?;
                }
            }
            Err(error) if error.code() == ErrorCode::Malformed => {
                self.drawn.clone_from(&self.objects);
            }
            Err(error) => return Err(error),
        }
        job.damaged.set(damaged);
        let mut next: Vec<(u32, u16)> = self.drawn.iter().copied().collect();
        while let Some(found) = next.pop() {
            for reached in self.edges.remove(&found).unwrap_or_default() {
                job.spend(1)?;
                if self.drawn.insert(reached) {
                    next.push(reached);
                }
            }
        }
        for &(number, generation) in self.objects.difference(&self.drawn) {
            stage.set(job, ObjectId { generation, number }, Object::Null)?;
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

/// Of `removed`, by category and name, those that the appearance streams `shown` without
/// resources of their own name, run within the resources of their page; all where one cannot be
/// read (12.5.5).
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
pub(super) fn referenced(job: &Job<'_>, found: &Resolved) -> Result<Ids, PdfError> {
    let mut ids: Ids = found.id().map(held).into_iter().collect();
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
            Ok(entry) => ids.extend(entry.id().map(held)),
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
            let _new: bool = ids.insert(held(id));
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
) -> Result<Ids, PdfError> {
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
        .flat_map(|entry| entry.1.iter().copied())
        .collect())
}
