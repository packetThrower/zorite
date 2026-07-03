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
			logo: {
				src: './src/assets/icon.svg',
				replacesTitle: false,
			},
			favicon: '/favicon.svg',
			customCss: ['./src/styles/theme.css'],

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
				Hero: './src/components/Hero.astro',
				// Wraps Starlight's default SocialIcons to add a "Docs"
				// quick-access pill linking to /install/ — the most
				// common entry point for visitors landing on a deep
				// page who want to start over.
				SocialIcons: './src/components/SocialIcons.astro',
			},
			social: [
				{
					icon: 'github',
					label: 'GitHub',
					href: 'https://github.com/packetThrower/zorite',
				},
			],
			editLink: {
				baseUrl: 'https://github.com/packetThrower/zorite/edit/main/docs/',
			},
			sidebar: [
				{ label: 'Install', slug: 'install' },
				{
					label: 'Usage',
					items: [
						{ label: 'Journal & pages', slug: 'usage/journal' },
						{ label: 'All pages & the graph', slug: 'usage/navigate' },
						{ label: 'Math & equations', slug: 'usage/math' },
						{ label: 'Whiteboards', slug: 'usage/whiteboards' },
						{ label: 'PDF & images', slug: 'usage/pdf' },
						{ label: 'Search', slug: 'usage/search' },
						{ label: 'Password & encryption', slug: 'usage/security' },
						{ label: 'Import from Logseq', slug: 'usage/import' },
						{ label: 'Keyboard shortcuts', slug: 'usage/shortcuts' },
					],
				},
				{
					label: 'Customize',
					items: [{ label: 'Themes', slug: 'customize/themes' }],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Requirements', slug: 'reference/requirements' },
						{ label: 'Bundled icons', slug: 'reference/icons' },
						{
							label: 'Crates',
							items: [
								{ label: 'gpui-editor', slug: 'reference/crates/gpui-editor' },
								{ label: 'gpui-markdown', slug: 'reference/crates/gpui-markdown' },
								{ label: 'gpui-pdf', slug: 'reference/crates/gpui-pdf' },
								{ label: 'gpui-whiteboard', slug: 'reference/crates/gpui-whiteboard' },
								{ label: 'os-spellcheck', slug: 'reference/crates/os-spellcheck' },
								{ label: 'ratex-gpui', slug: 'reference/crates/ratex-gpui' },
							],
						},
					],
				},
				{ label: 'Changelog', slug: 'changelog' },
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
