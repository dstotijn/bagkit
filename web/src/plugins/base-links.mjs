// Prefixes the site's base path to links that start at the root, such as /guides/library/,
// so pages can link by absolute path whether the site is served from the root of a domain or
// from a subpath like https://dstotijn.github.io/bagkit/.
import { defineHastPlugin } from 'satteri';

export default function baseLinks(base) {
	const prefix = base.replace(/\/$/, '');
	return defineHastPlugin({
		name: 'base-links',
		element: {
			filter: ['a'],
			visit(node, ctx) {
				const href = node.properties?.href;
				if (!prefix || typeof href !== 'string' || !href.startsWith('/') || href.startsWith('//'))
					return;
				ctx.replaceNode(node, { ...node, properties: { ...node.properties, href: prefix + href } });
			},
		},
	});
}
