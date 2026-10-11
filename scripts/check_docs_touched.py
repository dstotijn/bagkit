"""Fail when a change touches documented code without touching any page that documents it.

python scripts/check_docs_touched.py BASE [HEAD]

Compares BASE...HEAD (HEAD defaults to the working tree's HEAD) against
scripts/docs_map.toml. Add the `docs-not-needed` label to a pull request to skip this check
when a change does not affect documented behaviour.
"""
import fnmatch
import pathlib
import subprocess
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = 'web/src/content/docs/'


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    base, head = sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else 'HEAD'
    changed = subprocess.run(
        ['git', 'diff', '--name-only', f'{base}...{head}'],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout.split()
    mapping = tomllib.loads((ROOT / 'scripts/docs_map.toml').read_text())['map']
    missing = []
    for entry in mapping:
        code = [f for f in changed if any(fnmatch.fnmatch(f, pattern) for pattern in entry['code'])]
        docs = [DOCS + page for page in entry['docs']]
        for page in docs:
            if not (ROOT / page).exists():
                sys.exit(f'scripts/docs_map.toml names a missing page: {page}')
        if code and not any(f in changed for f in docs):
            missing.append((code, docs))
    for code, docs in missing:
        print(f'{", ".join(code)} changed, but none of these pages did:')
        for page in docs:
            print(f'  {page}')
    if missing:
        print('\nUpdate the pages, or label the pull request docs-not-needed if behaviour is unchanged.')
        sys.exit(1)
    print('Documentation was updated alongside documented code.')


if __name__ == '__main__':
    main()
