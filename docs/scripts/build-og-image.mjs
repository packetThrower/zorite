// Generates docs/public/og-image.png at 1200x630 — the standard
// Open Graph card size. The image is what link-preview unfurlers
// (Slack, Discord, iMessage, Twitter, LinkedIn, etc.) display when
// a Zorite docs page is shared.
//
// Source-of-truth is the inline SVG below. Sharp rasterizes it to
// PNG. Run with `node scripts/build-og-image.mjs` whenever the
// branding or tagline changes; the resulting PNG is committed so
// CI doesn't have to regenerate. Sharp is already a dev dep
// (transitive via @astrojs/starlight) so no extra install.
//
// Design follows the rest of the brand family (packetthrower.github.io
// landing page, PortFinder docs hero):
//   - Navy radial gradient ground (#2e4368 → #162237 → #04080e)
//   - Gold glow at the lower-left, drifting up
//   - Zorite app icon, drop-shadowed
//   - "Zorite" set in a serif (Fraunces if available, Georgia
//     fallback). System fonts are intentional so the output is
//     reproducible without bundling font files.
//   - Tagline below in IBM Plex Sans / system sans

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const here = dirname(fileURLToPath(import.meta.url));
const iconPath = resolve(here, '../src/assets/icon.svg');
const outPath = resolve(here, '../public/og-image.png');

// Inline the icon SVG so we can place it inside our compositing
// SVG without dealing with sharp's two-step composite pipeline.
const iconRaw = readFileSync(iconPath, 'utf8');
// Strip the outer <?xml ?> declaration if present — embedding a
// nested xml decl breaks the parent SVG.
const iconInner = iconRaw
	.replace(/<\?xml[^?]*\?>/, '')
	.replace(/<!DOCTYPE[^>]*>/, '')
	.trim();

const W = 1200;
const H = 630;

const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">
	<defs>
		<radialGradient id="bg" cx="50%" cy="0%" r="100%">
			<stop offset="0%" stop-color="#2e4368"/>
			<stop offset="45%" stop-color="#162237"/>
			<stop offset="100%" stop-color="#04080e"/>
		</radialGradient>
		<radialGradient id="glow" cx="50%" cy="50%" r="50%">
			<stop offset="0%" stop-color="#d49b3a" stop-opacity="0.55"/>
			<stop offset="100%" stop-color="#d49b3a" stop-opacity="0"/>
		</radialGradient>
		<filter id="iconShadow" x="-20%" y="-20%" width="140%" height="140%">
			<feGaussianBlur in="SourceAlpha" stdDeviation="14"/>
			<feOffset dx="0" dy="18" result="off"/>
			<feComponentTransfer><feFuncA type="linear" slope="0.5"/></feComponentTransfer>
			<feMerge><feMergeNode/><feMergeNode in="SourceGraphic"/></feMerge>
		</filter>
	</defs>

	<rect width="${W}" height="${H}" fill="url(#bg)"/>
	<ellipse cx="${W * 0.78}" cy="${H * 1.1}" rx="${W * 0.55}" ry="${H * 0.8}" fill="url(#glow)"/>

	<!-- Icon, centered vertically in the left margin. The icon's
	     own viewBox is 0 0 1024 1024, so we wrap it in a fresh <svg>
	     that re-sizes to 240x240 while preserving that viewBox. -->
	<svg x="120" y="${H / 2 - 120}" width="240" height="240" viewBox="0 0 1024 1024" filter="url(#iconShadow)">
		${iconInner.replace(/<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '')}
	</svg>

	<!-- Wordmark + tagline, right of the icon -->
	<g font-family="'Fraunces', Georgia, 'Times New Roman', serif" fill="#fafbfd">
		<text x="420" y="270" font-size="120" font-weight="600" letter-spacing="-3">Zorite</text>
	</g>
	<g font-family="'IBM Plex Sans', system-ui, -apple-system, sans-serif" fill="#d3d8e1">
		<text x="420" y="335" font-size="32" font-weight="400">A local-first daily journal for the desktop.</text>
		<text x="420" y="385" font-size="26" font-weight="400" fill="#aab1c0">Linked notes. PDFs. Whiteboards. macOS · Windows · Linux.</text>
	</g>

	<!-- Footer URL, bottom-left -->
	<g font-family="'IBM Plex Mono', ui-monospace, monospace" fill="#6e7689">
		<text x="120" y="${H - 50}" font-size="22" letter-spacing="1">packetthrower.github.io/zorite</text>
	</g>
</svg>`;

await sharp(Buffer.from(svg))
	.resize(W, H)
	.png({ compressionLevel: 9 })
	.toFile(outPath);

console.log(`og-image: wrote ${outPath} (${W}×${H})`);
