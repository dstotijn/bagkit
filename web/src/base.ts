// The site's base path (see astro.config.mjs) is / on a domain of its own and /bagkit on
// GitHub Pages. Links in Markdown get it from src/plugins/base-links.mjs; links in components
// and frontmatter get it from here.
const base = import.meta.env.BASE_URL.replace(/\/$/, '');

/** Prefixes the base path to a link that starts at the root, such as /guides/library/. */
export function withBase(href: string): string {
	if (!href.startsWith('/') || href.startsWith('//') || href.startsWith(`${base}/`)) return href;
	return base + href;
}
