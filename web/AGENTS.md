# Documentation website

This directory is bagkit's public documentation, an Astro Starlight site styled after Mintlify. Read the repository's root `AGENTS.md`, `README.md` and `GLOSSARY.md` first: the site documents the same behaviour and uses the same terms.

- Pages live in `src/content/docs`; add new ones to the sidebar in `astro.config.mjs`. Every page needs a `title` and a one-sentence `description`, which the page header shows as a subtitle.
- Document only behaviour you have checked against the code, `bagkit --help` or a run. Take example output from real runs, and state no timings: they depend on the machine (see the root `AGENTS.md`).
- Never edit between `<!-- generated: NAME -->` and `<!-- end generated: NAME -->` by hand. Those regions come from the code through `tests/docs.rs`; change the code's descriptions and run `mise run docs:generate`.
- Rust examples are files in the repository's `examples/` directory, imported with `?raw` and shown with Starlight's `Code` component. Add new examples there, not inline.
- The root `AGENTS.md` Documentation section and `scripts/docs_map.toml` say which pages describe which code; update the map when you add a page.
- Link to pages by absolute path, such as `/guides/library/`. The site is served under a base path on GitHub Pages (`/bagkit`), which `src/plugins/base-links.mjs` adds to Markdown links. Links in components, component props and frontmatter need `withBase` from `src/base.ts`; hero actions get it from `src/routeData.ts`. CI checks the links with that base, as `SITE_URL=https://dstotijn.github.io/bagkit mise run docs:build && python scripts/check_docs_links.py web/dist`.
- Abbreviations get a tooltip on their first use on each page from `src/plugins/abbreviations.mjs`. Add new abbreviations there rather than writing `<abbr>` by hand.
- Styling lives in `src/styles/theme.css`; `src/components` holds the page title override and the link cards.
- The Open Graph image, `public/og.png`, is rendered from `scripts/og.html` by `npm run og`; render it again after changing the wordmark, tagline, map or colours. Its meta tags appear once `SITE_URL` is set.
- The landing page's map, `public/netherlands.svg`, is drawn from a real address list by `scripts/landing_map.py` in the repository root; its docstring has the commands to regenerate it.
- Text is set in Inter with its single-storey a (`cv11`), from the official Inter 4.1 files in `public/fonts` (SIL Open Font License, `Inter-LICENSE.txt`). Fontsource's Inter builds leave out character variants, so don't switch back to them.
- Run `mise run docs:format` after editing, and `mise run docs:check` before committing. It checks formatting (oxfmt), lints (oxlint), types (`astro check`), builds the site failing on any warning (`scripts/build.mjs`; only the sitemap warning is allowed while `SITE_URL` is unset), checks links, runs the drift test, and checks accessibility (`scripts/a11y.mjs`: axe-core's WCAG 2.2 AA and best-practice rules on every page, in light and dark mode at desktop and 320px widths, and no sideways page scrolling at 320px). The accessibility check needs Playwright's Chromium once: `npx playwright install --only-shell chromium`.
- oxfmt formats Markdown, MDX, CSS, JSON and scripts, but not `.astro` files yet; keep those in the same style by hand. Markdown keeps spaces, everything else uses tabs (`.oxfmtrc.json`).
- Pin new dependencies to exact versions released at least two weeks earlier, and review install scripts: `allowScripts` in `package.json` records each decision. For layout changes, look at light and dark mode and a phone-width viewport.
- Use `astro dev --background` for a dev server, and manage it with `astro dev stop`, `astro dev status` and `astro dev logs`.
