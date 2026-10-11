---
title: Peildatum
description: How bagkit decides which voorkomen of each object is current.
---

Every conversion has one **peildatum**, by default today's local date. The output describes the BAG as it stood on that date.

```sh
bagkit convert lvbag-extract-nl.zip --date 2026-10-08
```

## Current voorkomens

A voorkomen is **current** when:

1. its validity period contains the peildatum: its `beginGeldigheid` is on or before the peildatum, and its `eindGeldigheid` is after it or missing, which means it is still valid; and
2. it is neither inactief (`tijdstipInactief`, `tijdstipInactiefLV`) nor niet-BAG (`tijdstipNietBagLV`).

The start is inclusive and the end exclusive, so a voorkomen that ends on the peildatum is no longer current, and one that begins on it is.

The relations between woonplaatsen and gemeenten follow the same rule with their own validity periods. When a municipal merger takes effect on a future date, a conversion at today's date still shows the old gemeente; one at or after that date shows the new one.

## Choosing a date

- **Today** (the default) is right for most uses.
- **The extract's date**, recorded as `extract_date` in the [metadata](/reference/metadata/), reproduces the extract exactly as Kadaster published it.
- **A future date** shows changes the extract already knows about, such as nummeraanduidingen that become valid next month.
- **A past date** works only as far as the extract's history goes, and corrections made since then are already applied.

## Voorkomens that ended earlier

About half of the extract's voorkomens ended before any recent peildatum. bagkit skips them without parsing them, which is a large part of its speed. They still count in the "records read" total that a conversion prints.
