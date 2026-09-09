# Entity essay, mathematics and web image delivery

The creator requested an illustrated engineering essay for a Hacker News audience, centered on Tem as a persistent entity around episodic model reasoning. The README now presents that vision and seven mathematical mechanisms with implemented formulas, units, assumptions and source links. The essay is published by the repository's existing GitHub Pages configuration (`main`, `/docs`). No HN submission or comment was posted.

## Artifacts

- `docs/index.html`, `docs/essay.css`, `docs/images/`: standalone static essay; no client JavaScript, analytics, remote fonts or sign-in required on GitHub Pages. Native details expose derivations. Opening image is eager; the second illustration is lazy-loaded with intrinsic dimensions.
- `assets/web/`: WebP presentation derivatives, preserving all 38 original PNGs and their original manifests.
- `scripts/optimize_web_images.py`: reproducible quality/dimension policy, manifest and embedded-image reference updates. Requires Pillow/WebP. Source links remain archival.
- `README.md`: full feature tour preserved, with prominent entity vision and mathematics.

## Size measurements

All 38 source images: **193,019,559 → 15,120,384 bytes** (92.2% reduction). The 18 README images: **48,801,536 → 6,799,656 bytes** (86.1% reduction). These are file payloads, not measured page-load timings. Original pixel dimensions are unchanged. Originals remain in the repository, so this improves web delivery, not clone size or historical Git storage.

## Validation

The essay production build and TypeScript check passed. Focused lint of authored page/export source passed; a full scaffold lint found pre-existing warnings/errors in unused generated UI components, which are not included in the static export. HTML checks validate local assets, source links, unique anchors, image alternatives/dimensions and absence of scripts. Compressed hero visually inspected. No browser screenshot/interaction QA was requested or claimed.

Formulas were checked against v6.0.0 source commit `10f4cc603940bccde087e0c47e73a27bea394257`. The narrative retains unresolved durable-pursuit work, the original 30/30 versus 29/30 A/B result and clarified-pair distinction, and inherited dependency advisory disclosure. No Rust code, release version or historical benchmark result changes.

A Sites source checkout at `work/temm1e-essay` retains the React authoring source and a no-JavaScript static exporter. Its separate private preview is optional for public readers; the GitHub Pages copy is self-contained. Future editors can directly maintain the static HTML/CSS or regenerate from the authoring source.

## Narrative revision

After creator feedback that the first draft felt scattered, the essay follows one explicit design scenario: give Tem work, leave, return later. Each section introduces the next requirement—environment/action, memory, temporal return, evidence, then continuity through interruption/model changes. All seven mathematical mechanisms remain, introduced where they answer the corresponding engineering question rather than grouped into a catalogue. The scenario is explicitly a design objective, not an end-to-end demo claim.

## Mission revision, September 10, 2026

The creator asked for the wider context of improving models and the variety of agent harnesses, followed by Tem's entity-first mission and the endeavours required to pursue it. The published essay now follows that argument. It removes the overnight-task framing, rhetorical pull quotes, dramatic closing line and standalone mathematics catalogue. All seven mechanisms remain within the relevant argument; detailed statistical assumptions and the unchanged benchmark results are expandable.

Editorial pass informed by https://github.com/blader/humanizer/blob/main/SKILL.md (version 3.0.0, read September 10). No em/en dashes appear in the page. The essay keeps an AI-assistance disclosure and does not invent founder anecdotes or claim unique capabilities compared with other harnesses. Field references link to first-party descriptions of Codex, Anthropic, Pi and OpenCode.

For future editing, `docs/index.html` and `docs/essay.css` are now the canonical source as well as the deployable files. Edit them directly. The earlier private Sites/React checkout is historical and must not overwrite this version. GitHub Pages remains the requested public host. No JavaScript build or Sites deployment is needed for these static prose edits.

Publication runner issue: two consecutive CI attempts failed before Rust checks because the preinstalled Google Chrome apt source returned a mismatched Packages index. The CI-only apt helper filters that source from legacy .list files on disposable Actions runners; Ubuntu sources, installed Chrome and signature/hash validation remain intact. Workflow installation steps use the helper. A separate Windows command_check_echo failure was retained and retried.

The runner uses a source format missed by the initial legacy-only filter. The helper now supports Deb822 .sources as well; fixtures cover multiline URIs and preservation of unrelated/mixed source stanzas.
