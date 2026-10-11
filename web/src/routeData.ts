// Starlight route middleware: adds the base path to hero action links in frontmatter, which
// Starlight passes through unchanged.
import { defineRouteMiddleware } from '@astrojs/starlight/route-data';
import { withBase } from './base';

export const onRequest = defineRouteMiddleware((context) => {
	for (const action of context.locals.starlightRoute.entry.data.hero?.actions ?? []) {
		action.link = withBase(action.link);
	}
});
