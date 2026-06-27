// Copies each workspace crate's README.md into
// src/content/docs/reference/crates/<crate>.md with Starlight frontmatter
// prepended. The crate READMEs stay the single source of truth; these pages
// are regenerated on every build (and on dev startup) so the rendered docs
// never drift from the READMEs. Mirrors scripts/sync-changelog.mjs.
//
// Only three links in the READMEs aren't plain https/# anchors, and they're
// the only ones rewritten here:
//   • cross-crate links  ../<crate>  -> the sibling docs page
//   • gpui-markdown's     sample.md  -> the file on GitHub
// Everything else is left untouched — in particular the many `](url)` /
// `](src)` snippets that illustrate Markdown syntax all live inside code
// spans, so they render as literal text and must not be touched.
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../..');
const outDir = resolve(here, '../src/content/docs/reference/crates');

const GH = 'https://github.com/packetThrower/zorite';
// Mirrors `base` in astro.config.mjs. Root-relative links need the base
// baked in — Astro does not prepend it to plain Markdown hrefs.
const DOCS = '/zorite/reference/crates';

const crates = [
	{
		name: 'gpui-editor',
		description:
			"A from-scratch multi-line text editor for GPUI — the engine behind Zorite's Word-like note editor.",
	},
	{
		name: 'gpui-markdown',
		description:
			'A host-agnostic Markdown renderer for GPUI: wrapping text, clickable links, images, mermaid, and in-page find.',
	},
	{
		name: 'gpui-pdf',
		description:
			'Page-virtualized PDF viewing for GPUI, rasterized with the pure-Rust hayro engine.',
	},
	{
		name: 'gpui-whiteboard',
		description:
			'An infinite, pannable and zoomable freeform whiteboard canvas for GPUI.',
	},
	{
		name: 'os-spellcheck',
		description:
			'Native OS spell-checking (NSSpellChecker / ISpellChecker) with a tiny, host-agnostic API.',
	},
	{
		name: 'ratex-gpui',
		description:
			'A structural (MathQuill-style) math editor for GPUI, plus a LaTeX → image / PNG / SVG renderer, built on the RaTeX engine.',
	},
];

mkdirSync(outDir, { recursive: true });

for (const { name, description } of crates) {
	const body = readFileSync(resolve(repo, 'crates', name, 'README.md'), 'utf8')
		// Drop the leading H1 — Starlight renders the title from frontmatter.
		.replace(/^#[^\n]*\n/, '')
		// Cross-crate links -> the sibling docs pages (inline + ref-def forms).
		.replace(/\]\(\.\.\/([a-z0-9-]+)\)/g, `](${DOCS}/$1/)`)
		.replace(/^(\[[^\]]+\]:\s*)\.\.\/([a-z0-9-]+)\s*$/gm, `$1${DOCS}/$2/`)
		// The one local-file link (gpui-markdown's sample.md) -> GitHub.
		.replace(
			/\]\(sample\.md\)/g,
			`](${GH}/blob/main/crates/gpui-markdown/sample.md)`,
		);

	const frontmatter = `---
title: ${name}
description: "${description}"
editUrl: ${GH}/edit/main/crates/${name}/README.md
---

`;

	const dst = resolve(outDir, `${name}.md`);
	writeFileSync(dst, frontmatter + body);
	console.log(`sync-crate-readmes: wrote ${dst}`);
}
