// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { satteri } from '@astrojs/markdown-satteri';
import codeTheme from './src/code-theme.mjs';
import abbreviations from './src/plugins/abbreviations.mjs';
import baseLinks from './src/plugins/base-links.mjs';
import scrollTables from './src/plugins/scroll-tables.mjs';

// The public URL, for the sitemap and canonical links. Its path is the base every page and
// asset is served under: / on a domain of its own, /bagkit on GitHub Pages
// (https://dstotijn.github.io/bagkit). Pages link by absolute path; base-links adds the base.
const site = process.env.SITE_URL;
const base = site ? new URL(site).pathname : '/';

// https://astro.build/config
export default defineConfig({
	site,
	base,
	// The library guide shows the repository's examples, outside this directory.
	vite: { server: { fs: { allow: ['..'] } } },
	markdown: { processor: satteri({ hastPlugins: [baseLinks(base), scrollTables, abbreviations] }) },
	integrations: [
		starlight({
			title: 'bagkit',
			description:
				'Convert the Dutch BAG (addresses and buildings) to SQLite, CSV or Parquet, and look addresses up.',
			// The mark only; the title beside it is text, so it is set in Inter like the rest.
			logo: { src: './src/assets/logo.svg', alt: '' },
			favicon: '/favicon.svg',
			// The Open Graph image (public/og.png, rendered by scripts/og.mjs). Link previews need an
			// absolute URL, so it is only announced once SITE_URL gives the site its address.
			head: site
				? [
						['og:image', new URL('og.png', site.replace(/\/?$/, '/')).href],
						['og:image:width', '1200'],
						['og:image:height', '630'],
						[
							'og:image:alt',
							'bagkit: a toolkit for the Dutch register of addresses and buildings (BAG)',
						],
					].map(([property, content]) => ({ tag: 'meta', attrs: { property, content } }))
				: [],
			// src/content/docs/404.md is the not-found page.
			disable404Route: true,
			// Adds the base path to hero action links (src/routeData.ts).
			routeMiddleware: './src/routeData.ts',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/dstotijn/bagkit' }],
			editLink: {
				baseUrl: 'https://github.com/dstotijn/bagkit/edit/main/web/',
			},
			lastUpdated: true,
			customCss: ['./src/styles/theme.css', './src/styles/prototype.css'],
			components: {
				PageTitle: './src/components/PageTitle.astro',
				Footer: './src/components/Footer.astro',
				MobileMenuToggle: './src/components/MobileMenuToggle.astro',
				ThemeSelect: './src/components/ThemeSelect.astro',
			},
			expressiveCode: {
				// One dark theme in the landing page's palette, in both modes (src/code-theme.mjs).
				themes: [codeTheme],
				// Plain blocks with wrapped lines, rather than terminal windows.
				defaultProps: {
					wrap: true,
					overridesByLang: { 'sh,bash,shell': { frame: 'code' } },
				},
				styleOverrides: {
					borderRadius: '0',
					borderColor: 'hsl(0, 0%, 20%)',
					codeFontFamily: "'Paper Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
					frames: {
						shadowColor: 'transparent',
						editorTabBarBackground: 'hsl(0, 0%, 9%)',
						editorTabBarBorderBottomColor: 'hsl(0, 0%, 20%)',
						editorActiveTabBackground: 'hsl(0, 0%, 6%)',
						editorActiveTabForeground: 'hsl(0, 0%, 75%)',
						editorActiveTabIndicatorTopColor: 'transparent',
						editorActiveTabIndicatorBottomColor: 'transparent',
						terminalTitlebarBackground: 'hsl(0, 0%, 9%)',
						terminalTitlebarForeground: 'hsl(0, 0%, 75%)',
						terminalTitlebarBorderBottomColor: 'hsl(0, 0%, 20%)',
						terminalBackground: 'hsl(0, 0%, 6%)',
						inlineButtonForeground: 'hsl(0, 0%, 75%)',
						inlineButtonBorder: 'hsl(0, 0%, 30%)',
						tooltipSuccessBackground: 'hsl(224, 100%, 50%)',
					},
				},
			},
			sidebar: [
				{
					label: 'Get started',
					items: [
						{ label: 'Introduction', slug: 'introduction' },
						{ label: 'Quickstart', slug: 'quickstart' },
						{ label: 'Installation', slug: 'installation' },
					],
				},
				{
					label: 'Guides',
					items: [
						{ label: 'Converting extracts', slug: 'guides/converting' },
						{ label: 'Address lists', slug: 'guides/address-lists' },
						{ label: 'Looking up addresses', slug: 'guides/lookups' },
						{ label: 'Using GIS tools', slug: 'guides/gis' },
						{ label: 'Keeping data current', slug: 'guides/updating' },
						{ label: 'Using the Rust library', slug: 'guides/library' },
					],
				},
				{
					label: 'Concepts',
					items: [
						{ label: 'The BAG', slug: 'concepts/bag' },
						{ label: 'Peildatum', slug: 'concepts/reference-dates' },
						{ label: 'Import rules', slug: 'concepts/import-rules' },
						{ label: 'How bagkit works', slug: 'concepts/architecture' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Command line', slug: 'reference/cli' },
						{ label: 'Tables', slug: 'reference/tables' },
						{ label: 'Address list', slug: 'reference/address-list' },
						{ label: 'HTTP API', slug: 'reference/http' },
						{ label: 'Metadata', slug: 'reference/metadata' },
					],
				},
			],
		}),
	],
});
