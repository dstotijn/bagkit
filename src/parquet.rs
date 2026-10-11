//! Parquet output, one file per table, compressed with zstd. Geometrie is stored as
//! GeoParquet WKB in RD New, described in each file's `geo` metadata.
//!
//! Encoding and compressing a row group is the expensive part, so each full row group is
//! encoded on a helper thread into in-memory column chunks; the writer appends finished
//! row groups to the file in order.
use crate::{
    dataset::{Content, Encoded, Kind, Table, Value},
    output::{Files, Target},
};
use anyhow::{Result, bail};
use bytes::Bytes;
use parquet::{
    basic::{Compression, LogicalType, Repetition, Type as PhysicalType, ZstdLevel},
    column::writer::{ColumnCloseResult, get_column_writer, get_typed_column_writer},
    data_type::{BoolType, ByteArray, ByteArrayType, DataType, DoubleType, Int64Type},
    file::{
        metadata::KeyValue,
        properties::{WriterProperties, WriterPropertiesPtr},
        writer::{SerializedFileWriter, SerializedPageWriter, TrackedWrite},
    },
    schema::types::{ColumnDescPtr, Type},
};
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    sync::Arc,
    thread::{self, JoinHandle},
};

/// Row groups of one table encoding at once; more waits for the oldest to finish.
const IN_FLIGHT: usize = 2;

/// A row group's column chunks, encoded.
type Chunks = Vec<(Bytes, ColumnCloseResult)>;

/// Rows buffered per row group.
const ROW_GROUP: usize = 65_536;

/// RD New (EPSG:28992) as PROJJSON, for GeoParquet's `crs`.
const RD_NEW_PROJJSON: &str = r#"{"$schema":"https://proj.org/schemas/v0.7/projjson.schema.json","type":"ProjectedCRS","name":"Amersfoort / RD New","base_crs":{"name":"Amersfoort","datum":{"type":"GeodeticReferenceFrame","name":"Amersfoort","ellipsoid":{"name":"Bessel 1841","semi_major_axis":6377397.155,"inverse_flattening":299.1528128}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]},"id":{"authority":"EPSG","code":4289}},"conversion":{"name":"RD New","method":{"name":"Oblique Stereographic","id":{"authority":"EPSG","code":9809}},"parameters":[{"name":"Latitude of natural origin","value":52.1561605555556,"unit":"degree","id":{"authority":"EPSG","code":8801}},{"name":"Longitude of natural origin","value":5.38763888888889,"unit":"degree","id":{"authority":"EPSG","code":8802}},{"name":"Scale factor at natural origin","value":0.9999079,"unit":"unity","id":{"authority":"EPSG","code":8805}},{"name":"False easting","value":155000,"unit":"metre","id":{"authority":"EPSG","code":8806}},{"name":"False northing","value":463000,"unit":"metre","id":{"authority":"EPSG","code":8807}}]},"coordinate_system":{"subtype":"Cartesian","axis":[{"name":"Easting","abbreviation":"X","direction":"east","unit":"metre"},{"name":"Northing","abbreviation":"Y","direction":"north","unit":"metre"}]},"id":{"authority":"EPSG","code":28992}}"#;

/// Values of one column waiting for the next row group, with definition levels for
/// optional columns.
enum Buffer {
    Integer(Vec<i64>),
    Boolean(Vec<bool>),
    Real(Vec<f64>),
    Bytes(Vec<ByteArray>),
}

struct Column {
    optional: bool,
    levels: Vec<i16>,
    values: Buffer,
}

struct TableWriter {
    table: &'static Table,
    file: SerializedFileWriter<File>,
    properties: WriterPropertiesPtr,
    columns: Vec<Column>,
    rows: usize,
    /// Row groups being encoded, oldest first.
    encoding: VecDeque<JoinHandle<Result<Chunks>>>,
}

impl TableWriter {
    fn new(table: &'static Table, file: File) -> Result<Self> {
        let fields = table
            .columns
            .iter()
            .map(|column| {
                let (physical, logical) = match column.kind {
                    Kind::Integer | Kind::Identifier(_) => (PhysicalType::INT64, None),
                    Kind::Boolean => (PhysicalType::BOOLEAN, None),
                    Kind::Real(_) => (PhysicalType::DOUBLE, None),
                    Kind::Text => (PhysicalType::BYTE_ARRAY, Some(LogicalType::String)),
                    Kind::Geometry => (PhysicalType::BYTE_ARRAY, None),
                };
                let repetition = if column.nullable {
                    Repetition::OPTIONAL
                } else {
                    Repetition::REQUIRED
                };
                Ok(Arc::new(
                    Type::primitive_type_builder(column.name, physical)
                        .with_repetition(repetition)
                        .with_logical_type(logical)
                        .build()?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let schema = Arc::new(
            Type::group_type_builder(table.name)
                .with_fields(fields)
                .build()?,
        );
        let properties = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
            .set_max_row_group_row_count(Some(ROW_GROUP))
            .build();
        let properties = Arc::new(properties);
        let file = SerializedFileWriter::new(file, schema, properties.clone())?;
        let columns = table
            .columns
            .iter()
            .map(|column| Column {
                optional: column.nullable,
                levels: Vec::new(),
                values: match column.kind {
                    Kind::Integer | Kind::Identifier(_) => Buffer::Integer(Vec::new()),
                    Kind::Boolean => Buffer::Boolean(Vec::new()),
                    Kind::Real(_) => Buffer::Real(Vec::new()),
                    Kind::Text | Kind::Geometry => Buffer::Bytes(Vec::new()),
                },
            })
            .collect();
        Ok(Self {
            table,
            file,
            properties,
            columns,
            rows: 0,
            encoding: VecDeque::new(),
        })
    }

    fn push(&mut self, row: Vec<Value>) -> Result<()> {
        for (column, value) in self.columns.iter_mut().zip(row) {
            let present = !matches!(value, Value::Null);
            if column.optional {
                column.levels.push(i16::from(present));
            }
            match (&mut column.values, value) {
                (_, Value::Null) if column.optional => (),
                (Buffer::Integer(values), Value::Integer(i)) => values.push(i),
                (Buffer::Boolean(values), Value::Integer(b)) => values.push(b != 0),
                (Buffer::Real(values), Value::Real(r)) => values.push(r),
                (Buffer::Bytes(values), Value::Text(t)) => values.push(t.into_bytes().into()),
                (Buffer::Bytes(values), Value::Blob(b)) => values.push(b.into()),
                (_, value) => bail!("{} cannot store {value:?} in this column", self.table.name),
            }
        }
        self.rows += 1;
        if self.rows == ROW_GROUP {
            self.flush()?;
        }
        Ok(())
    }

    /// Starts encoding the buffered rows as a row group, and appends row groups that are
    /// done while too many are in flight.
    fn flush(&mut self) -> Result<()> {
        if self.rows == 0 {
            return Ok(());
        }
        let descriptors: Vec<ColumnDescPtr> = self.file.schema_descr().columns().to_vec();
        let columns: Vec<Column> = self
            .columns
            .iter_mut()
            .map(|column| Column {
                optional: column.optional,
                levels: std::mem::take(&mut column.levels),
                values: column.values.take(),
            })
            .collect();
        let properties = self.properties.clone();
        self.encoding.push_back(
            thread::Builder::new()
                .name("bagkit-parquet".into())
                .spawn(move || encode(descriptors, properties, columns))?,
        );
        self.rows = 0;
        while self.encoding.len() > IN_FLIGHT {
            self.append_oldest()?;
        }
        Ok(())
    }

    /// Waits for the oldest row group being encoded and appends it to the file.
    fn append_oldest(&mut self) -> Result<()> {
        let Some(handle) = self.encoding.pop_front() else {
            return Ok(());
        };
        let chunks = handle
            .join()
            .map_err(|_| anyhow::anyhow!("Parquet encoding panicked"))??;
        let mut group = self.file.next_row_group()?;
        for (bytes, close) in chunks {
            group.append_column(&bytes, close)?;
        }
        group.close()?;
        Ok(())
    }

    /// Writes the last row group, GeoParquet metadata when the table has geometrie, and
    /// the footer.
    fn close(mut self) -> Result<()> {
        self.flush()?;
        while !self.encoding.is_empty() {
            self.append_oldest()?;
        }
        if let Some(column) = self.table.geometry_column() {
            let geo = serde_json::json!({
                "version": "1.1.0",
                "primary_column": column,
                "columns": {
                    column: {
                        "encoding": "WKB",
                        "geometry_types": ["Polygon"],
                        "crs": serde_json::from_str::<serde_json::Value>(RD_NEW_PROJJSON)?,
                    }
                }
            });
            self.file
                .append_key_value_metadata(KeyValue::new("geo".into(), geo.to_string()));
        }
        self.file.close()?;
        Ok(())
    }
}

impl Buffer {
    /// The values so far, leaving an empty buffer of the same kind.
    fn take(&mut self) -> Self {
        match self {
            Self::Integer(v) => Self::Integer(std::mem::take(v)),
            Self::Boolean(v) => Self::Boolean(std::mem::take(v)),
            Self::Real(v) => Self::Real(std::mem::take(v)),
            Self::Bytes(v) => Self::Bytes(std::mem::take(v)),
        }
    }
}

/// Encodes one row group's columns into in-memory column chunks.
fn encode(
    descriptors: Vec<ColumnDescPtr>,
    properties: WriterPropertiesPtr,
    columns: Vec<Column>,
) -> Result<Chunks> {
    descriptors
        .into_iter()
        .zip(columns)
        .map(|(descriptor, column)| {
            let mut sink = TrackedWrite::new(Vec::new());
            let close = {
                let pages = Box::new(SerializedPageWriter::new(&mut sink));
                let writer = get_column_writer(descriptor, properties.clone(), pages);
                let levels = column.optional.then_some(&column.levels[..]);
                match column.values {
                    Buffer::Integer(values) => write::<Int64Type>(writer, &values, levels)?,
                    Buffer::Boolean(values) => write::<BoolType>(writer, &values, levels)?,
                    Buffer::Real(values) => write::<DoubleType>(writer, &values, levels)?,
                    Buffer::Bytes(values) => write::<ByteArrayType>(writer, &values, levels)?,
                }
            };
            Ok((Bytes::from(sink.into_inner()?), close))
        })
        .collect()
}

fn write<T: DataType>(
    writer: parquet::column::writer::ColumnWriter<'_>,
    values: &[T::T],
    levels: Option<&[i16]>,
) -> Result<ColumnCloseResult> {
    let mut writer = get_typed_column_writer::<T>(writer);
    writer.write_batch(values, levels, None)?;
    Ok(writer.close()?)
}

/// Parquet files, one per table.
pub struct Writer {
    files: Files<TableWriter>,
}

impl Writer {
    /// Writes `tables` into `directory` as `<table>.parquet`, or the only table to `file`.
    pub fn new(tables: &[&'static Table], target: Target) -> Result<Self> {
        let mut files = HashMap::new();
        for &table in tables {
            let file = match &target {
                Target::Directory(directory) => {
                    File::create_new(directory.join(format!("{}.parquet", table.name)))?
                }
                Target::File(file) => {
                    anyhow::ensure!(tables.len() == 1, "a single Parquet file holds one table");
                    file.try_clone()?
                }
            };
            files.insert(table.name, TableWriter::new(table, file)?);
        }
        Ok(Self {
            files: Files { files },
        })
    }

    pub fn write(&mut self, encoded: Encoded) -> Result<()> {
        let writer = self.files.get(encoded.table)?;
        let Content::Rows(rows) = encoded.content else {
            bail!("Parquet rows must stay values");
        };
        rows.into_iter().try_for_each(|row| writer.push(row))
    }

    /// Completes every file.
    pub fn finish(&mut self) -> Result<()> {
        for (_, writer) in self.files.files.drain() {
            writer.close()?;
        }
        Ok(())
    }
}
