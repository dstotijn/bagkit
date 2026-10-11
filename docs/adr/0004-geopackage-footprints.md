# Store footprints as GeoPackage geometry in RD New

For a toolkit, building and object footprints are BAG data, not an optional extra. They are stored as GeoPackage geometry in RD New, and the database is registered as a GeoPackage, so QGIS and GDAL open `buildings` and `addressable_objects` directly as feature layers with their attributes. GeoPackage geometry also keeps interior rings, which the earlier polygon text format dropped. RD New is kept because the BAG supplies it to millimetre precision and it measures distances in metres; WGS84 is derived only for points.

## Consequences

- GDAL warns that the file lacks a `.gpkg` extension; the warning is harmless.
- Footprints take about 2.5 GiB of the national database. `import --no-geometry` leaves them out.
- Footprints first lived in separate tables; an earlier, unpublished schema moved them into the tables of panden and adresseerbare objecten, removing about 23.6 million rows.
