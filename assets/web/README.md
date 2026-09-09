# Web delivery images

These WebP files are smaller presentation copies of the archival PNGs elsewhere in `assets/`. The original art, dimensions, exact generation prompts and original manifests remain unchanged.

`MANIFEST.json` records source/output byte counts, dimensions and SHA-256 hashes. The conversion uses Pillow, WebP quality 84, method 6, with no resizing. Run `python3 scripts/optimize_web_images.py` in an environment with Pillow and WebP support to regenerate them and update embedded Markdown/HTML image references. Plain links to archival sources are preserved. Encoding bytes can vary by Pillow/libwebp version; inspect the result before committing.

The first conversion reduced 38 images from 193,019,559 to 15,120,384 bytes (92.2%). This is image payload reduction, not a measured end-to-end page speed improvement or a reduction of Git history size.
