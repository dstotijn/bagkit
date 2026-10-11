"""Regenerate src/municipalities.tsv from PDOK's administrative boundaries.

The BAG names no municipalities: the extract's GEM-WPL-RELATIE gives each locality a
municipality code only. This table supplies each code's name and province. Run it after a
municipal reorganisation and commit the result.
"""
import datetime
import io
import json
import pathlib
import urllib.request

URL = 'https://api.pdok.nl/kadaster/brk-bestuurlijke-gebieden/ogc/v1/collections/gemeentegebied/items?f=json&limit=1000'
OUTPUT = pathlib.Path(__file__).resolve().parent.parent / 'src' / 'municipalities.tsv'


def main():
    rows = []
    url = URL
    while url:
        with urllib.request.urlopen(url, timeout=60) as response:
            page = json.load(response)
        for feature in page['features']:
            p = feature['properties']
            rows.append((p['code'], p['naam'], p['ligt_in_provincie_naam']))
        url = next((link['href'] for link in page['links'] if link['rel'] == 'next'), None)
    rows.sort()
    codes = [code for code, _, _ in rows]
    assert len(codes) == len(set(codes)), 'duplicate municipality codes'
    assert all(len(code) == 4 and code.isdigit() for code in codes), 'unexpected municipality code'
    assert not any(c in field for row in rows for field in row for c in '\t\n'), 'field contains a tab'
    out = io.StringIO()
    out.write(f'# PDOK brk-bestuurlijke-gebieden gemeentegebied, retrieved {datetime.date.today()}\n')
    for row in [('code', 'name', 'province'), *rows]:
        out.write('\t'.join(row) + '\n')
    OUTPUT.write_text(out.getvalue(), encoding='utf-8')
    print(f'Wrote {len(rows)} municipalities to {OUTPUT}')


if __name__ == '__main__':
    main()
