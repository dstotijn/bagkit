//! Writers for each format, and building outputs beside their destination so a completed
//! output replaces the previous one in one step.
use crate::dataset::{Content, Encoded, Format, Table};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

/// Where an output goes: one file, or a directory with one file per table.
#[derive(Debug)]
pub struct Destination {
    pub path: PathBuf,
    pub force: bool,
}

impl Destination {
    /// Fails when the destination exists and may not be replaced, or is a database that
    /// still has writers.
    pub fn check(&self) -> Result<()> {
        ensure!(
            self.force || !self.path.exists(),
            "{} exists; use --force to replace it",
            self.path.display()
        );
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut sidecar = self.path.as_os_str().to_os_string();
            sidecar.push(suffix);
            ensure!(
                !Path::new(&sidecar).exists(),
                "database has a {suffix} sidecar; close its writers and checkpoint it before replacing"
            );
        }
        Ok(())
    }

    fn parent(&self) -> &Path {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }

    /// A temporary file beside the destination, for [`Destination::publish_file`]. Like
    /// any new file, and unlike a default temporary file, it is readable as the umask
    /// allows, since it becomes the output.
    pub fn temporary_file(&self, suffix: &str) -> Result<tempfile::NamedTempFile> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(".bagkit-").suffix(suffix);
        #[cfg(unix)]
        builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666));
        Ok(builder.tempfile_in(self.parent())?)
    }

    /// A temporary directory beside the destination, for [`Destination::publish_directory`].
    pub fn temporary_directory(&self) -> Result<tempfile::TempDir> {
        Ok(tempfile::Builder::new()
            .prefix(".bagkit-")
            .tempdir_in(self.parent())?)
    }

    /// Syncs a completed file and moves it into place atomically.
    pub fn publish_file(&self, temp: tempfile::NamedTempFile) -> Result<()> {
        temp.as_file().sync_all()?;
        if self.force {
            temp.persist(&self.path)?;
        } else {
            temp.persist_noclobber(&self.path)?;
        }
        sync_directory(self.parent())?;
        Ok(())
    }

    /// Syncs a completed directory and moves it into place. An existing directory is moved
    /// aside first and removed afterwards, so the destination is never half-written; it
    /// is briefly absent.
    pub fn publish_directory(&self, temp: tempfile::TempDir) -> Result<()> {
        for entry in fs::read_dir(temp.path())? {
            File::open(entry?.path())?.sync_all()?;
        }
        sync_directory(temp.path())?;
        let old = if self.path.exists() {
            ensure!(self.force, "{} exists", self.path.display());
            ensure!(
                self.path.is_dir(),
                "{} exists and is not a directory",
                self.path.display()
            );
            let aside = self.temporary_directory()?.keep();
            fs::remove_dir(&aside)?;
            fs::rename(&self.path, &aside)?;
            Some(aside)
        } else {
            None
        };
        fs::rename(temp.keep(), &self.path)?;
        sync_directory(self.parent())?;
        if let Some(old) = old {
            fs::remove_dir_all(old)?;
        }
        Ok(())
    }
}

/// Syncs a directory, so the files created or renamed in it survive a crash. Windows can't
/// open a directory as a file, and NTFS journals these changes itself.
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Stores encoded rows in their format.
pub enum Writer<'db> {
    #[cfg(feature = "sqlite")]
    Sqlite(crate::sqlite::Writer<'db>),
    Csv(CsvWriter),
    #[cfg(feature = "parquet")]
    Parquet(crate::parquet::Writer),
    #[doc(hidden)]
    _Lifetime(std::marker::PhantomData<&'db ()>),
}

impl Writer<'_> {
    pub fn format(&self) -> Format {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(_) => Format::Sqlite,
            Self::Csv(_) => Format::Csv,
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => Format::Parquet,
            Self::_Lifetime(_) => unreachable!(),
        }
    }

    pub fn write(&mut self, encoded: Encoded) -> Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(writer) => writer.write(encoded),
            Self::Csv(writer) => writer.write(encoded),
            #[cfg(feature = "parquet")]
            Self::Parquet(writer) => writer.write(encoded),
            Self::_Lifetime(_) => unreachable!(),
        }
    }

    /// Writes what is buffered and completes the output's files.
    pub fn finish(&mut self) -> Result<()> {
        match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(writer) => writer.flush(),
            Self::Csv(writer) => writer.flush(),
            #[cfg(feature = "parquet")]
            Self::Parquet(writer) => writer.finish(),
            Self::_Lifetime(_) => unreachable!(),
        }
    }
}

/// Files of one table each, by table name.
pub struct Files<W> {
    pub files: HashMap<&'static str, W>,
}

impl<W> Files<W> {
    pub fn get(&mut self, table: &Table) -> Result<&mut W> {
        self.files
            .get_mut(table.name)
            .with_context(|| format!("no output for table {}", table.name))
    }
}

/// CSV files with a header row, one per table.
pub struct CsvWriter {
    files: Files<BufWriter<File>>,
}

impl CsvWriter {
    /// Writes `tables` into `directory` as `<table>.csv`, or the only table to `file`.
    pub fn new(tables: &[&'static Table], target: Target) -> Result<Self> {
        let mut files = HashMap::new();
        for table in tables {
            let file = match &target {
                Target::Directory(directory) => {
                    File::create_new(directory.join(format!("{}.csv", table.name)))?
                }
                Target::File(file) => {
                    ensure!(tables.len() == 1, "a single CSV file holds one table");
                    file.try_clone()?
                }
            };
            let mut file = BufWriter::with_capacity(1 << 20, file);
            writeln!(
                file,
                "{}",
                table.column_names().collect::<Vec<_>>().join(",")
            )?;
            files.insert(table.name, file);
        }
        Ok(Self {
            files: Files { files },
        })
    }

    fn write(&mut self, encoded: Encoded) -> Result<()> {
        let file = self.files.get(encoded.table)?;
        match encoded.content {
            Content::Text(text) => file.write_all(&text)?,
            Content::Rows(_) => bail!("CSV rows must be encoded as text"),
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        for file in self.files.files.values_mut() {
            file.flush()?;
        }
        Ok(())
    }
}

/// Where a file writer puts its output.
pub enum Target<'a> {
    /// One file per table, named after it.
    Directory(&'a Path),
    /// The only table, in this file.
    File(&'a File),
}
