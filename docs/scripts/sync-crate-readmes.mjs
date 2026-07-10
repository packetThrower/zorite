// Copies each workspace crate's README.md into
// src/content/docs/reference/crates/<crate>.md — and, when the crate has one,
// its API.md (the complete public-API reference) into
// <crate>-api.md — with Starlight frontmatter prepended. The crate files stay
// the single source of truth; these pages are regenerated on every build (and
// on dev startup) so the rendered docs never drift. Mirrors
// scripts/sync-changelog.mjs.
//
// Only a few links in the sources aren't plain https/# anchors, and they're
// the only ones rewritten here:
//   • cross-crate links  ../<crate>  -> the sibling docs page
//   • README <-> API.md links        -> the paired docs pages
//   • gpui-markdown's     sample.md  -> the file on GitHub
// Everything else is left untouched — in particular the many `](url)` /
// `](src)` snippets that illustrate Markdown syntax all live inside code
// spans, so they render as literal text and must not be touched.
import { existsSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs';
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
		name: 'os-cursors',
		description:
			'Per-app custom mouse cursors without forking the UI toolkit — NSCursor swizzling, a WM_SETCURSOR hook, XCURSOR_* environment; packs are standard XCursor themes.',
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

// Shared link rewrites for a crate page (README or API.md).
function rewrite(src, name) {
	return (
		src
			// Drop the leading H1 — Starlight renders the title from frontmatter.
			.replace(/^#[^\n]*\n/, '')
			// Cross-crate links -> the sibling docs pages (inline + ref-def forms).
			.replace(/\]\(\.\.\/([a-z0-9-]+)\)/g, `](${DOCS}/$1/)`)
			.replace(/^(\[[^\]]+\]:\s*)\.\.\/([a-z0-9-]+)\s*$/gm, `$1${DOCS}/$2/`)
			// README <-> API.md pairing (keeps GitHub-relative links working there).
			.replace(/\]\(API\.md(#[^)]*)?\)/g, `](${DOCS}/${name}-api/$1)`)
			.replace(/\]\(README\.md(#[^)]*)?\)/g, `](${DOCS}/${name}/$1)`)
			// The one local-file link (gpui-markdown's sample.md) -> GitHub.
			.replace(
				/\]\(sample\.md\)/g,
				`](${GH}/blob/main/crates/gpui-markdown/sample.md)`,
			)
	);
}

function frontmatter(title, description, editFile) {
	return `---
title: ${title}
description: "${description}"
editUrl: ${GH}/edit/main/${editFile}
---

`;
}

for (const { name, description } of crates) {
	const body = rewrite(
		readFileSync(resolve(repo, 'crates', name, 'README.md'), 'utf8'),
		name,
	);
	const dst = resolve(outDir, `${name}.md`);
	writeFileSync(
		dst,
		frontmatter(name, description, `crates/${name}/README.md`) + body,
	);
	console.log(`sync-crate-readmes: wrote ${dst}`);

	// The crate's complete public-API reference, when it has one.
	const apiSrc = resolve(repo, 'crates', name, 'API.md');
	if (existsSync(apiSrc)) {
		const apiBody = rewrite(readFileSync(apiSrc, 'utf8'), name);
		const apiDst = resolve(outDir, `${name}-api.md`);
		writeFileSync(
			apiDst,
			frontmatter(
				`${name} API`,
				`The complete public API of ${name}: signatures, parameters, return contracts, edge cases, and cost notes.`,
				`crates/${name}/API.md`,
			) + apiBody,
		);
		console.log(`sync-crate-readmes: wrote ${apiDst}`);
	}
}
