CREATE INDEX nummeraanduidingen_by_postcode
    ON nummeraanduidingen (postcode, huisnummer, huisletter, huisnummertoevoeging);
CREATE INDEX nummeraanduidingen_by_adresseerbaar_object
    ON nummeraanduidingen (adresseerbaar_object_identificatie);
CREATE INDEX nummeraanduidingen_by_openbare_ruimte
    ON nummeraanduidingen (openbare_ruimte_identificatie);
CREATE INDEX openbare_ruimten_by_naam ON openbare_ruimten (naam);
CREATE INDEX verblijfsobject_panden_by_pand
    ON verblijfsobject_panden (pand_identificatie);
