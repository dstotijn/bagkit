//! Toolkit for the Dutch BAG (Basisregistratie Adressen en Gebouwen): read extracts,
//! shape their current voorkomens into tables or an adreslijst, and write them as SQLite,
//! CSV or Parquet; look adressen up in SQLite output.
//!
//! - [`extract`] reads the current voorkomens of selected object types in parallel.
//! - [`tables`] and [`address_list`] shape voorkomens into rows.
//! - [`output`], [`sqlite`] and [`parquet`] write rows; [`convert`] runs all of it.
//! - [`query`] looks adressen up in SQLite output, [`api`] shapes them into the HTTP
//!   API's resources and [`server`] serves those.

/// The output layout version, recorded in every output's metadata.
pub const SCHEMA_VERSION: &str = "1";

pub mod address_list;
#[cfg(feature = "sqlite")]
pub mod api;
#[cfg(feature = "cli")]
pub mod cli;
pub mod convert;
pub mod dataset;
pub mod derived;
pub mod extract;
pub mod geo;
pub mod geometry;
pub mod ids;
pub mod municipalities;
pub mod output;
#[cfg(feature = "parquet")]
pub mod parquet;
#[cfg(feature = "sqlite")]
pub mod query;
#[cfg(feature = "sqlite")]
pub mod rtree;
#[cfg(feature = "server")]
pub mod server;
pub mod source;
#[cfg(feature = "sqlite")]
pub mod sqlite;
pub mod tables;
pub mod xml;
