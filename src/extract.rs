//! Reads the current voorkomens of a BAG extract in parallel.
//!
//! [`Extract`] streams the voorkomens of selected [object types](ObjectType) that are
//! current on a peildatum. One thread lists documents, worker threads inflate, parse
//! and convert them with a [`Converter`], and the calling thread receives the converted
//! items in document order. Bounded channels, and a limit on how far workers may run ahead
//! of the caller, keep memory flat.
//!
//! Reading happens in two [parts](Part): first everything nummeraanduidingen depend on,
//! then nummeraanduidingen and panden. Callers can so hold what nummeraanduidingen refer to
//! in memory before the nummeraanduidingen arrive, in any input order. Documents are
//! assigned to parts by name, as the extract names them (`WPL`, `GEM-WPL-RELATIE`, `OPR`,
//! `VBO`, `LIG`, `STA`, `NUM`, `PND`); documents of object types nobody selected are
//! skipped unread. Documents with other names are read in the first part, and may hold
//! voorkomens of any type.
//!
//! ```no_run
//! use bagkit::extract::{Extract, ObjectType, Selection};
//!
//! let extract = Extract::new(vec!["lvbag-extract-nl.zip".into()], "2026-10-08")?;
//! let mut localities = 0;
//! extract.for_each(Selection::only(&[ObjectType::Locality]), |_object| {
//!     localities += 1;
//!     Ok(())
//! })?;
//! # anyhow::Ok(())
//! ```
use crate::{
    source::{self, Document},
    xml::{self, BagObject, Kind},
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    thread,
};

/// The kinds of voorkomen an extract holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectType {
    /// Woonplaats.
    Locality,
    /// GEM-WPL-RELATIE, the gemeente of each woonplaats, which Kadaster supplies alongside
    /// the BAG.
    MunicipalityRelation,
    /// Openbare ruimte.
    PublicSpace,
    /// Nummeraanduiding.
    Address,
    /// Adresseerbaar object: verblijfsobject, ligplaats or standplaats.
    AddressableObject,
    /// Pand.
    Building,
}

impl ObjectType {
    pub const ALL: [ObjectType; 6] = [
        Self::Locality,
        Self::MunicipalityRelation,
        Self::PublicSpace,
        Self::Address,
        Self::AddressableObject,
        Self::Building,
    ];

    fn of(kind: Kind) -> Self {
        match kind {
            Kind::Woonplaats => Self::Locality,
            Kind::GemeenteWoonplaatsRelatie => Self::MunicipalityRelation,
            Kind::OpenbareRuimte => Self::PublicSpace,
            Kind::Nummeraanduiding => Self::Address,
            Kind::Verblijfsobject | Kind::Ligplaats | Kind::Standplaats => Self::AddressableObject,
            Kind::Pand => Self::Building,
        }
    }

    /// The object type a document or archive name says it holds.
    pub fn of_document(name: &str) -> Option<Self> {
        let name = name.to_ascii_uppercase();
        let has = |code: &str| name.contains(code);
        Some(if has("GEM-WPL") {
            Self::MunicipalityRelation
        } else if has("WPL") {
            Self::Locality
        } else if has("OPR") {
            Self::PublicSpace
        } else if has("NUM") {
            Self::Address
        } else if has("VBO") || has("LIG") || has("STA") {
            Self::AddressableObject
        } else if has("PND") {
            Self::Building
        } else {
            return None;
        })
    }

    /// The name used on the command line and in metadata: the table it fills, in kebab
    /// case.
    pub fn name(self) -> &'static str {
        match self {
            Self::Locality => "woonplaatsen",
            Self::MunicipalityRelation => "gemeenten",
            Self::PublicSpace => "openbare-ruimten",
            Self::Address => "nummeraanduidingen",
            Self::AddressableObject => "adresseerbare-objecten",
            Self::Building => "panden",
        }
    }

    /// The type a [name](Self::name) stands for.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The part of the read this type belongs to.
    pub fn part(self) -> Part {
        match self {
            Self::Address | Self::Building => Part::Second,
            _ => Part::First,
        }
    }
}

/// A set of object types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection(u8);

impl Selection {
    pub fn all() -> Self {
        Self::only(&ObjectType::ALL)
    }

    pub fn only(types: &[ObjectType]) -> Self {
        let mut selection = Self::default();
        for &t in types {
            selection.insert(t);
        }
        selection
    }

    pub fn insert(&mut self, object_type: ObjectType) {
        self.0 |= 1 << object_type as u8;
    }

    pub fn remove(&mut self, object_type: ObjectType) {
        self.0 &= !(1 << object_type as u8);
    }

    pub fn contains(self, object_type: ObjectType) -> bool {
        self.0 & (1 << object_type as u8) != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The selected types in `part`.
    pub fn in_part(self, part: Part) -> Self {
        Self::only(
            &ObjectType::ALL
                .into_iter()
                .filter(|&t| self.contains(t) && t.part() == part)
                .collect::<Vec<_>>(),
        )
    }

    pub fn types(self) -> impl Iterator<Item = ObjectType> {
        ObjectType::ALL
            .into_iter()
            .filter(move |&t| self.contains(t))
    }
}

/// The two halves of a read; see the [module documentation](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Woonplaatsen, their gemeenten, openbare ruimten and adresseerbare objecten, plus
    /// every document whose name names no object type.
    First,
    /// Nummeraanduidingen and panden.
    Second,
}

/// Turns current voorkomens into the items a caller writes, on worker threads. Each
/// document gets its own state, so converters can batch what they emit.
pub trait Converter: Sync {
    type State;
    type Item: Send;

    fn begin(&self) -> Self::State;
    fn convert(
        &self,
        state: &mut Self::State,
        object: BagObject,
        emit: &mut dyn FnMut(Self::Item) -> Result<()>,
    ) -> Result<()>;
    /// Called after a document's last voorkomen.
    fn end(
        &self,
        _state: Self::State,
        _emit: &mut dyn FnMut(Self::Item) -> Result<()>,
    ) -> Result<()> {
        Ok(())
    }
}

/// What a read went through.
#[derive(Debug, Default, Clone)]
pub struct ReadStats {
    /// Voorkomens parsed or skipped, current or not.
    pub records: u64,
    /// Current voorkomens of selected types.
    pub current: u64,
    pub documents: u64,
    /// The `StandTechnischeDatum` each document declares.
    pub extract_dates: BTreeSet<String>,
}

impl ReadStats {
    pub fn add(&mut self, other: ReadStats) {
        self.records += other.records;
        self.current += other.current;
        self.documents += other.documents;
        self.extract_dates.extend(other.extract_dates);
    }
}

/// The inputs of one extract and how to read them.
#[derive(Debug, Clone)]
pub struct Extract {
    pub paths: Vec<PathBuf>,
    /// The peildatum, YYYY-MM-DD.
    pub date: String,
    /// Worker threads; 0 uses every available core.
    pub threads: usize,
    /// Parse the geometrie of panden and the vlakken of adresseerbare objecten. Without
    /// them, adresseerbare objecten with a vlak still get its centroid as their punt.
    pub footprints: bool,
}

impl Extract {
    pub fn new(paths: Vec<PathBuf>, date: &str) -> Result<Self> {
        xml::validate_date(date)?;
        ensure!(
            !paths.is_empty(),
            "provide XML files, directories, ZIPs or gzip files"
        );
        Ok(Self {
            paths,
            date: date.to_owned(),
            threads: 0,
            footprints: true,
        })
    }

    /// Calls `visit` with every current voorkomen of the selected types, in document order:
    /// the first part, then the second.
    pub fn for_each(
        &self,
        selection: Selection,
        mut visit: impl FnMut(BagObject) -> Result<()>,
    ) -> Result<ReadStats> {
        struct Identity;
        impl Converter for Identity {
            type State = ();
            type Item = BagObject;
            fn begin(&self) {}
            fn convert(
                &self,
                _: &mut (),
                object: BagObject,
                emit: &mut dyn FnMut(BagObject) -> Result<()>,
            ) -> Result<()> {
                emit(object)
            }
        }
        let mut stats = self.read(Part::First, selection, &Identity, &mut visit)?;
        stats.add(self.read(Part::Second, selection, &Identity, &mut visit)?);
        Ok(stats)
    }

    /// Reads one part: converts the current voorkomens of the selected types in it on worker
    /// threads, and passes the items to `write` in document order. In the second part, a
    /// current voorkomen of a first-part type fails the read, since it arrives too late.
    pub fn read<C: Converter>(
        &self,
        part: Part,
        selection: Selection,
        converter: &C,
        write: &mut dyn FnMut(C::Item) -> Result<()>,
    ) -> Result<ReadStats> {
        let wanted = selection.in_part(part);
        // Archives are skipped by name only when they name a type nobody wants here.
        let select = |name: &str| match ObjectType::of_document(name) {
            Some(t) => wanted.contains(t),
            None => part == Part::First,
        };
        // Documents of another part's types are read in that part.
        let work = |document: &Document, emit: &mut dyn FnMut(C::Item) -> Result<()>| {
            let mut state = converter.begin();
            let mut current = 0;
            let summary = document.parse(Some(&self.date), |record| {
                let object_type = ObjectType::of(record.kind);
                if !selection.contains(object_type) {
                    return Ok(());
                }
                if !record
                    .validity
                    .is_current(&self.date)
                    .with_context(|| format!("{} {}", record.kind, record.id))?
                {
                    return Ok(());
                }
                ensure!(
                    part == Part::First || object_type.part() == Part::Second,
                    "{} records must be read before nummeraanduidingen and panden, so files \
                     named NUM or PND may hold only nummeraanduidingen and panden",
                    record.kind
                );
                current += 1;
                converter.convert(&mut state, record.into_object(self.footprints)?, emit)
            })?;
            converter.end(state, emit)?;
            Ok(ReadStats {
                records: summary.records,
                current,
                documents: 0,
                extract_dates: summary.extract_date.into_iter().collect(),
            })
        };
        if wanted.is_empty() && part == Part::Second {
            return Ok(ReadStats::default());
        }
        pipeline(&self.paths, self.threads, &select, &work, write)
    }
}

/// Raised in threads that stop because another thread failed.
#[derive(Debug)]
struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("import stopped")
    }
}

impl std::error::Error for Stopped {}

/// Items one worker sends at a time.
const BATCH: usize = 1024;

/// Converts one document, emitting items for the writer and counting what it read.
type Work<'a, T> =
    dyn Fn(&Document, &mut dyn FnMut(T) -> Result<()>) -> Result<ReadStats> + Sync + 'a;

/// Runs `work` on every document [`source::documents`] finds whose name `select` accepts,
/// and passes the items it emits to `write` in document order.
fn pipeline<T: Send>(
    paths: &[PathBuf],
    threads: usize,
    select: &(dyn Fn(&str) -> bool + Sync),
    work: &Work<'_, T>,
    write: &mut dyn FnMut(T) -> Result<()>,
) -> Result<ReadStats> {
    let threads = match threads {
        0 => thread::available_parallelism().map_or(4, |n| n.get()),
        n => n,
    };
    let (documents_tx, documents_rx) = sync_channel::<(u64, Document)>(threads * 2);
    let documents_rx = Mutex::new(documents_rx);
    let (items_tx, items_rx) = sync_channel::<Batch<T>>(threads * 4);
    let stop = AtomicBool::new(false);
    // The next document the writer needs; workers wait rather than run too far ahead of it.
    let progress = Progress::default();
    let ahead = threads as u64 * 2;
    let (documents_rx, stop, progress) = (&documents_rx, &stop, &progress);
    let halt = || {
        stop.store(true, Ordering::Relaxed);
        progress.advanced.notify_all();
    };
    thread::scope(|scope| {
        let reader = scope.spawn(move || -> Result<()> {
            let mut number = 0;
            for path in paths {
                source::documents(path, select, &mut |document| {
                    if stop.load(Ordering::Relaxed) {
                        bail!(Stopped);
                    }
                    if !select(document.file_name()) {
                        return Ok(());
                    }
                    documents_tx.send((number, document)).map_err(|_| Stopped)?;
                    number += 1;
                    Ok(())
                })
                .with_context(|| format!("importing {}", path.display()))?;
            }
            Ok(())
        });
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let items_tx = items_tx.clone();
                scope.spawn(move || -> Result<ReadStats> {
                    let mut stats = ReadStats::default();
                    loop {
                        let next = documents_rx.lock().unwrap().recv();
                        let Ok((number, document)) = next else { break };
                        progress.wait_until_within(number, ahead, stop);
                        if stop.load(Ordering::Relaxed) {
                            bail!(Stopped);
                        }
                        if let Err(error) = run(number, &document, work, &items_tx)
                            .map(|part| stats.add(part))
                            .with_context(|| format!("importing {}", document.name()))
                        {
                            halt();
                            return Err(error);
                        }
                    }
                    Ok(stats)
                })
            })
            .collect();
        drop(items_tx);
        let mut write_error = None;
        let mut in_order = InOrder {
            write,
            next: 0,
            waiting: BTreeMap::new(),
            progress,
        };
        for batch in items_rx.iter() {
            if let Err(error) = in_order.accept(batch) {
                halt();
                write_error = Some(error);
                break;
            }
        }
        let waiting = in_order.waiting.len();
        // Dropping the receiver unblocks workers waiting to send after a write error.
        drop(items_rx);
        let mut stats = ReadStats::default();
        let mut first_error = write_error;
        for result in workers
            .into_iter()
            .map(|worker| worker.join().expect("import worker panicked"))
            .chain([reader
                .join()
                .expect("import reader panicked")
                .map(|()| ReadStats::default())])
        {
            match result {
                Ok(part) => stats.add(part),
                Err(error) if error.is::<Stopped>() => (),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => {
                ensure!(waiting == 0, "{waiting} documents were never written");
                Ok(stats)
            }
        }
    })
}

/// Converts one document, sending its items to the writer [`BATCH`] at a time.
fn run<T>(
    number: u64,
    document: &Document,
    work: &Work<'_, T>,
    items: &SyncSender<Batch<T>>,
) -> Result<ReadStats> {
    let mut pending = Vec::with_capacity(BATCH);
    let mut stats = work(document, &mut |item| {
        pending.push(item);
        if pending.len() == BATCH {
            let full = std::mem::replace(&mut pending, Vec::with_capacity(BATCH));
            items
                .send(Batch {
                    document: number,
                    items: full,
                    last: false,
                })
                .map_err(|_| Stopped)?;
        }
        Ok(())
    })?;
    items
        .send(Batch {
            document: number,
            items: pending,
            last: true,
        })
        .map_err(|_| Stopped)?;
    stats.documents += 1;
    Ok(stats)
}

/// Items from one document; `last` marks the document's final batch.
struct Batch<T> {
    document: u64,
    items: Vec<T>,
    last: bool,
}

/// The next document the writer needs, shared with workers.
#[derive(Default)]
struct Progress {
    next: Mutex<u64>,
    advanced: Condvar,
}

impl Progress {
    /// Blocks until document `number` is fewer than `ahead` documents past the writer, or the
    /// import stops. The document the writer needs never waits, so this cannot deadlock.
    fn wait_until_within(&self, number: u64, ahead: u64, stop: &AtomicBool) {
        let mut next = self.next.lock().unwrap();
        while number >= *next + ahead && !stop.load(Ordering::Relaxed) {
            next = self.advanced.wait(next).unwrap();
        }
    }
}

/// Writes batches in document order. Documents are numbered in the order they are read,
/// and a BAG extract's documents are sorted by identificatie, so each table receives
/// ascending keys and SQLite appends instead of searching and shifting. Batches of
/// documents that finish early wait here; workers stay within a fixed number of documents
/// of the writer (see [`Progress`]), which bounds the memory this takes.
struct InOrder<'w, 'p, T> {
    write: &'w mut dyn FnMut(T) -> Result<()>,
    next: u64,
    waiting: BTreeMap<u64, (Vec<Vec<T>>, bool)>,
    progress: &'p Progress,
}

impl<T> InOrder<'_, '_, T> {
    fn accept(&mut self, batch: Batch<T>) -> Result<()> {
        if batch.document != self.next {
            let entry = self.waiting.entry(batch.document).or_default();
            entry.0.push(batch.items);
            entry.1 |= batch.last;
            return Ok(());
        }
        self.write(batch.items)?;
        if batch.last {
            self.next += 1;
            while let Some((batches, last)) = self.waiting.remove(&self.next) {
                for items in batches {
                    self.write(items)?;
                }
                if !last {
                    break;
                }
                self.next += 1;
            }
            *self.progress.next.lock().unwrap() = self.next;
            self.progress.advanced.notify_all();
        }
        Ok(())
    }

    fn write(&mut self, items: Vec<T>) -> Result<()> {
        items.into_iter().try_for_each(|item| (self.write)(item))
    }
}
