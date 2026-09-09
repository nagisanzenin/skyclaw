# Temm1e artwork — playful punk science

Approved direction, September 9, 2026. The creator asked for a more personal punk, AuDHD, mathematical, scientific and futuristic style, while preserving the playful groups of Tems and cool bipedal variants in the original illustrations. This supersedes the uniformly sparse diagram direction for new artwork. It extends the original character brief; it does not replace Tem's identity.

## The feeling

A homemade research club populated by a litter of curious Tems. They build, explore, think, help, play and rest. Science has warmth and personality. A scrappy punk-science home and future workshop, with the lovable jank of a 2005 pixel avatar. The creator subsequently rejected empty paper backgrounds: Tems must inhabit a cozy, interesting place.

Express nonlinear curiosity through composition, associative little discoveries and varied activity. Do not use medical imagery, diagnostic labels or stereotypes. Avoid generic glossy cyberpunk, chrome, corporate vector mascots and ornate steampunk frames.

## Character invariants

The creator-approved [model sheet](../assets/character/tem-character-study.png) and [face/body study](TEM_CHARACTER_EXPRESSIONS.md) are mandatory references. The original Gaze and Anima artwork remains the source of truth for the requested hairstyle, faces and lean body.

- Full dark crown, diagonal/parted fringe and long messy cheek-framing locks, based on the original Gaze Tem. Feminine, pretty, eccentric-genius hair; never a spiky black ring around a large bare white forehead. See [hair and expression study](TEM_CHARACTER_EXPRESSIONS.md).
- Left eye amber/gold; right eye ice blue. These are the signature heterochromia colors. Keep their orientation consistent in a front-facing pose; side views can naturally hide one eye.
- Hot-pink scarf, tiny dark-red heart, gold `1` fur marking and lavender ear/detail accents.
- Compact colored eyes, tiny nose and small cat-like mouth. Vary curiosity, mischievous focus, skepticism, delight and sleepy contentment across the group; do not clone one :3face. Oversized head, stubby limbs, playful asymmetry and gold pixel sparkles remain.
- Original base-sprite logic is roughly 26–32 pixels. When making actual sprite assets, use a real low-resolution grid and nearest-neighbor scaling. Generated large illustrations emulate that chunky language; do not falsely describe their entire raster as a literal 32-pixel sprite.
- Bipedal Tem is the same creature standing on two short legs, with paws and tail. Lab coat, glasses, goggles or tool belt can identify a role. It is not a human wearing cat ears. Accessories must not erase signature identity.

Character palette: white `#FFFFFF`, black `#000000`, pink `#FF69B4`, red `#DC2850`, gold `#FFB428`, cyan `#50C8FF`, lavender `#BEA8DC`. Environment may use restrained paper/graphite and muted prop colors, while these accents remain dominant. Pixel bodies remain flat and legible; fine paper texture belongs primarily to the environment.

## Composition and storytelling

Use a wide 2:1 canvas for README/feature illustrations. Keep one clear title and one short tagline. A scene should explain the feature through actions, not a wall of claims.

Usually include 4–7 Tems, with a different pose/activity for each. Combine one or two focused workers with helpers, a playful discovery and a resting Tem. Include a bipedal role where it serves the scene. Characters should interact: passing a task card, inspecting together, untangling a cable, helping a friend or chasing a star. Do not line up identical copied mascots.

Design a continuous full-bleed room with foreground, midground and background: walls, floor, shelves, warm lamps, windows, plants, books, soft resting spots and personal tools. Use muted lavender/indigo environments with amber light and signature pink/cyan accents. Each feature is a different nook in the same cozy future home. The punk notebook language belongs ON wall boards, taped notes, books and patched furniture, rather than an empty paper canvas. Preserve readable silhouettes and a clear wall sign/title area. Dense detail should reward looking closer, not prevent reading the main idea.

Feature diagrams may use up to three useful short labels. Reduce incidental slogans; leave most prop text abstract. The hero can be richer and more personal. Never fill every blank area merely because generation can add detail.

## Truth and mathematical content

Illustrations are concepts, not screenshots or test evidence. Do not put old benchmark percentages, version numbers, model rankings, unverified speedups, security guarantees or invented interface measurements into the art. Keep measured results in dated text reports.

Use simple correct mathematics when explicit notation is needed: a labelled complex unit circle, Euler's identity, a valid graph, or a correctly written Bayes proportionality. Curves without data are decorative and must not be labelled measured performance. Playful metaphorical doodles in the hero are not implementation formulas; avoid adding them to explanatory diagrams. Never manufacture scientific authority through dense meaningless equations.

## Repeatable generation workflow

1. Read this guide and the feature's current implementation/limitations. Write the exact title, tagline, three optional labels and one concrete scene.
2. Use `assets/character/tem-character-study.png` as the primary character reference and `assets/character/tem-expression-atlas.png` for expressions. Use `assets/modernization/banner.png` ONLY for the cozy Den environment and lighting; never let an environmental reference override approved hair, face or slim body proportions. Use original multi-Tem artwork only for character activity/bipedal poses; explicitly prohibit copying its obsolete copy, numerical claims and ornate framing.
3. Preserve character invariants in every prompt. State the actions of individual Tems and the intended interaction.
4. Generate one asset per request with imagegen. Inspect the actual result for title spelling, eye colors, scarf/mark, character variation, readable scene, excessive copy and misleading mathematics/claims.
5. Save approved assets in `assets/modernization/` under their existing semantic filenames. Update `MANIFEST.json` with actual SHA-256 values and keep exact per-asset prompts in `assets/modernization/PROMPTS.json`.
6. Update README/feature references if filenames change. Keep old artwork in Git history; original historical report assets can remain to preserve those reports. Do not rewrite historical experiment evidence to match new art.
7. Generation is not deterministic. A prompt and reference set make the direction reproducible, not a pixel-identical guarantee. Review each output rather than trusting the prompt alone.

## Prompt template

> Wide 2:1 Temm1e illustration in the approved playful punk-science notebook style. Full-bleed cozy futuristic room with depth, warm lamps, shelves, plants, soft resting spaces and personal scientific tools. Ragged pixel Tems, ink/tape details on physical objects, pink/cyan/gold/lavender accents. Preserve black fluff, white face, amber-left/cyan-right eyes, pink heart scarf and gold1mark. Include [number] distinct Tems: [individual actions], with [bipedal role] and [play/rest moment]. Scene: [feature-specific activity]. Exact title: “[title]”. Exact tagline: “[tagline]”. Optional labels: [labels]. Keep most incidental prop text abstract. No copied historical benchmark claims, fake UI data, generic cyberpunk, human cat-ear characters or cloned poses. Strong legibility within a rich inhabited background; no empty paper canvas. Conceptual artwork, not a screenshot.

## Web delivery

Keep original generated PNGs and their source manifests as the archival art. Publish small WebP presentation copies through `scripts/optimize_web_images.py`; record their hashes and dimensions separately in `assets/web/MANIFEST.json`. README and feature-document embedded images should use these derivatives. Preserve full-resolution source links for character/art reviews. The essay uses two compressed images, intrinsic image dimensions and lazy loading below the opening image. Do not regenerate character artwork merely to reduce file size.
