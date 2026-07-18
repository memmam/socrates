# mdsite — a static site generator written in Socrates

A small, complete static site generator: it reads Markdown pages from
`content/`, converts them to HTML with a hand-written line-based
converter, wraps each page in a shared template whose navigation bar is
generated from the page list, writes the finished site to `out/`, and
prints a build report. About 500 lines of Socrates across four files.

## Run it

From the repository root:

```sh
./target/release/socrates demos/mdsite/main.soc   # build the site into out/
./target/release/socrates test demos/mdsite         # golden tests (report + converter)
```

(`main.soc` also works with `demos/mdsite/` as the working directory.)
Open `demos/mdsite/out/index.html` in a browser to see the result; the
generated pages are committed as sample output.

## Sample output

```
mdsite: building 3 pages  content/ -> out/

  source           title               words  bytes
  ------           -----               -----  -----
  index.md         Welcome to mdsite     140   2618
  about.md         About                 139   2774
  hello-socrates.md Hello, Socrates       178   2990

  wrote 3 pages, 8382 bytes of HTML
  3/3 pages byte-identical to the committed out/
```

The last line is a regeneration check: before overwriting each page the
builder reads the committed bytes back (`fs.read_bytes`, v0.7) and
compares them to the fresh render — `Bytes` equality is structural — so
the golden test pins that a fresh build reproduces the committed site
byte for byte, not just byte counts.

And an excerpt of the HTML it produces (`out/about.html`):

```html
<header><span class="brand">mdsite</span><nav><a href="index.html">Home</a> ...</nav></header>
<main>
<h1>About</h1>
<ul>
  <li><code>markdown.soc</code> — a line-based Markdown-to-HTML converter</li>
  ...
</ul>
<p>... text like 2 &lt; 3, AT&amp;T, and &lt;em&gt;this fake tag&lt;/em&gt; ...</p>
```

## The dialect of Markdown

- `#` … `######` headings, with inline markup
- paragraphs (adjacent lines join; blank lines separate)
- `**bold**`, `*italic*`, `` `code` ``, and `[text](url)` links —
  unmatched delimiters fall back to literal text
- unordered lists (`- item`)
- fenced code blocks with an optional language
  (` ```soc ` → `class="language-soc"`)
- HTML metacharacters are escaped everywhere; code spans and code
  blocks are opaque to further markup

## How it maps onto Socrates

| Generator concept | Socrates feature |
|---|---|
| filesystem walk & output | `fs.list_dir` / `fs.read` / `fs.write` / `fs.create_dir` |
| error plumbing | `Result[T, String]` + the `?` operator; one `match` at the bottom of `main.soc` handles every I/O failure |
| slugs and extensions | `std.path` (`ext`, `strip_ext`, `join`) |
| line splitting, word counts | `std.strings` (`lines`, `words`) |
| inline-markup scanning | a char-index cursor over the string itself: `index_of_from` / `slice` (before v0.6 this needed hand-rolled `matches_at`/`find_at` helpers over an exploded char list); missing delimiters propagate via `?` inside `parse_link` |
| HTML assembly | `strings.Builder` (v0.7) everywhere strings grow in a loop — escaping, the inline scanner, the block state machine, the page shell; O(n) where `+=` would be O(n²) (before v0.7: `List[String]` + `join`) |
| slug collisions | `std.set` (v0.7): `insert` reports whether the slug was new, so two sources can never silently claim one output file |
| output pinning | `fs.read_bytes` + structural `Bytes` equality (v0.7): each committed page is compared byte-for-byte against the fresh render |
| page ordering / nav | `sort_by` with a key function that pins `index` first |

Files:

- `markdown.soc` — escaping, the inline-span scanner, the block-level
  state machine (`to_html`), and `first_heading` for titles
- `site.soc` — the `Page` struct, nav builder, HTML shell, and CSS
- `main.soc` — the driver and build report; its full output is pinned
  by `//? expect:` directives
- `spec.soc` — 25 golden checks for the converter, including edge
  cases (unclosed delimiters, `#######`, unterminated fences, and the
  accumulator edges pinned across the v0.7 Builder refactor: the empty
  document, spans crossing joined paragraph lines, a code block whose
  first line is blank)
