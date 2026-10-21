# Visual directions: shared brief

Status: design exploration, 2026-10-06. Three directions are mocked as clickable HTML so the user can pick or mix one. Nothing here is built in the app yet.

## Why

The Flutter app now matches the reference app closely, and the user finds it "dead and empty and lifeless". Matching the reference app was the point of Milestone 1; now the app should become its own. The user's answers (2026-10-06):

- Life should come from **motion, colour, personality and depth**, all four.
- The structure also feels flat: the demo and the home screen show little more than text and to-dos. Richer structure is wanted, which means some features that don't exist yet.
- Mood: **playful**.
- It joins the **BubbleBuffer design family**, but it must not look like the sibling products. It should feel varied.

## The family rules that apply

Read `%USERPROFILE%\.claude\plugins\cache\bubblebuffer-local\bubblebuffer-design\0.1.0\assets\design-system\DESIGN.md` in full before designing. The essentials:

- **Palette:** milk cloth `#F3EFE7`, raised milk `#FFFAF1` and duvet `#2F3138` / ink `#282A2F` own most of the area. Calico apricot `#DE945F` (action; small text `#874016`; dark-mode `#F0A064`), paw pink `#DDA39D` (creative, recovery) and eye moss `#697A6B` (grounding) arrive in unequal fields. The semantic pastels are constructive `#CAD8CD`, information `#CBD5DF`, attention `#F0D29C` and danger `#E5B1AA`. Focus blue `#7AB7FF` is for keyboard focus only.
- **Type:** Bricolage Grotesque for display and titles, Atkinson Hyperlegible Next for UI and body, Fragment Mono only for technical values. Use the Google Fonts versions. The Atkinson Hyperlegible Next family name on Google Fonts is "Atkinson Hyperlegible Next".
- **Shape:** fitted, slightly unequal corners (`7px 11px 6px 9px` for controls, `12px 18px 10px 16px` for panels). Pills only for statuses and round controls.
- **Avoid:**
  - The reference app's blue
  - Generic SaaS card walls, bento grids and decorative blobs
  - Broad glass blur
  - Gradients used as a shortcut for character (the current app's gradient covers count)
  - Tiny all-caps mono eyebrows
- **States:** state is never shown by colour alone, and focus is always visible.
- **Motion:** prefer `prefers-reduced-motion`; animations snap when it is set.

"Playful" is the user's explicit request and outranks the family's default restraint. The family allows an *expressive* profile, which is what this is. Keep the operational parts (editing text, menus, lists) easy to read, and spend the play on navigation, icons, empty states, completion moments, covers and structure.

## Same content in every direction

Use this vault so the three can be compared fairly. The data is illustrative.

- **Spaces / top level:** Home life, Work, Trips, Reading, Garden.
- **Pages:**
  - Lisbon in May (in Trips)
  - Weekly review (in Work)
  - Q4 roadmap (in Work, a database)
  - Recipes (in Home life, a database)
  - Sourdough (a recipe)
  - Garden plan
  - Book list (in Reading, a database)
- **Tasks database:** about 12 tasks across those areas, with status (To do, Doing, Done), due dates around "today" (Tuesday 6 October) and a few overdue. Examples: "Book Lisbon flights", "Feed sourdough starter", "Draft Q4 goals", "Plant garlic", "Return library books", "Call plumber".
- **Calendar items today:** "Standup 09:30", "Dentist 14:00", "Pottery class 19:00".
- **The rich page to show:** *Lisbon in May*. It has:
  - a cover and an icon
  - properties: dates 12–19 May, status Planning, budget €1,400, tags
  - a callout with the flight confirmation
  - two columns, "Must see" and "Must eat"
  - a checklist "Packing" with progress
  - a toggle "Day by day"
  - a small inline board "Bookings" (To book / Booked)
  - a mention of the page "Weekly review"
  - an image placeholder
  - backlinks at the bottom

## Each direction must show

All in one self-contained `index.html` in its folder. Google Fonts are allowed; no other network, no frameworks, and inline SVG for icons and artwork.

1. **Home, desktop (about 1440×900),** with a sidebar. Richer structure than a list of recents. Propose features, and mark each feature that doesn't exist yet with a small "proposed" tag, in a way that does not dominate. Ideas: a Today area (agenda from calendar items plus tasks due or overdue), quick capture, spaces with their own colour or character, pinned pages, progress, a daily note, recent activity. Pick what suits the direction; don't use all of them.
2. **The Lisbon page, desktop,** reached by clicking it on home. Navigation between the two must animate.
3. **Home and the Lisbon page on mobile (about 390×844),** shown in a phone frame or through a frame switch. Mobile-first is a core requirement: the controls the user needs should be in the bottom half.
4. **One empty or first-run state** (a new empty page, or an empty space), with personality.
5. **A completion moment:** ticking a to-do or finishing the checklist.
6. **Light and dark,** with a toggle.
7. **Motion:**
   - hover and press feedback
   - page transition
   - menu or popover in and out (for example a "+" or "/" menu)
   - list or row entry
   - the completion moment
   - honour `prefers-reduced-motion`
8. **A short note in the page** (collapsible or at the bottom) naming the direction, its one signature idea, and what it would cost to build in Flutter (rough: small, medium, large per part).

Clickable means the main navigation works (home ↔ page, theme toggle, desktop/mobile switch, to-dos tick, toggle opens, the "+" menu opens). It is a prototype, not the app: no persistence and no real editing beyond ticking and toggling.

## Quality bar

- It must feel alive within five seconds of opening it, without being noisy while reading.
- Text-heavy reading and editing areas stay calm and legible (body 15–17px Atkinson, line height about 1.55).
- Check your own result headlessly. Take screenshots with Playwright in headless Chromium only:
  - Playwright is in `prototypes/tauri/node_modules`; use the pattern in `design/directions/playwright.cjs` (it exports `launch()` and `newPage()` for headless Chromium).
  - Capture desktop light and dark, mobile light and dark, the page, and the empty state.
  - Look at every screenshot and fix what is off.
  - Never open windows on the user's desktop.
- Keep the screenshots in your folder under `shots/`.
