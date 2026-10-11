// Checks the built site for accessibility problems: axe-core's WCAG 2.2 AA and best-practice
// rules on every page, in light and dark mode, on a wide and a 320px screen. At 320px it also
// fails pages that scroll sideways, which axe does not check (WCAG 1.4.10, Reflow).
//
// Run after building: npm run a11y. It needs Playwright's Chromium once:
// npx playwright install --only-shell chromium
import { createReadStream } from 'node:fs';
import { readdir, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { extname, join, relative, sep } from 'node:path';
import { chromium } from 'playwright';

const dist = new URL('../dist/', import.meta.url).pathname;
const axeSource = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
const tags = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'];
const variants = [
	{ colorScheme: 'light', viewport: { width: 1440, height: 900 } },
	{ colorScheme: 'dark', viewport: { width: 1440, height: 900 } },
	{ colorScheme: 'light', viewport: { width: 320, height: 640 } },
	{ colorScheme: 'dark', viewport: { width: 320, height: 640 } },
];
// Expressive Code gives a code block that scrolls sideways role="region" without a name, so it
// can be focused and scrolled from the keyboard. A region without a name is not a landmark,
// but axe's best-practice landmark-unique rule still reports several on one page.
const unnamedCodeRegions = (violation) =>
	violation.id === 'landmark-unique' &&
	violation.nodes.every((n) => n.target.every((t) => t.includes('pre[role="region"]')));
// Starlight's search button is named "Search" and shows that word, hidden from assistive
// technology, beside its keyboard shortcut. axe 4.14 counts the shortcut keys as visible label
// text missing from the name. Speaking "Search" still activates it, so the label is in the name.
const searchShortcut = (violation) =>
	violation.id === 'label-content-name-mismatch' &&
	violation.nodes.every((n) => n.target.every((t) => t.includes('button[data-open-modal')));

const types = {
	'.html': 'text/html',
	'.css': 'text/css',
	'.js': 'text/javascript',
	'.json': 'application/json',
	'.svg': 'image/svg+xml',
	'.woff2': 'font/woff2',
	'.wasm': 'application/wasm',
};

async function htmlFiles(dir) {
	const files = [];
	for (const entry of await readdir(dir, { withFileTypes: true, recursive: true })) {
		if (entry.isFile() && entry.name.endsWith('.html'))
			files.push(join(entry.parentPath, entry.name));
	}
	return files;
}

// The URL path each page is served at: /guides/converting/ for guides/converting/index.html.
const pathOf = (file) => {
	const rel = relative(dist, file).split(sep).join('/');
	return '/' + (rel.endsWith('index.html') ? rel.slice(0, -'index.html'.length) : rel);
};

function serve() {
	const server = createServer(async (req, res) => {
		let file = join(dist, decodeURIComponent(new URL(req.url, 'http://x').pathname));
		try {
			if ((await stat(file)).isDirectory()) file = join(file, 'index.html');
			await stat(file);
		} catch {
			res.writeHead(404).end();
			return;
		}
		res.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream' });
		createReadStream(file).pipe(res);
	});
	return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server)));
}

const pages = (await htmlFiles(dist)).map(pathOf).toSorted();
if (pages.length === 0) {
	console.error(`No pages in ${dist}; build the site first.`);
	process.exit(1);
}

const server = await serve();
const base = `http://127.0.0.1:${server.address().port}`;
let browser;
try {
	browser = await chromium.launch();
} catch (error) {
	console.error(
		`${error.message}\n\nInstall the browser with: npx playwright install --only-shell chromium`,
	);
	process.exit(1);
}

const problems = [];
await Promise.all(
	variants.map(async ({ colorScheme, viewport }) => {
		const label = `${colorScheme}, ${viewport.width}px`;
		const context = await browser.newContext({ colorScheme, viewport });
		const page = await context.newPage();
		// One page at a time per variant, so the variants share the work evenly.
		for (const path of pages) {
			// oxlint-disable-next-line no-await-in-loop
			await page.goto(base + path, { waitUntil: 'networkidle' });
			// Expressive Code makes a code block that scrolls focusable a moment after load; wait
			// for it, so the result does not depend on timing.
			// oxlint-disable-next-line no-await-in-loop
			await page.waitForFunction(() =>
				[...document.querySelectorAll('.expressive-code pre')].every(
					(pre) => pre.scrollWidth > pre.clientWidth === pre.hasAttribute('tabindex'),
				),
			);
			// oxlint-disable-next-line no-await-in-loop
			await page.addScriptTag({ path: axeSource });
			// oxlint-disable-next-line no-await-in-loop
			const violations = await page.evaluate(
				async (values) =>
					(await globalThis.axe.run(document, { runOnly: { type: 'tag', values } })).violations,
				tags,
			);
			for (const v of violations.filter(
				(found) => !unnamedCodeRegions(found) && !searchShortcut(found),
			)) {
				const nodes = v.nodes.map(
					(n) =>
						`      ${n.target.join(' ')}\n        ${n.failureSummary.split('\n').slice(1).join(' ').trim()}`,
				);
				problems.push(`${path} (${label}): ${v.id}, ${v.help}\n${nodes.slice(0, 5).join('\n')}`);
			}
			if (viewport.width === 320) {
				// oxlint-disable-next-line no-await-in-loop
				const overflow = await page.evaluate(
					() => document.documentElement.scrollWidth - document.documentElement.clientWidth,
				);
				if (overflow > 0)
					problems.push(`${path} (${label}): the page scrolls sideways by ${overflow}px`);
			}
		}
		await context.close();
	}),
);
await browser.close();
server.close();

if (problems.length > 0) {
	console.error(problems.toSorted().join('\n'));
	console.error(`\n${problems.length} accessibility problem(s).`);
	process.exit(1);
}
console.log(`${pages.length} pages, ${variants.length} variants each, no accessibility problems`);
