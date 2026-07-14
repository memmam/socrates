# Welcome to mdsite

**mdsite** is a tiny static site generator written in the *Fable*
programming language. It reads Markdown from `content/`, converts it to
HTML, and writes a complete little site to `out/` — shared template,
navigation bar, and all.

This sample site has three pages:

- this front page
- an [about page](about.html) describing the generator
- a [blog post](hello-fable.html) with fenced code blocks

## How the site is built

Every page is wrapped in one shared template, and the navigation bar
above is generated from the page list itself — add a Markdown file to
`content/` and it shows up everywhere. The output is plain HTML with
inline CSS: no build steps, no dependencies, ready for any web server.

## Rebuild it

```
./target/release/fable demos/mdsite/main.fable
```

One command, and the whole site under `out/` is fresh again.
