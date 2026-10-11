-- Adresseerbare objecten with at least one current nummeraanduiding, in RD New
-- millimetres. Integer coordinates
-- are exact, so distances need no auxiliary full-precision columns. The importer bulk
-- loads it in Hilbert order (see src/rtree.rs).
CREATE VIRTUAL TABLE adresseerbare_objecten_rtree USING rtree_i32 (
    id,                                        -- adresseerbare_objecten.identificatie
    min_x, max_x,
    min_y, max_y
);

-- One row per nummeraanduiding, including ingetrokken ones; filter on
-- nummeraanduiding_status. Names follow Kadaster's BAG API.
CREATE VIEW adressen AS
SELECT
    n.identificatie AS nummeraanduiding_identificatie,
    o.naam AS openbare_ruimte_naam,
    n.huisnummer,
    n.huisletter,
    n.huisnummertoevoeging,
    n.postcode,
    w.naam AS woonplaats_naam,
    g.naam AS gemeente_naam,
    g.provincie_naam,
    n.status AS nummeraanduiding_status,
    n.openbare_ruimte_identificatie,
    w.identificatie AS woonplaats_identificatie,
    n.adresseerbaar_object_identificatie,
    n.indicatie_nevenadres,
    a.type AS type_adresseerbaar_object,
    a.status AS adresseerbaar_object_status,
    a.gebruiksdoelen,
    a.oppervlakte,
    a.rd_x,
    a.rd_y,
    a.breedtegraad,
    a.lengtegraad
FROM nummeraanduidingen AS n
JOIN openbare_ruimten AS o ON o.identificatie = n.openbare_ruimte_identificatie
JOIN woonplaatsen AS w ON w.identificatie = COALESCE(n.woonplaats_identificatie, o.woonplaats_identificatie)
LEFT JOIN gemeenten AS g ON g.code = w.gemeente_code
LEFT JOIN adresseerbare_objecten AS a ON a.identificatie = n.adresseerbaar_object_identificatie;
