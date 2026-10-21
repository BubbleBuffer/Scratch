# Links, embeds and files

Links connect pages to each other, to web addresses and to attached files. They are written in the page's markdown, so `markdown` covers the syntax of the rest of the block.

## Page links

- `[[Title]]` links to a page. Write `[[Title|shown text]]` to show other text.
- `[[Title#Heading]]` links to the page `Title`. The part after `#` is kept in the shown text, but a link never points at a heading or a block inside a page.
- `[[<page id>]]` names the page by its id, which is exact.

A target is looked up as a page id first, then as a title ignoring case. If several pages share a title, a live page wins over one in the trash, and then the most recently updated one wins.

A target that matches no page stays as plain text `[[Name]]`. It is not a link. Find the page with `search_notes` and use its title or id.

When the app writes a page link, it uses the page's title. It uses the id instead when the title is empty, shared with another page, or contains `[`, `]`, `|`, `#`, `^` or a line break.

## External links

- `[text](https://example.com)` is a link to a web address.
- `[text](Other.md)` links to a page by its file name, the part before `#`. The name is matched against page titles, so `Notes/My%20Page.md` finds the page `My Page`.

## Links from outside the vault

- `notes://page/<page id>` opens that page in the Scratch client, and `notes://page/<page id>?block=<block id>` opens it at that block. The app's Copy link writes these.
- `http://<hub>/open/<page id>` and `http://<hub>/open/<page id>?block=<block id>` open the same places from a browser. The hub answers with a page that sends the browser to the `notes://` link, and shows an "Open in Scratch" link in case that does not happen. `<hub>` is the serving hub's `public_url`, or `127.0.0.1:<port>` when it has none. The standalone hub and the desktop app's embedded hub both serve this route.
- Agents use the http form, because the Claude app opens http links from chat but refuses custom schemes. Between pages in the vault, use `[[Title]]` or `[[<page id>]]` as above.
- Android App Links: with the hub's `android_package` and `android_sha256_certs` set, `https://notes.breadcat.cc/open/<page id>[?block=<block id>]` opens the Scratch app at that page instead of the browser. The app is registered for that host and path prefix as a verified link, and the hub serves the matching `/.well-known/assetlinks.json` ([hub config](../../wiki/architecture/hub.md#config)). Without a verified statement, Android opens the link in the browser, which shows the bounce page. The app's parser reads the https form as the same page link, from any host; only `notes.breadcat.cc` is verified by Android.
- An agent writes a link as `[Title](<link>)`, taking the link from the `link:` line of a `read_page`, `create_page` or `edit_page` result. A block's link is on its `block link:` line.

## Blocks that link

- `![[Title]]` alone on its line is a link-to-page block. It shows the page's title. If no page matches, the line stays as paragraph text.
- `![[Title]] <!-- database -->` shows a database inside the page. See `properties`.
- `<https://example.com> caption` is a bookmark. Only http, https and mailto addresses are bookmarks.
- An embed shows another page's title live. The app writes embeds; leave them as `read_page` shows them.

## Images and files

- An image is `![caption](../.vault/blobs/xx/id)`. A file is `[name](../.vault/blobs/xx/id)`.
- `xx` is the first two characters of the attachment id, and `id` is the whole id: 64 lowercase hex digits.
- A link resolves only when that attachment is already in the vault. If it is not, the link is kept as a link to the written path.
- An image's width is a block setting, `<!-- width=50 -->`, a percentage from 1 to 99. See `markdown`.

## Pages that link here

- `search_notes` with `backlinks_of` set to a page lists the pages that link to it. Trashed pages are left out.
- `read_page` with `include_backlinks` set adds the same list after the page.
- In a search, `links:[[Title]]` finds pages that link to a page, and `from:[[Title]]` finds pages it links to.

## Examples

A paragraph with a page link and a link with other text:

```
See [[Project plan]] and [[Project plan|the plan]] before Friday.
```

A link-to-page block, alone on its line:

```
![[Reading list]]
```

A bookmark with a caption:

```
<https://example.com/spec> the design spec
```

Pass the same text as `markdown` to `create_page` or `edit_page` with op `append`. Titles are matched when the tool runs, so check the title with `search_notes` first when the page may not exist yet.
