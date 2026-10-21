# Markdown and blocks

A Scratch page is a list of blocks. Tools take block content as markdown: `create_page` (`markdown`), `edit_page` (`markdown` for the `append`, `replace` and `set_body` ops) and `upsert_row` (`markdown`). `read_page` returns the same syntax. Blank lines separate blocks.

Write only what the page needs. Plain paragraphs and headings need no help. Check the other topics before writing links, properties, day notes or tasks. Call `notes_help` with a topic id to open one.

## Block types

Each built-in block type and how it is written. Kinds the app writes itself are marked as such.

<!-- check: block-types -->
- `paragraph`: plain text with inline marks. An empty paragraph is `<br>`.
- `heading1`: `# Text`.
- `heading2`: `## Text`.
- `heading3`: `### Text`. Four to six `#` also read as heading3.
- `bulleted`: `- Text`. `*` and `+` also read.
- `numbered`: `1. Text`. `1)` also reads. Numbers are recomputed on output.
- `todo`: `- [ ] Text`, or `- [x] Text`. `[X]` also reads as done.
- `quote`: `> Text`, one `>` per line. Consecutive `>` lines form one block.
- `code`: a fenced block. The word after the opening fence is the language and is kept.
- `table`: a GFM pipe table: a header row, a delimiter row, then the rows.
- `divider`: `---`. `***` and `___` also read.
- `linktopage`: `![[Title]]` alone on its line. See `links`.
- `image`: `![caption](../.vault/blobs/xx/id)`. See `links`.
- `file`: `[name](../.vault/blobs/xx/id)`. See `links`.
- `bookmark`: `<https://example.com> caption`. Only http, https and mailto addresses.
- `database`: `![[Title]] <!-- database -->`. See `properties`.
- `toggle`: `- Title <!-- toggle -->`, with children indented under it.
- `toggleheading1`: `# Title <!-- toggle -->`.
- `toggleheading2`: `## Title <!-- toggle -->`.
- `toggleheading3`: `### Title <!-- toggle -->`.
- `callout`: `> Text <!-- callout icon=💡 -->`. The icon is one token with no spaces.
- `columnlist`: `<!-- columns -->` alone on its line. Its children are columns.
- `column`: `<!-- column -->` alone on its line, with its blocks indented under it.
- `toc`: `<!-- toc -->` alone on its line. Shows the page's headings.
- `chat_ref`: a reference to an agent chat. App-written: leave it as `read_page` shows it.
- `query`: a fence labelled `query`. A saved search card with no children; its text is a search spec. Leave it alone unless the user asks for one.
- `embed`: a live link to another page. App-written: leave it as `read_page` shows it.
<!-- /check -->

## Children and layout

- Indent children two spaces under a bullet, to-do, toggle or heading.
- Under a numbered item, indent by the width of its number and dot: `1. ` is three columns.
- A blank line separates blocks, except between consecutive list items (bulleted, numbered, to-do).
- A line with no marker, directly under a paragraph or list item with no blank line, continues that block on a new line.
- Text that would read as markup is escaped when written: `\#`, `\-`, `\*`, `\_`. A backslash before any punctuation mark makes it literal.

Code fences use three backticks. Use tildes (`~~~~`) instead when the code itself holds a three-backtick line.

Children are indented, for example:

```
- Groceries
  - Milk
  - Eggs
```

## Inline marks

- Bold `**text**`, italic `*text*` or `_text_`, strike `~~text~~`, code with single backticks.
- Underline `<u>text</u>`.
- Text colour `<span data-color="red">text</span>`.
- Date mention `<time datetime="2026-10-15">2026-10-15</time>`. A timed mention uses `2026-10-15T14:30`, read as UTC.
- Links and page links: see `links`.

Plain text such as `2026-10-15` is not a date. Only a `time` element makes one.

## Block colour and width

- Colour: `<!-- color=red -->` at the end of the block's first line. Names are plain colour names, such as red. A background colour name ends in the word background.
- Width of an image: `<!-- width=50 -->`, a percentage from 1 to 99.

## Where tools write it

`read_page` prints blocks in this syntax. With `block_ids` set, each top-level block is preceded by its id. Pass those ids to `edit_page` as `block_ids` (for `replace` and `delete`). For `append`, pass a block id as `after_block` to insert after it.
