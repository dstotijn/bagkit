// Renders the Open Graph image, scripts/og.html, to public/og.png at 1200 by 630. Run it after
// changing the wordmark, the tagline, the map or the colours: npm run og. It needs Playwright's
// Chromium, as the accessibility check does.
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const page = new URL('og.html', import.meta.url);
const target = fileURLToPath(new URL('../public/og.png', import.meta.url));

// The template loads fonts and images from public/ beside it, which file pages may only do with
// this flag.
const browser = await chromium.launch({ args: ['--allow-file-access-from-files'] });
const tab = await browser.newPage({ viewport: { width: 1200, height: 630 } });
await tab.goto(page.href);
await tab.evaluate(() => document.fonts.ready);
// Let the map's packets travel a moment, so the image catches them mid-route.
await tab.waitForTimeout(2500);
await tab.screenshot({ path: target });
await browser.close();
console.log(`Wrote ${target}`);
