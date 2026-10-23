---
type: topic
layer: state
summary: "Note sharing: the hub's signed-in web viewer at /open, share links at /s/<token> (public or private, snapshot or live) with preview cards, and the app's Share popup."
parent: "[[integrations/index]]"
status: current
covers: ["crates/hub/src/web.rs", "crates/hub/src/render.rs", "crates/hub/src/html.rs", "app/lib/features/share/**"]
---

# Sharing

Built 2026-10-23 on branch `lane/note-sharing`. The hub renders pages as small, self-contained HTML. It does this in two places:

- the **web viewer** at `/open/<id>`, for people signed in to the hub;
- **share links** at `/s/<token>`, made from the app or the viewer.

Routes, config and security for the hub as a whole are in [Hub server](../architecture/hub.md).

## Rendering

[render.rs](../../crates/hub/src/render.rs) renders a page in steps:

- **Body.** The body goes through the same markdown export as the MCP `read_page` (`Vault::blocks_markdown`), then through pulldown-cmark, with tables, task lists, strikethrough and wikilinks on. The MCP output is unchanged.
- **Raw HTML.** Raw HTML in a note is shown as text, and the export's `<!-- ... -->` metadata is dropped.
- **Header.** Properties go in a table in the header. Keys starting with `$` are skipped. Values use the parent database's schema, so select options show their names.
- **Inline databases.** An inline database block becomes a plain table: Name plus the schema's columns, at most 200 rows, with "N of M rows" when cut. Rows come from `Vault::query`. A link-to-page block becomes "↳ Title".
- **Images and files.** These point at the route that serves the page, `/open/<id>/a/<attachment>` or `/s/<token>/a/<attachment>`. Each request passes the same access check as the page, and only attachments the page itself uses (its image and file blocks, plus its cover) are served. Attachments are sent with `X-Content-Type-Options: nosniff` and a sandboxing CSP. Anything that isn't an image is sent as a download.
- **Links.** A link to another page is a real link only in the signed-in viewer, and only when the target page exists and isn't in the trash. Otherwise it's plain text, as it always is on share pages. External links are kept only for `http`, `https` and `mailto`, and get `rel="noopener noreferrer nofollow"`.
- **Look.** The page's CSS is inline and uses the app's palette and fonts, light and dark through `prefers-color-scheme`. Images are capped at the page width and 70% of the window height. Pages load nothing from outside.

## Web viewer and sign-in

`GET /open/<id>`:

- **With a hub web session:** the rendered page, with "Open in app", "Share" and "Sign out" buttons.
- **Without one:** the bounce page, which still sends the browser to `notes://page/<id>` straight away. It reads "Open this note", with an "Open in Scratch" button and, when OIDC is configured, a "View in browser" button that goes through sign-in. The app's deep links work as before.

The sign-in is `GET /auth/web?return=<path>`. It uses the same OIDC provider, PKCE and callback as the app login (`<public_url>/auth/callback`), so the provider needs no change. `return` must be `/open/<page id>` or `/s/<token>`.

- **Allowed accounts** (`allowed_users` or `allowed_groups`) get the session cookie `scratch_session`, with `HttpOnly; Secure; SameSite=Lax; Path=/`, valid for 30 days. Its value is 256 random bits. The hub keeps only the value's SHA-256 in `.vault/hub/web-sessions.json`, with the account and the expiry, and keeps at most 1,000 sessions; the oldest are dropped first.
- **Any other account** gets a 403 page and no cookie.
- **Sign out** is `POST /auth/web/logout`. It deletes the session, clears the cookie, and goes back to the page.

## Share links

A share is stored in `.vault/hub/shares.json` with these fields:

- `id`: 64 random bits, used to revoke it;
- `token`: 192 random bits, hex, in the URL `https://<hub>/s/<token>`;
- the page;
- `visibility`: `public` or `private`;
- `kind`: `snapshot` or `live`;
- `created` and `created_by` (`device:<id>` or the web account);
- `revoked`, set once it is turned off.

Who can open a link and what it shows:

| | Shows |
|---|---|
| Public | Anyone with the link. |
| Private | The page for a hub web session. Everyone else, crawlers included, gets a "Private note · Scratch" card with a sign-in button, and nothing from the page. Its attachments return 401. |
| This version (snapshot) | The HTML and the page's attachments are written to `.vault/hub/shares/<id>/` at creation and served from there. Later edits don't change it. |
| Live link | Rendered on every request. A page that was deleted or moved to the trash gives 404. |
| Revoked | 410, "Link turned off". The page and its attachments both stop. Snapshot files are kept. |

Every share page carries `<meta name="robots" content="noindex">` and `X-Robots-Tag: noindex`, has no script, and doesn't include subpages. A token opens only its own page and that page's attachments. Databases can't be shared (400 `databases_not_shareable`).

**Preview cards.** Public links (and the signed-in viewer) carry Open Graph and Twitter tags: the title; a description made from the first ~200 characters of text, cut at a word; the first image, or else the app icon at `/icon.png`; and the site name Scratch. Private links carry only `og:title` "Private note", the icon and the site name.

## Share API

| Route | |
|---|---|
| `GET /shares?page=<id>` | The page's active links, newest first: `{shares: [{id, url, page, visibility, kind, created, created_by}]}` |
| `POST /shares` `{page, visibility, kind}` | 201 with the new share |
| `DELETE /shares/<id>` | 204, or 404 `no_share` |

Callers authenticate in one of two ways:

- a device token with `X-Device`, as for `/sync`;
- a web session cookie, which also requires an `Origin` header equal to the hub's own origin (403 otherwise).

Agent tokens and OAuth JWTs are refused.

## In the app

The page menu has **Share** when the app is signed in to a hub, on desktop and phone. It isn't shown for databases. The popup ([share_popup.dart](../../app/lib/features/share/share_popup.dart), client in [share_client.dart](../../app/lib/features/share/share_client.dart)) has these parts:

- Public / Private, default Private;
- This version / Live link, default This version;
- **Create link**, which copies the new URL and shows it;
- the page's links, each with its kind and age, Copy and Revoke ("No links yet." when there are none).

The two choices are labelled "Who can view" and "What to share", and only the selected option's explanation shows. A copy is confirmed in place: the button reads "Copied" for two seconds. Revoke asks in the row first ("Revoke this link? It stops working for everyone.", Cancel or Revoke).

The viewer's Share button opens the same controls, with the same behaviour, in a small HTML dialog, the only script on the viewer.

## Tests

[tests/share.rs](../../crates/hub/tests/share.rs) runs against the mock OIDC issuer of `login.rs` and covers:

- rendering: headings, a list, todos, properties, a database table, an image, page links, raw HTML as text;
- web sign-in: the redirect URI, cookie flags, a hashed store, a refused account, logout;
- `/open` with and without a session;
- public, private, snapshot, live, revoke and trash;
- tokens reaching only their own page's attachments;
- Origin checks and refused agent tokens;
- the preview tags.

Setting `SHARE_SHOTS_DIR` makes `write_screenshot_pages` write the sample pages used for the screenshots. Widget tests are in `app/test/share_popup_test.dart` and `app/test/share_menu_test.dart`.

## Limits

- Out of scope: sharing subpages or whole databases, comments or editing on the web, search indexing, per-person access lists, expiry dates.
- The viewer and shares need `public_url` to be the hub's real `https://` address. Session cookies are `Secure`, and card URLs are built from it.
- Shares and sessions live only on the hub. They aren't synced to devices.
