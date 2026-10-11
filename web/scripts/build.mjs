// Builds the site and fails on any warning. The sitemap warning is expected while SITE_URL
// is unset, before the site has a public address.
import { spawn } from 'node:child_process';

const build = spawn('astro', ['build'], { shell: true, stdio: ['inherit', 'pipe', 'pipe'] });
let output = '';
for (const stream of [build.stdout, build.stderr]) {
	stream.on('data', (chunk) => {
		output += chunk;
		process.stdout.write(chunk);
	});
}
build.on('close', (code) => {
	const warnings = output
		.split('\n')
		.filter((line) => line.includes('[WARN]'))
		.filter((line) => process.env.SITE_URL || !line.includes('[@astrojs/sitemap]'));
	if (code !== 0) process.exit(code ?? 1);
	if (warnings.length > 0) {
		console.error(`\nThe build reported ${warnings.length} warning(s):\n${warnings.join('\n')}`);
		process.exit(1);
	}
});
