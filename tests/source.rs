use bagkit::source::{self, Document};
use std::io::{Cursor, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

fn deflated() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

/// A ZIP written as a stream, so every entry's sizes follow its data in a descriptor, as
/// in the national extract's nested archives.
fn streamed_zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::new_stream(Vec::new());
    for (name, contents) in entries {
        writer.start_file(*name, deflated()).unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// A ZIP written with seeking, so sizes are in the local headers.
fn seekable_zip(entries: &[(&str, Vec<u8>)], options: SimpleFileOptions) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn xml(i: usize) -> Vec<u8> {
    format!("<doc>{}</doc>", "x".repeat(i * 7919 % 50_000)).into_bytes()
}

/// Every document in a ZIP file, with its contents.
fn read_all(outer: &[u8]) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extract.zip");
    std::fs::write(&path, outer).unwrap();
    let mut documents = Vec::new();
    source::documents(&path, &|_| true, &mut |document: Document| {
        let name = document.name();
        let mut contents = Vec::new();
        document.read(|input| Ok(input.read_to_end(&mut contents)?))?;
        documents.push((name.rsplit(" > ").next().unwrap().to_owned(), contents));
        Ok(())
    })?;
    Ok(documents)
}

/// Removes the optional signature from every data descriptor in a streamed ZIP.
fn without_descriptor_signatures(zip: &[u8], descriptors: usize) -> Vec<u8> {
    let mut out = zip.to_vec();
    for _ in 0..descriptors {
        let at = memchr::memmem::rfind(&out, b"PK\x07\x08").unwrap();
        out.drain(at..at + 4);
    }
    out
}

#[test]
fn streamed_nested_archives_yield_every_document_intact() {
    let documents: Vec<(String, Vec<u8>)> = (0..40)
        .map(|i| (format!("part-{i:03}.xml"), xml(i)))
        .collect();
    let entries: Vec<(&str, Vec<u8>)> = documents
        .iter()
        .map(|(n, c)| (n.as_str(), c.clone()))
        .collect();
    let signed = streamed_zip(&entries);
    let unsigned = without_descriptor_signatures(&signed, entries.len());
    assert!(unsigned.len() < signed.len());
    for inner in [signed, unsigned, seekable_zip(&entries, deflated())] {
        let outer = seekable_zip(&[("9999NUM01012026.zip", inner)], deflated());
        assert_eq!(read_all(&outer).unwrap(), documents);
    }
}

#[test]
fn archives_nested_inside_streamed_archives_are_read() {
    let deepest = seekable_zip(&[("inactive.xml", xml(3))], deflated());
    let stored = seekable_zip(
        &[("stored.xml", xml(4))],
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
    );
    let inner = streamed_zip(&[
        ("9999IANUM01012026.zip", deepest),
        ("9999IAVBO01012026.zip", stored),
        ("readme.txt", b"skipped".to_vec()),
    ]);
    let outer = seekable_zip(&[("9999Inactief01012026.zip", inner)], deflated());
    assert_eq!(
        read_all(&outer).unwrap(),
        [
            ("inactive.xml".to_owned(), xml(3)),
            ("stored.xml".to_owned(), xml(4))
        ]
    );
}

#[test]
fn corrupt_streamed_entries_fail_their_checksum() {
    let inner = streamed_zip(&[("a.xml", xml(1)), ("b.xml", xml(2))]);
    // Flip the last byte of a.xml's compressed data, just before its data descriptor.
    let data_start = 30
        + usize::from(u16::from_le_bytes([inner[26], inner[27]]))
        + usize::from(u16::from_le_bytes([inner[28], inner[29]]));
    let descriptor = memchr::memmem::find(&inner[data_start..], b"PK\x07\x08").unwrap();
    let mut corrupt = inner.clone();
    corrupt[data_start + descriptor - 1] ^= 0x01;
    let outer = seekable_zip(&[("9999NUM01012026.zip", corrupt)], deflated());
    let error = read_all(&outer).unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("CRC-32") || message.contains("corrupt deflate stream"),
        "{message}"
    );
}
