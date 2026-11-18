// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import sitemap from '@astrojs/sitemap';

// https://astro.build/config
export default defineConfig({
	site: 'https://packetthrower.github.io',
	// LOWERCASE repo name — the GitHub repo is github.com/packetThrower/zorite
	// (lowercase), even though the product/display name is "Zorite".
	base: '/zorite/',
	trailingSlash: 'ignore',
	integrations: [
		starlight({
			title: 'Zorite',
			description:
				'A local-first, Logseq-style daily journal for the desktop — with a Word-like typing experience. Linked Markdown notes, embedded PDFs, freeform whiteboards, and full-text search, all in a local SQLite database.',
			favicon: '/favicon.svg',
			customCss: ['@packetthrower/site-theme/styles/starlight.css'],

			// Site-wide head additions for discoverability: the Open
			// Graph / Twitter image. Without this, link-preview
			// unfurlers (Slack, Discord, Twitter, iMessage, etc.)
			// render every share as a bare text card.
			head: [
				{
					tag: 'meta',
					attrs: {
						property: 'og:image',
						content: 'https://packetthrower.github.io/zorite/og-image.png',
					},
				},
				{
					tag: 'meta',
					attrs: {
						property: 'og:image:width',
						content: '1200',
					},
				},
				{
					tag: 'meta',
					attrs: {
						property: 'og:image:height',
						content: '630',
					},
				},
				{
					tag: 'meta',
					attrs: {
						name: 'twitter:image',
						content: 'https://packetthrower.github.io/zorite/og-image.png',
					},
				},
			],
			components: {
				// Shared packetThrower header: global bar (Apps menu, search,
				// theme, GitHub) + Zorite's app bar. The landing page at
				// /zorite/ is src/pages/index.astro, outside Starlight.
				Header: '@packetthrower/site-theme/components/StarlightHeader.astro',
			},
			editLink: {
				baseUrl: 'https://github.com/packetThrower/zorite/edit/main/docs/',
			},
			sidebar: [
				// Same groups, in the same order, on every packetThrower docs site.
				{
					label: 'Get started',
					items: [
						{ label: 'Install', slug: 'install' },
						{ label: 'Requirements', slug: 'reference/requirements' },
					],
				},
				{
					label: 'Guides',
					items: [
						{ label: 'Journal & pages', slug: 'usage/journal' },
						{ label: 'All pages & the graph', slug: 'usage/navigate' },
						{ label: 'Math & equations', slug: 'usage/math' },
						{ label: 'Whiteboards', slug: 'usage/whiteboards' },
						{ label: 'PDF & images', slug: 'usage/pdf' },
						{ label: 'Search', slug: 'usage/search' },
						{ label: 'Notebooks', slug: 'usage/notebooks' },
						{ label: 'Password & encryption', slug: 'usage/security' },
						{ label: 'Import & export', slug: 'usage/import' },
						{ label: 'Keyboard shortcuts', slug: 'usage/shortcuts' },
					],
				},
				{
					label: 'Customize',
					items: [
						{ label: 'Themes', slug: 'customize/themes' },
						{ label: 'Mouse cursors', slug: 'customize/cursors' },
						{ label: 'Language', slug: 'customize/language' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Bundled icons', slug: 'reference/icons' },
						{
							label: 'Crates',
							items: [
								{
									label: 'zorite-editor',
									items: [
										{ label: 'Overview', slug: 'reference/crates/zorite-editor' },
										{ label: 'API reference', slug: 'reference/crates/zorite-editor-api' },
									],
								},
								{
									label: 'zorite-markdown',
									items: [
										{ label: 'Overview', slug: 'reference/crates/zorite-markdown' },
										{ label: 'API reference', slug: 'reference/crates/zorite-markdown-api' },
									],
								},
								{
									label: 'gpui-pdf',
									items: [
										{ label: 'Overview', slug: 'reference/crates/gpui-pdf' },
										{ label: 'API reference', slug: 'reference/crates/gpui-pdf-api' },
									],
								},
								{
									label: 'gpui-whiteboard',
									items: [
										{ label: 'Overview', slug: 'reference/crates/gpui-whiteboard' },
										{ label: 'API reference', slug: 'reference/crates/gpui-whiteboard-api' },
									],
								},
								{
									label: 'os-cursors',
									items: [
										{ label: 'Overview', slug: 'reference/crates/os-cursors' },
										{ label: 'API reference', slug: 'reference/crates/os-cursors-api' },
									],
								},
								{
									label: 'os-spellcheck',
									items: [
										{ label: 'Overview', slug: 'reference/crates/os-spellcheck' },
										{ label: 'API reference', slug: 'reference/crates/os-spellcheck-api' },
									],
								},
								{
									label: 'ratex-gpui',
									items: [
										{ label: 'Overview', slug: 'reference/crates/ratex-gpui' },
										{ label: 'API reference', slug: 'reference/crates/ratex-gpui-api' },
									],
								},
							],
						},
					],
				},
				{
					label: 'Project',
					items: [
						{ label: 'Changelog', slug: 'changelog' },
						{
							label: 'Issues',
							link: 'https://github.com/packetThrower/zorite/issues',
							attrs: { target: '_blank', rel: 'noopener' },
						},
					],
				},
			],
			lastUpdated: true,
		}),
		// Explicit `@astrojs/sitemap` config — Starlight auto-pulls
		// the integration, but its default emits `<loc>`-only
		// entries. Adding it here lets us pass a `lastmod` so each
		// URL carries a freshness timestamp. Google Search Console
		// uses lastmod for crawl scheduling; without it, every
		// entry looks equally stale and the crawler is less
		// aggressive about re-indexing changed pages.
		//
		// `new Date()` evaluates at build time, so every page in
		// the sitemap gets the deployment timestamp. Per-page
		// per-file mtime would be more accurate but requires a
		// `serialize()` callback that walks git history per URL
		// — not worth the build-time cost for a docs site this
		// small, where everything tends to ship in the same
		// release anyway.
		sitemap({
			lastmod: new Date(),
		}),
	],
});
