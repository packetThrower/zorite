// Fetches each crate's README.md from its own repo (packetThrower/<crate>) into
// src/content/docs/reference/crates/<crate>.md — and, when the crate has one,
// its API.md (the complete public-API reference) into <crate>-api.md — with
// Starlight frontmatter prepended. The crate repos stay the single source of
// truth; these pages are regenerated on every build (and on dev startup) so
// the rendered docs never drift. Mirrors scripts/sync-changelog.mjs.
//
// Only a few links in the sources aren't plain https/# anchors, and they're
// the only ones rewritten here:
//   • cross-crate links  github.com/packetThrower/<crate>  -> the sibling docs page
//   • README <-> API.md links        -> the paired docs pages
//   • zorite-markdown's     sample.md  -> the file on GitHub
// Everything else is left untouched — in particular the many `](url)` /
// `](src)` snippets that illustrate Markdown syntax all live inside code
// spans, so they render as literal text and must not be touched.
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const outDir = resolve(here, '../src/content/docs/reference/crates');

const GH = 'https://github.com/packetThrower';
// Mirrors `base` in astro.config.mjs. Root-relative links need the base
// baked in — Astro does not prepend it to plain Markdown hrefs.
const DOCS = '/zorite/reference/crates';

const names = ['gpui-bidi', 'zorite-editor', 'zorite-markdown', 'gpui-pdf', 'gpui-whiteboard', 'os-cursors', 'os-spellcheck', 'ratex-gpui'];
const crateLinks = names.join('|');

// A file from a crate repo's main branch; null when the repo has no such file.
async function raw(name, file) {
	const res = await fetch(`https://raw.githubusercontent.com/packetThrower/${name}/main/${file}`);
	if (res.status === 404) return null;
	if (!res.ok) throw new Error(`sync-crate-readmes: ${name}/${file}: HTTP ${res.status}`);
	return res.text();
}

mkdirSync(outDir, { recursive: true });

// Shared link rewrites for a crate page (README or API.md).
function rewrite(src, name) {
	return (
		src
			// Drop the leading H1 — Starlight renders the title from frontmatter.
			.replace(/^#[^\n]*\n/, '')
			// Cross-crate links -> the sibling docs pages.
			.replace(new RegExp(`\\]\\(${GH}/(${crateLinks})\\)`, 'g'), `](${DOCS}/$1/)`)
			// README <-> API.md pairing (keeps GitHub-relative links working there).
			.replace(/\]\(API\.md(#[^)]*)?\)/g, `](${DOCS}/${name}-api/$1)`)
			.replace(/\]\(README\.md(#[^)]*)?\)/g, `](${DOCS}/${name}/$1)`)
			// The one local-file link (zorite-markdown's sample.md) -> GitHub.
			.replace(/\]\(sample\.md\)/g, `](${GH}/zorite-markdown/blob/main/sample.md)`)
	);
}

function frontmatter(title, description, name, file) {
	return `---
title: ${title}
description: "${description}"
editUrl: ${GH}/${name}/edit/main/${file}
---

`;
}

await Promise.all(
	names.map(async (name) => {
		const [manifest, readme, api] = await Promise.all(
			['Cargo.toml', 'README.md', 'API.md'].map((f) => raw(name, f)),
		);
		// Each page's description is the crate's own Cargo.toml `description`, so
		// crates.io and the docs site say the same thing.
		const description = manifest.match(/^description = "(.*)"$/m)[1].replace(/\\"/g, '"');

		const dst = resolve(outDir, `${name}.md`);
		writeFileSync(dst, frontmatter(name, description, name, 'README.md') + rewrite(readme, name));
		console.log(`sync-crate-readmes: wrote ${dst}`);

		// The crate's complete public-API reference, when it has one.
		if (api) {
			const apiDst = resolve(outDir, `${name}-api.md`);
			writeFileSync(
				apiDst,
				frontmatter(
					`${name} API`,
					`The complete public API of ${name}: signatures, parameters, return contracts, edge cases, and cost notes.`,
					name,
					'API.md',
				) + rewrite(api, name),
			);
			console.log(`sync-crate-readmes: wrote ${apiDst}`);
		}
	}),
);
