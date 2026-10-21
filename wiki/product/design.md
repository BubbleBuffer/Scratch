---
type: topic
layer: state
summary: "The built design: the Stacks look (a warm desk of paper sheets, space colours and Calico tabs), the Lucide icon set and AppTooltip rule, and the app icon."
parent: "[[product/index]]"
status: current
---

# Design

## Stacks (approved 2026-10-07)

**The idea.** The workspace is a desk; pages are sheets of paper laid on it. Home sits on the desk; an open page is a sheet on top, and on desktop the sheets underneath show their spines, so going back is picking up the sheet below.

**Identity**, from the [prototype](../../design/directions/c-stacks/index.html):
- **Colour.** A warm desk (#ECE6DB) under cream sheets (#FFFAF1), ink #282A2F text, and a dark duvet rail (#2F3138) for navigation in both themes. Apricot #DE945F is the one accent: the main action and today. Status colours come from the family: moss/ok, sky/info, sun/attention, paw/danger. Space colours stay small: a dot, a ring, a bar.
- **Type.** Bricolage Grotesque for display (greeting, page titles, headings, section titles), Atkinson Hyperlegible Next for interface and body text, Fragment Mono for times and code. All three are bundled (SIL Open Font License).
- **Shape.** Fitted, slightly uneven corners (controls 7/11/6/9 px, fields 6/10/6/8, panels 12/18/10/16), pills only for statuses. Cards that hold a stack show paper edges under them.
- **Depth.** Soft seam shadows under cards, a longer stack shadow on sheets.
- **Motion.** Quick and springy: hover lifts by 1–2 px, presses sink by 1 px, ticked tasks settle and slide away, and sheets slide in over the desk. Reduced motion makes all of it instant.

**Phases.**
1. Tokens, fonts and components; the rail; the desk; Home in the Stacks layout; pages on a sheet. *Built 2026-10-07.* Deviations: Today status chips await a projection that exposes status values; the exact approved light tertiary token does not meet body-text AA. Existing navigation controls and real event ranges remain. Verification and visual differences.
2. Stack navigation with spines on desktop, sheet transitions, the page header, database views and a dark-mode pass. *Built 2026-10-07*, including heading progress and Today status projection/chips. Deviations: bookmark, history, import and palette flows with no page-opening action stay unchanged; legacy mobile database pushes are preserved. Chosen-colour foregrounds were adjusted for AA, and concurrent agenda updates now use transactions.
3. **Calico tabs and fun** (*Built 2026-10-07*). The user missed Patchwork's colour and playfulness but found Patchwork too messy for notes. They compared three strengths in the [Stacks Calico trial](../../design/directions/d-stacks-calico/index.html) and chose **Tabs with Fun on**:
   - **Tabs:** space colour sits on edges and tiles:
     - filled spines on covered sheets;
     - a 6 px band along the top of each page;
     - filled space tiles in the rail;
     - folder tabs on Home cards;
     - to-dos ticked in the space's colour;
     - coloured underlines on page mentions.
   - **Fun:** the calico mark, the sleeping cat on empty pages, confetti when a checklist is finished, and a small wiggle on space tiles.
   - **Tidiness rules:**
     - one colour per space;
     - at most one large colour field per screen;
     - no colour behind body text;
     - no dashed stitches;
     - same-size shapes in a row.
   - **Not chosen:** the larger "Patches" level, with cover bands and filled space cards on Home.
   - **Built choices:** moss spine titles keep their original backing for hover contrast; rail duvet uses the visible light patch in both themes. Home keeps its greeting. Icon-less titles retain their existing layout, the empty cat follows the editable first row, and its guidance wraps on narrow phones. Default callouts use neutral paper with a small space seam; authored block formatting remains available. All acceptance checks pass, including regenerated goldens and table p95 **19.3 ms**. Checks, inspected goldens and prototype differences.

**Fixed-width pile (decided 2026-10-07, after the UX review (internal wiki)).** Pages opened from a database row were squeezed by the side peek and by a spine per sheet. Now everything opened lands on top at full width:
- Depth 1: no spines; the sheet sits 12 px from the rail and the top.
- Depth 2 and more: only the sheet directly below the top shows its 58 px spine. Older sheets show as up to three 4 px paper edges left of it. The top sheet's left edge is 12 + 58 + 4 × min(depth − 2, 3) from the rail; all sheets are top-aligned at 12 px. At most four pages sit above Home.
- Clicking the spine goes back one sheet. Resting the pointer on the spine or edges for 300 ms, clicking the edges, or Enter on a focused spine opens a list of every open page plus Home; picking one goes back to it. Esc or a click outside closes the list. Nothing reflows while it is open.
- The slide-in motion stays; reduced motion makes it instant.
- **Peek removed on desktop.** Rows open as sheets on top of their database, whose name leads the row's breadcrumb. The side and center peek, their prev/next buttons and the desktop "Open pages in" setting are gone. The setting is kept in the view config and still shows on mobile.
- **Related tidy-ups:** a new empty page shows one empty state (starter chips Blank, Database, each template and + Template under the title, the cat below). Backlinks are a "Linked from" section at the end of the page, hidden when the only link is the parent's sub-page block. Untitled pages show their first line in tertiary italics where they are listed, open or not; a new database row opens with its title focused, as a new page does. Ctrl+K and Ctrl+P open a search overlay. `[[` opens the page picker. The rail drops the pencil icon and the chevron on pages without children, and its section is called "Pages".

## Icons and tooltips (2026-10-21)

- **Icon set.** [Lucide](https://lucide.dev) (ISC), as published, from the `lucide_icons_flutter` font package (pinned in `app/pubspec.yaml`; source, version and licence in [THIRD_PARTY.md](../../THIRD_PARTY.md)). Call sites keep the app's names: `Ico` ([icons.dart](../../app/lib/ui/icons.dart)) and `Ix` ([icon_x.dart](../../app/lib/ui/icon_x.dart)) map each name to a Lucide glyph, drawn by `Icon20`/`IconX`/`IconBtn`/`IconBtnX`. The stroke follows the app's 1.5 px line: [lucide.dart](../../app/lib/ui/lucide.dart) picks the nearest of Lucide's 1.55/2/2.5 weights for the size. The filled star and the toggle triangle are Lucide's `star` and `play` with their inside filled.
- **Feedback** has its own icon, `Ix.feedback` (Lucide `message-square-warning`); "Add icon" keeps the smile.
- **Tooltips.** `AppTooltip(label:, command:/shortcut:)` ([tooltip.dart](../../app/lib/ui/tooltip.dart)) is the one tooltip: a small elevated label with the shortcut as a muted hint. Desktop: after 500 ms of hover or on keyboard focus; hides on leave, click, Escape or scroll. Touch: long-press, which does not press the button. It stays inside the window, follows the theme and reduced motion, and leaves the button's semantic label alone. `IconBtn` and `IconBtnX` add it from their label; `command:` takes the shortcut from the command registry so the hint never drifts.
- **Rule for new UI.** New UI uses only these icons (add a name to `Ico` or `Ix`, mapped to a Lucide glyph; never a hand-drawn or Material icon) and gives every icon-only button an `AppTooltip` (or an `IconBtn`/`IconBtnX` with a `semanticLabel`, plus `command:` when it runs a registered command).

## App icon

- **Brief.** The notebook is the cat: an upright spiral notebook whose top edge rises into ears, with the face on the cover, calico ears (charcoal left, apricot right), a page block, and a bookmark ribbon. It uses the BubbleBuffer family contour and colours (NeedleBub, LilBub).
- **Source.** [notes-icon.svg](../../design/icon/notes-icon.svg) is built in Pawth from the kit [bubblebuffer.kit.svg](../../design/icon/kits/bubblebuffer.kit.svg) (the `book/*` and `face/cover` parts). Run [rebuild.sh](../../design/icon/rebuild.sh) from `design/icon` to rebuild it through `pawthctl`. Edit the kit parts, not the generated SVG.
- **Other options.** B (hardcover), C (open book), D (strap journal) and E (tail bookmark) are in `design/icon/options/`, compared on [options.html](../../design/directions/icon/options.html).
- **Shipped.** The Pawth icon pack (`design/icon/notes-icon-icons/`) supplies the Android mipmaps, the round and adaptive icons (with a monochrome layer) and the Windows `app_icon.ico`. To update the icon, re-export the pack and copy it over these files.
- **Style check.** No errors. It gives safe-zone warnings only; the circle mask grazes the ear tips.

## Decisions

- 2026-10-07: the Stacks direction (C) is chosen, with Patchwork's space colours used quietly.
- 2026-10-07: Calico tabs and fun are approved: space colour on edges and tiles, with the calico mark and small playful touches.
- 2026-10-08: the app icon is option A, the spiral notebook with calico ears.
- 2026-10-10: mobile navigation is one dock row, with side panels on Home instead of a four-tab bar.
- 2026-10-21: the icon set is Lucide behind the app's own icon names, and `AppTooltip` is the one tooltip; new UI uses both.

Plans, open questions and the full decision record are in the internal wiki (notes vault, notes-app > Product > Design).
