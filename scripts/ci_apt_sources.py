#!/usr/bin/env python3
"""Filter only Google Chrome entries from disposable runner apt source files."""
import os
from pathlib import Path
import re
from urllib.parse import urlsplit


def is_chrome(uri):
    parsed = urlsplit(uri)
    return parsed.scheme in {'http', 'https'} and parsed.hostname == 'dl.google.com' and bool(
        re.fullmatch(r'/linux/chrome(?:-stable)?/deb/?', parsed.path)
    )


def without_chrome(text, suffix):
    if suffix == '.list':
        return ''.join(line for line in text.splitlines(keepends=True)
                       if not any(is_chrome(word) for word in line.split()))
    kept = []
    for stanza in re.split(r'\n\s*\n', text):
        fields, key = {}, None
        for line in stanza.splitlines():
            if line.startswith('#'):
                continue
            if line[:1].isspace() and key:
                fields[key] += ' ' + line.strip()
            elif ':' in line:
                key, value = line.split(':', 1)
                key = key.lower()
                fields[key] = value.strip()
        uris = fields.get('uris', '').split()
        # Preserve mixed-source stanzas rather than dropping unrelated repos.
        if not uris or not all(is_chrome(uri) for uri in uris):
            kept.append(stanza)
    return '\n\n'.join(kept)


if __name__ == '__main__':
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise SystemExit('Only disposable GitHub Actions runners are supported.')
    for source in Path('/etc/apt/sources.list.d').iterdir():
        if source.suffix in {'.list', '.sources'} and source.is_file():
            before = source.read_text()
            after = without_chrome(before, source.suffix)
            if after != before:
                source.write_text(after)
                print(f'Omitted unrelated Chrome apt entries in {source.name}')
