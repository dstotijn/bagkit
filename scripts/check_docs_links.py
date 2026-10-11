"""Fail when the built documentation site links to a page or anchor that does not exist.

Run after building the site: python scripts/check_docs_links.py [web/dist]

Set SITE_URL as for the build when the site has a base path, such as
https://dstotijn.github.io/bagkit: every link must then start with that path.
"""
import html
import os
import pathlib
import re
import sys
import urllib.parse


def main():
    dist = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'web/dist')
    pages = {str(p.relative_to(dist)): p.read_text(encoding='utf-8') for p in dist.rglob('*.html')}
    if not pages:
        sys.exit(f'no pages in {dist}; build the site first')
    ids = {name: set(re.findall(r'id="([^"]+)"', text)) for name, text in pages.items()}
    base = urllib.parse.urlparse(os.environ.get('SITE_URL', '')).path.rstrip('/')
    broken = set()
    for name, text in pages.items():
        for link in re.findall(r'href="(/[^"]*)"', text):
            link = html.unescape(link)
            if not link.startswith(base + '/'):
                broken.add((name, link))
                continue
            href = link[len(base):]
            if href.startswith(('/_astro/', '/pagefind/', '/favicon', '/sitemap')):
                continue
            path, _, anchor = href.partition('#')
            path = path.strip('/')
            target = f'{path}/index.html' if path else 'index.html'
            if target not in pages or (anchor and anchor not in ids[target]):
                broken.add((name, link))
    for name, href in sorted(broken):
        print(f'{name}: broken link {href}')
    if broken:
        sys.exit(1)
    print(f'{len(pages)} pages, no broken links')


if __name__ == '__main__':
    main()
