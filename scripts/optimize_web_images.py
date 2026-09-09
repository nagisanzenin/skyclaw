#!/usr/bin/env python3
"""Create small web derivatives; never overwrite the archival PNG sources.
Requires Pillow with WebP support. Run from any directory.
"""
import hashlib
import json
from pathlib import Path
from PIL import Image, features

ROOT = Path(__file__).resolve().parents[1]
assert features.check('webp'), 'Pillow WebP support required'
records = []
for source in sorted((ROOT / 'assets').rglob('*.png')):
    if 'web' in source.relative_to(ROOT / 'assets').parts:
        continue
    target = ROOT / 'assets/web' / source.relative_to(ROOT / 'assets').with_suffix('.webp')
    target.parent.mkdir(parents=True, exist_ok=True)
    with Image.open(source) as im:
        im.load()
        im = im.convert('RGBA' if 'A' in im.getbands() else 'RGB')
        # Keep the original pixel dimensions: text and character shapes stay legible.
        im.save(target, 'WEBP', quality=84, method=6)
        width, height = im.size
    if target.stat().st_size >= source.stat().st_size:
        target.unlink()
        continue
    records.append({'source': source.relative_to(ROOT).as_posix(), 'web': target.relative_to(ROOT).as_posix(), 'source_bytes': source.stat().st_size, 'web_bytes': target.stat().st_size, 'width': width, 'height': height, 'sha256': hashlib.sha256(target.read_bytes()).hexdigest()})
(ROOT / 'assets/web/MANIFEST.json').write_text(json.dumps({'format': 'WebP', 'quality': 84, 'method': 6, 'resized': False, 'images': records}, indent=2)+'\n')
# Only rewrite embedded images, not source/archive links or historical measurements.
import os
import re
mapping = {(ROOT / row['source']).resolve(): (ROOT / row['web']).resolve() for row in records}
changed = []
for document in sorted(ROOT.rglob('*.md')):
    if any(part in {'.git', 'target', 'node_modules', 'assets'} for part in document.relative_to(ROOT).parts):
        continue
    before = document.read_text()
    def replacement(match):
        url = match.group(2)
        if '://' in url or url.startswith('data:'):
            return match.group(0)
        original = (document.parent / url).resolve()
        if original not in mapping:
            return match.group(0)
        return match.group(1) + Path(os.path.relpath(mapping[original], document.parent)).as_posix() + match.group(3)
    after = re.sub(r'(!\[[^\]]*\]\()([^\s)]+)(\))', replacement, before)
    after = re.sub(r'(<img\b[^>]*?\bsrc=")([^"]+)(")', replacement, after)
    if after != before:
        document.write_text(after)
        changed.append(document.relative_to(ROOT).as_posix())
print(json.dumps({'images': len(records), 'source_bytes': sum(r['source_bytes'] for r in records), 'web_bytes': sum(r['web_bytes'] for r in records), 'documents_updated': changed}))
