# Changelog

Product updates surfaced in-app (the sidebar "What's new" bell).

Add an update by dropping a new `.mdx` file here. One file per release. They're
compiled at build time — no markdown runtime ships to the client — and ordered
by `date` (newest first).

## Frontmatter (required)

```mdx
---
id: 2026-07-01-my-update      # stable, unique, never reuse; date-prefixed slug
title: A short headline
date: 2026-07-01              # YYYY-MM-DD, drives ordering + the seen watermark
badge: new                   # optional: new | improvement | fix
summary: One line shown in the compact list.
editions: [enterprise]       # optional: edition ids (oss | enterprise); absent = every edition
---
```

Mark an entry `editions: [enterprise]` when it is only about features of the Enterprise
edition: the other editions never list it.

## Body

Everything under the frontmatter is the expanded view (opened from the list).
Use `##` headings for sub-sections.

For images, `import` them so Vite hashes and bundles them as assets (a plain
`![](./x.png)` markdown src won't resolve in the production build). Keep the
file next to the entry:

```mdx
import shot from './my-screenshot.png'

## A bigger feature

<img src={shot} alt="Alt text" />

Some longer explanation...
```

Keep the summary punchy; put the detail (and any images) in the body.
