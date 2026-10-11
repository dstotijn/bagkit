//! Finds the XML documents in import inputs. ZIP entries are copied out still compressed,
//! so documents can be inflated and parsed in parallel while reading stays sequential.
use crate::xml::{self, ParseSummary};
use anyhow::{Context, Result, bail, ensure};
use flate2::{
    Crc, CrcReader, Decompress, FlushDecompress, Status, read::DeflateDecoder, read::MultiGzDecoder,
};
use std::cell::RefCell;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Cursor, Read, Seek},
    path::{Path, PathBuf},
};
use zip::{CompressionMethod, ZipArchive};

/// Largest ZIP entry inflated into memory for parsing; the national extract's are under
/// 20 MiB.
const IN_MEMORY_DOCUMENT: u64 = 256 << 20;

thread_local! {
    /// Inflated documents, reused across a thread's documents.
    static SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// One XML document, possibly compressed.
pub enum Document {
    /// A plain or gzip-compressed file on disk.
    File(PathBuf),
    /// A ZIP entry's raw deflate stream, with the size and CRC-32 of its contents.
    Deflated {
        name: String,
        data: Vec<u8>,
        size: u64,
        crc32: u32,
    },
    /// A ZIP entry's contents, already decompressed: XML, or gzip when `gzip` is set.
    Bytes {
        name: String,
        data: Vec<u8>,
        gzip: bool,
    },
}

impl Document {
    pub fn name(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Deflated { name, .. } | Self::Bytes { name, .. } => name.clone(),
        }
    }

    /// The document's own file name, without the archives that contain it.
    pub fn file_name(&self) -> &str {
        match self {
            Self::File(path) => path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
            Self::Deflated { name, .. } | Self::Bytes { name, .. } => {
                let name = name.rsplit(" > ").next().unwrap_or(name);
                name.rsplit('/').next().unwrap_or(name)
            }
        }
    }

    /// Parses the document's voorkomens. ZIP entries up to [`IN_MEMORY_DOCUMENT`] are inflated
    /// into a buffer each thread reuses and parsed without copying; others are streamed.
    /// Voorkomens that ended on or before `date` may be skipped unread (see [`xml::parse_bytes`]).
    pub fn parse(
        &self,
        date: Option<&str>,
        emit: impl FnMut(xml::Record) -> Result<()>,
    ) -> Result<ParseSummary> {
        match self {
            Self::Deflated {
                data, size, crc32, ..
            } if *size <= IN_MEMORY_DOCUMENT => SCRATCH.with_borrow_mut(|buffer| {
                // The buffer only grows, so it is zeroed once rather than per document. One
                // spare byte shows whether the entry is longer than it claims.
                let size = *size as usize;
                if buffer.len() < size + 1 {
                    buffer.resize(size + 1, 0);
                }
                let mut inflater = Decompress::new(false);
                let status =
                    inflater.decompress(data, &mut buffer[..size + 1], FlushDecompress::Finish)?;
                let contents = &buffer[..inflater.total_out() as usize];
                let mut crc = Crc::new();
                crc.update(contents);
                ensure!(
                    status == Status::StreamEnd && contents.len() == size && crc.sum() == *crc32,
                    "ZIP entry is corrupt: size or CRC-32 mismatch"
                );
                xml::parse_bytes(contents, date, emit)
            }),
            Self::Bytes {
                data, gzip: false, ..
            } => xml::parse_bytes(data, date, emit),
            _ => self.read(|input| xml::parse(input, emit)),
        }
    }

    /// Reads the document with `read`, checking ZIP entry sizes and checksums afterwards.
    pub fn read<T>(&self, read: impl FnOnce(&mut dyn BufRead) -> Result<T>) -> Result<T> {
        const BUFFER: usize = 256 * 1024;
        match self {
            Self::File(path) => {
                let file = File::open(path)?;
                if extension(path) == "gz" {
                    read(&mut BufReader::with_capacity(
                        BUFFER,
                        MultiGzDecoder::new(file),
                    ))
                } else {
                    read(&mut BufReader::with_capacity(BUFFER, file))
                }
            }
            Self::Deflated {
                data, size, crc32, ..
            } => {
                let mut reader = BufReader::with_capacity(
                    BUFFER,
                    CrcReader::new(DeflateDecoder::new(&data[..])),
                );
                let result = read(&mut reader)?;
                // Drain anything the parser left, so the checksum covers the whole entry.
                std::io::copy(&mut reader, &mut std::io::sink())?;
                let crc = reader.get_ref().crc();
                ensure!(
                    u64::from(crc.amount()) == size & 0xFFFF_FFFF && crc.sum() == *crc32,
                    "ZIP entry is corrupt: size or CRC-32 mismatch"
                );
                Ok(result)
            }
            Self::Bytes { data, gzip, .. } => {
                if *gzip {
                    read(&mut BufReader::with_capacity(
                        BUFFER,
                        MultiGzDecoder::new(&data[..]),
                    ))
                } else {
                    read(&mut &data[..])
                }
            }
        }
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}

fn supported(path: &Path) -> bool {
    matches!(extension(path).as_str(), "xml" | "gz" | "zip")
}

/// Calls `found` with every XML document in `path`, in a stable order: directory entries
/// sorted by name, ZIP entries in archive order. ZIPs nested in a top-level archive whose
/// names `select` rejects are skipped unread; callers still filter the documents themselves.
pub fn documents(
    path: &Path,
    select: &dyn Fn(&str) -> bool,
    found: &mut impl FnMut(Document) -> Result<()>,
) -> Result<()> {
    if path.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(path)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<_>>()?;
        paths.sort_unstable();
        for child in paths {
            if child.is_dir() || supported(&child) {
                documents(&child, select, found)?;
            }
        }
        return Ok(());
    }
    match extension(path).as_str() {
        "xml" | "gz" => found(Document::File(path.to_owned())),
        "zip" => top_level_archive(path, select, found),
        _ => bail!("unsupported input {}", path.display()),
    }
}

/// Nested ZIPs found inside a streamed archive are read in memory up to this size; larger
/// ones are spooled to disk.
const IN_MEMORY_ZIP: u64 = 64 << 20;

/// The order to read a top-level archive's nested ZIPs in. Small archives come first.
/// Ligplaatsen and standplaatsen share a table with verblijfsobjecten and their
/// identificaties interleave, so they follow verblijfsobjecten to keep those inserts
/// appending. Nummeraanduidingen follow all adresseerbare objecten, so the writer knows
/// each nummeraanduiding's object when it writes it. Panden come last.
fn rank(name: &str) -> u8 {
    let name = name.to_ascii_uppercase();
    if name.contains("VBO") {
        1
    } else if name.contains("LIG") || name.contains("STA") {
        2
    } else if name.contains("NUM") {
        3
    } else if name.contains("PND") {
        4
    } else {
        0
    }
}

/// Reads a ZIP from disk. Its nested ZIPs are streamed: their entries are cut out of the
/// inflating stream by their local headers, so documents flow as soon as each one arrives
/// and nothing is spooled to disk.
fn top_level_archive(
    path: &Path,
    select: &dyn Fn(&str) -> bool,
    found: &mut impl FnMut(Document) -> Result<()>,
) -> Result<()> {
    let parent = path.display().to_string();
    let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
    let (mut entries, mut nested) = (Vec::new(), Vec::new());
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        if entry.is_dir() {
            continue;
        }
        if extension(Path::new(entry.name())) == "zip" {
            let name = entry.name().rsplit('/').next().unwrap_or("");
            // InOnderzoek holds only investigation markers (kenmerkInOnderzoek), never
            // BAG objects, so it is skipped unread.
            if select(name) && !name.to_ascii_uppercase().contains("INONDERZOEK") {
                nested.push((rank(entry.name()), entry.compressed_size(), i));
            }
        } else {
            entries.push(i);
        }
    }
    nested.sort_unstable();
    entries.extend(nested.into_iter().map(|(_, _, i)| i));
    for i in entries {
        let name = archive.by_index_raw(i)?.name().to_owned();
        let full_name = format!("{parent} > {name}");
        let result = if extension(Path::new(&name)) == "zip" {
            let input = BufReader::with_capacity(1 << 20, archive.by_index(i)?);
            stream_archive(input, &full_name, found, 1)
        } else {
            entry(&mut archive, i, &full_name, found, 0)
        };
        result.with_context(|| format!("ZIP entry {name}"))?;
    }
    Ok(())
}

/// Reads a seekable ZIP.
fn archive<R: Read + Seek>(
    input: R,
    parent: &str,
    found: &mut impl FnMut(Document) -> Result<()>,
    depth: usize,
) -> Result<()> {
    ensure!(
        depth <= 2,
        "ZIP nesting exceeds the BAG distribution layout"
    );
    let mut archive = ZipArchive::new(input)?;
    for i in 0..archive.len() {
        let name = {
            let entry = archive.by_index_raw(i)?;
            if entry.is_dir() {
                continue;
            }
            entry.name().to_owned()
        };
        entry(&mut archive, i, &format!("{parent} > {name}"), found, depth)
            .with_context(|| format!("ZIP entry {name}"))?;
    }
    Ok(())
}

/// Reads entry `i` of a seekable ZIP at nesting `depth`.
fn entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    i: usize,
    name: &str,
    found: &mut impl FnMut(Document) -> Result<()>,
    depth: usize,
) -> Result<()> {
    let (kind, method, size) = {
        let entry = archive.by_index_raw(i)?;
        (
            extension(Path::new(entry.name())),
            entry.compression(),
            entry.size(),
        )
    };
    match kind.as_str() {
        "xml" if method == CompressionMethod::Deflated => {
            let mut entry = archive.by_index_raw(i)?;
            let (size, crc32) = (entry.size(), entry.crc32());
            let mut data = Vec::with_capacity(entry.compressed_size() as usize);
            entry.read_to_end(&mut data)?;
            found(Document::Deflated {
                name: name.to_owned(),
                data,
                size,
                crc32,
            })
        }
        "xml" | "gz" => {
            let mut data = Vec::new();
            archive.by_index(i)?.read_to_end(&mut data)?;
            found(Document::Bytes {
                name: name.to_owned(),
                data,
                gzip: kind == "gz",
            })
        }
        "zip" => nested_zip(archive.by_index(i)?, size, name, found, depth + 1),
        _ => Ok(()),
    }
}

/// Reads a nested ZIP from its decompressed contents: in memory when small, otherwise
/// spooled to disk, since reading a ZIP's directory needs seeking.
fn nested_zip(
    mut contents: impl Read,
    size: u64,
    name: &str,
    found: &mut impl FnMut(Document) -> Result<()>,
    depth: usize,
) -> Result<()> {
    if size <= IN_MEMORY_ZIP {
        let mut data = Vec::with_capacity(size as usize);
        contents.read_to_end(&mut data)?;
        archive(Cursor::new(data), name, found, depth)
    } else {
        let mut file = tempfile::tempfile()?;
        std::io::copy(&mut contents, &mut file)?;
        file.rewind()?;
        archive(file, name, found, depth)
    }
}

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_DIRECTORY: u32 = 0x0605_4b50;
const DATA_DESCRIPTOR: u32 = 0x0807_4b50;

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

/// Reads a ZIP from a stream that cannot seek, by its local file headers. The extract's
/// nested ZIPs store each entry's sizes in a data descriptor after the data, so an entry
/// ends where a descriptor recording exactly the bytes read so far is followed by the next
/// header. Workers check each entry's CRC-32 and size after inflating it.
fn stream_archive(
    input: impl Read,
    parent: &str,
    found: &mut impl FnMut(Document) -> Result<()>,
    depth: usize,
) -> Result<()> {
    ensure!(
        depth <= 2,
        "ZIP nesting exceeds the BAG distribution layout"
    );
    let mut stream = Stream::new(input);
    loop {
        ensure!(
            stream.fill(4)?,
            "ZIP stream ends without a central directory"
        );
        match stream.peek_u32() {
            LOCAL_HEADER => (),
            CENTRAL_HEADER | END_OF_DIRECTORY => return Ok(()),
            _ => bail!("unexpected data in ZIP stream"),
        }
        let header = stream.take(30)?;
        let (flags, method) = (u16_at(&header, 6), u16_at(&header, 8));
        let name_length = usize::from(u16_at(&header, 26));
        let extra_length = usize::from(u16_at(&header, 28));
        let name = String::from_utf8_lossy(&stream.take(name_length)?).into_owned();
        stream.take(extra_length)?;
        let (data, crc32, size) = if flags & 8 == 0 {
            let compressed = u32_at(&header, 18);
            ensure!(
                compressed != u32::MAX,
                "ZIP64 entries without data descriptors are not supported in streams"
            );
            let data = stream.take(compressed as usize)?;
            (data, u32_at(&header, 14), u64::from(u32_at(&header, 22)))
        } else {
            stream.take_until_descriptor()?
        };
        let full_name = format!("{parent} > {name}");
        let kind = extension(Path::new(&name));
        let result = match (kind.as_str(), method) {
            _ if name.ends_with('/') => Ok(()),
            ("xml", 8) => found(Document::Deflated {
                name: full_name.clone(),
                data,
                size,
                crc32,
            }),
            ("xml" | "gz", 0) => found(Document::Bytes {
                name: full_name.clone(),
                data,
                gzip: kind == "gz",
            }),
            ("gz", 8) => {
                let document = Document::Deflated {
                    name: full_name.clone(),
                    data,
                    size,
                    crc32,
                };
                let mut contents = Vec::with_capacity(size as usize);
                document.read(|input| Ok(input.read_to_end(&mut contents)?))?;
                found(Document::Bytes {
                    name: full_name.clone(),
                    data: contents,
                    gzip: true,
                })
            }
            ("zip", 0) => nested_zip(&data[..], size, &full_name, found, depth + 1),
            ("zip", 8) => {
                let document = Document::Deflated {
                    name: full_name.clone(),
                    data,
                    size,
                    crc32,
                };
                document.read(|input| nested_zip(input, size, &full_name, found, depth + 1))
            }
            ("xml" | "gz" | "zip", other) => bail!("unsupported ZIP compression method {other}"),
            _ => Ok(()),
        };
        result.with_context(|| format!("ZIP entry {name}"))?;
    }
}

/// A buffered, forward-only reader over a ZIP stream.
struct Stream<R> {
    input: R,
    /// Holds unread bytes in `start..end`; the rest is spare space, already initialized,
    /// so reads need not zero it again.
    buffer: Vec<u8>,
    start: usize,
    end: usize,
}

impl<R: Read> Stream<R> {
    const CHUNK: usize = 1 << 20;

    fn new(input: R) -> Self {
        Self {
            input,
            buffer: Vec::new(),
            start: 0,
            end: 0,
        }
    }

    fn available(&self) -> &[u8] {
        &self.buffer[self.start..self.end]
    }

    /// Buffers at least `n` unread bytes; false when the stream ends first.
    fn fill(&mut self, n: usize) -> Result<bool> {
        while self.available().len() < n {
            if self.start > 0 {
                self.buffer.copy_within(self.start..self.end, 0);
                self.end -= self.start;
                self.start = 0;
            }
            let wanted = self.end + Self::CHUNK.max(n - self.end);
            if self.buffer.len() < wanted {
                self.buffer.resize(wanted, 0);
            }
            let read = self.input.read(&mut self.buffer[self.end..])?;
            if read == 0 {
                return Ok(false);
            }
            self.end += read;
        }
        Ok(true)
    }

    fn peek_u32(&self) -> u32 {
        u32_at(self.available(), 0)
    }

    fn take(&mut self, n: usize) -> Result<Vec<u8>> {
        ensure!(self.fill(n)?, "truncated ZIP stream");
        let taken = self.available()[..n].to_vec();
        self.start += n;
        Ok(taken)
    }

    /// Takes an entry's data and its data descriptor, returning the data, CRC-32 and
    /// uncompressed size. The descriptor's signature is optional, and its sizes take eight
    /// bytes in ZIP64 archives; each layout must record exactly the data's length and be
    /// followed by another header.
    fn take_until_descriptor(&mut self) -> Result<(Vec<u8>, u32, u64)> {
        let mut searched = 0;
        loop {
            let available = self.available();
            let mut need = None;
            for found in memchr::memmem::find_iter(&available[searched..], b"PK") {
                let at = searched + found;
                match descriptor(available, at) {
                    Candidate::Found(length, end, crc32, size) => {
                        let data = available[..length].to_vec();
                        self.start += end;
                        return Ok((data, crc32, size));
                    }
                    Candidate::NeedMore => {
                        need = Some(at);
                        break;
                    }
                    Candidate::No => (),
                }
            }
            // Resume at an undecided candidate, or just before the end, in case "PK" is split.
            searched = need.unwrap_or(available.len().saturating_sub(1));
            let wanted = self.available().len() + Self::CHUNK;
            ensure!(
                self.fill(wanted)? || self.available().len() > searched + 1,
                "ZIP stream ends inside an entry"
            );
            if need.is_some() && self.available().len() < wanted {
                // The stream has ended; decide the remaining candidates with what there is.
                let available = self.available();
                for found in memchr::memmem::find_iter(&available[searched..], b"PK") {
                    if let Candidate::Found(length, end, crc32, size) =
                        descriptor(available, searched + found)
                    {
                        let data = available[..length].to_vec();
                        self.start += end;
                        return Ok((data, crc32, size));
                    }
                }
                bail!("ZIP stream ends inside an entry");
            }
        }
    }
}

enum Candidate {
    /// Data length, bytes up to the next header, CRC-32 and uncompressed size.
    Found(usize, usize, u32, u64),
    NeedMore,
    No,
}

/// Whether "PK" at `at` starts a data descriptor (`PK\x07\x08`) or the header after an
/// unsigned one, for an entry whose data starts at offset 0 of `bytes`.
fn descriptor(bytes: &[u8], at: usize) -> Candidate {
    let header_at = |offset: usize| {
        bytes.len() >= offset + 4 && matches!(u32_at(bytes, offset), LOCAL_HEADER | CENTRAL_HEADER)
    };
    if bytes.len() < at + 4 {
        return Candidate::NeedMore;
    }
    match u32_at(bytes, at) {
        DATA_DESCRIPTOR => {
            if bytes.len() < at + 28 {
                return Candidate::NeedMore;
            }
            if u64::from(u32_at(bytes, at + 8)) == at as u64 && header_at(at + 16) {
                let size = u64::from(u32_at(bytes, at + 12));
                return Candidate::Found(at, at + 16, u32_at(bytes, at + 4), size);
            }
            if u64_at(bytes, at + 8) == at as u64 && header_at(at + 24) {
                return Candidate::Found(
                    at,
                    at + 24,
                    u32_at(bytes, at + 4),
                    u64_at(bytes, at + 16),
                );
            }
            Candidate::No
        }
        LOCAL_HEADER | CENTRAL_HEADER => {
            if at >= 12 && u64::from(u32_at(bytes, at - 8)) == (at - 12) as u64 {
                let size = u64::from(u32_at(bytes, at - 4));
                return Candidate::Found(at - 12, at, u32_at(bytes, at - 12), size);
            }
            if at >= 20 && u64_at(bytes, at - 16) == (at - 20) as u64 {
                return Candidate::Found(
                    at - 20,
                    at,
                    u32_at(bytes, at - 20),
                    u64_at(bytes, at - 8),
                );
            }
            Candidate::No
        }
        _ => Candidate::No,
    }
}
