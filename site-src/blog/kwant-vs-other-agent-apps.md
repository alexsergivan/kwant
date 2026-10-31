title: kwant compared with other AI agent apps (placeholder)
description: A placeholder post that shows every part of a kwant blog post: headings, lists, code, a comparison table and related posts.
date: 2026-10-31
author: The kwant team
tags: comparison, self-hosted
card: Placeholder: how a kwant blog post looks
draft: yes
---
This post is a **placeholder**. It stays a draft, so it is never published; it exists to show and test how a blog post
renders. The real posts come from demand research and replace it.

## What a post can hold

Posts are written in Markdown in `site-src/blog/`. A post can have:

- Paragraphs with *emphasis*, **bold** and `inline code`.
- Links to the [docs]({{root}}docs/quickstart.html) or to [other sites](https://github.com/alexsergivan/kwant).
- Numbered steps, quotes, code blocks and tables.

### Steps

1. Write the post as `site-src/blog/<name>.md`.
2. Build with `python3 scripts/build_site.py --drafts` to preview it.
3. Remove `draft: yes` when it is ready.

> Claims in a comparison must match the products' own docs on the day the post is dated.

## A comparison table

```compare
caption: Placeholder rows, not a real comparison
| Feature | kwant | App A | App B |
| --- | --- | --- | --- |
| Self-hosted | yes | no | partial (paid plan) |
| Teams of bots | yes | yes | no |
| Price | Free | $20/month | $10/month |
```

## A plain table

| Command | What it does |
| --- | --- |
| `kwant` | Starts the app |
| `kwant --help` | Lists the options |

## Code

```
python3 scripts/build_site.py --drafts
```
