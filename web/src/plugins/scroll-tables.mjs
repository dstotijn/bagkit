// Wraps every Markdown table in a container that scrolls sideways, so a wide table scrolls on
// its own on a narrow screen instead of making the whole page scroll. The container is
// focusable, so it can also be scrolled with the keyboard.
import { defineHastPlugin } from 'satteri';

export default defineHastPlugin({
	name: 'scroll-tables',
	element: {
		filter: ['table'],
		visit(node, ctx) {
			ctx.wrapNode(node, {
				type: 'element',
				tagName: 'div',
				properties: { className: ['table-scroll'], tabIndex: 0 },
				children: [],
			});
		},
	},
});
