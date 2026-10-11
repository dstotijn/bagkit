# bagkit documentation

The public documentation website for bagkit, built with [Astro Starlight](https://starlight.astro.build) and styled after Mintlify.

```sh
mise run docs:dev     # serve on http://localhost:4321 with live reload
mise run docs:build   # install exact dependencies and build into web/dist
```

Pages are Markdown and MDX in `src/content/docs`; the sidebar is defined in `astro.config.mjs`. Set `SITE_URL` when building for deployment so the sitemap and canonical links use the public URL. Its path becomes the base the site is served under, such as `/bagkit` on GitHub Pages, where `.github/workflows/pages.yml` deploys it from `main`.
