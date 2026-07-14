# About

mdsite is a demo program for **Fable**, a statically-typed,
garbage-collected scripting language. The generator is a few hundred
lines of Fable, split into three modules:

- `markdown.fable` — a line-based Markdown-to-HTML converter
- `site.fable` — the page model, the HTML template, and the nav builder
- `main.fable` — the driver: reads `content/`, writes `out/`, prints a report

## Supported Markdown

- headings (`#` through `######`)
- paragraphs, with adjacent lines joined
- **bold**, *italic*, and `inline code`
- [links](index.html), like this one back home
- unordered lists (you are reading one)
- fenced code blocks — see the [hello post](hello-fable.html)

## Escaping

HTML metacharacters are escaped everywhere, so text like 2 < 3, AT&T,
and <em>this fake tag</em> arrive as harmless literal characters — in
paragraphs, in `code spans like x < y`, and inside code blocks.
