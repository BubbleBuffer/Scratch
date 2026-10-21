# Vault calendar mockup
Open `index.html` directly, or serve this folder as the web root with `python -m http.server 8000 --bind 127.0.0.1 --directory design/calendar` from the repository root. No application build or network service is needed. The three fonts and their licences are bundled in `fonts/`.
Toolbar: Light/Dark, Desktop (1280)/Phone (390), Screen picker, How it fits, Reset demo.
Existing screens: Month, Week, Day, Calendars, task detail, composer, busy-day overflow, jump, note date chip, kanban & embed, shortcuts.
Today screens: anchored on today with yesterday peeking above it, next week with an empty day, last week with done items, week-strip sync, calendar/source, future-day composer, created future task, How it fits.
Today and Calendar are sibling top-level views in the sidebar and phone navigation. Switching retains the selected date; Calendar's Today button only returns its calendar to today.
Today's header has the page title, the strip week's month, week arrows, a conditional Today pill and jump controls. Click the month or press G to jump within Today. The mockup toolbar holds How it fits.
Today continuously loads every day for 21 days before/after Mon 12 Oct and extends when jumping beyond that range. Empty days say Nothing planned; past headings are muted and today's is accented. Scroll up/down with sticky dates and a synced week strip. Week arrows scroll to the previous/next week's Monday; strip taps scroll to that day.
Today includes dated tasks, database rows and to-dos; plain @date references stay in Calendar, and Agent's Show in Today starts off. Calendar and Today share item edits, colours and independent visibility switches.
Every day list, popover, Today, kanban and embed uses the same short card: one desktop line with checkbox/kind, title, time and a muted source; at phone width, title then one metadata line. The left bar carries calendar colour. Task sources show calendar dot/name; rows show database icon/name; to-dos and mentions show note icon/title. Hover or tap the source to reveal calendar and source details.
Day headers provide dates; only overdue cards show their original date in the overdue colour, replacing time. Board and embed cards retain dates. Only Agent gets an assignment badge. Completed cards keep their height and become muted with a struck-through title. The detail sheet retains all fields. Overdue remains collapsible inside today's section.
Fake data: 103 Sep-Nov items, five calendars, four notes; edits reset on reload/Reset demo. Drag notes/tasks to calendar days; Undo is available.
Phone week/day views support pointer swipes. Desktop: T back to today; M/W/D calendar views; arrow keys or K/J move a period (a day in Today); Up/Down move a day.
G jump; N create on the selected day; Enter create; Esc close; ? shortcuts. Keys pause while typing; Today strip taps respect reduced motion.
Checks: `node --check calendar.js` / `calendar-data.js` / `today-agenda.js`; `check.cjs` needs installed Playwright (or `CALENDAR_PLAYWRIGHT`), Chrome and Python.
`node design/calendar/check.cjs` starts a local HTTP server with this folder as its root and stops both browser and server at the end. It covers every screen x theme x width, actual 390px page overflow, all three loaded font faces, desktop Today card height <=48px, and interactions. `.checks/` holds captures, including `today-*`, and `results.json`.
## Open questions (lead's recommendation in brackets)
- Agent calendar colour: keep duvet, or reserve a colour for the agent? [Keep duvet; the Agent badge on cards already marks agent work.]
- Phone dock: which actions sit beside Today and Calendar? [Pages, Today, Calendar, Search; the agent stays in the sidebar/menu.]
- Note with several dates: which date does the title-area chip show? [The next upcoming date, with "+2" opening the list; the earliest one if all are past.]
- Removing a database calendar: [Only hides its overlay (Show in calendar off); rows and their dates stay in the database.]
- Timed task crossing midnight: [One block per day, clipped at midnight, the second marked as continued; Today lists it on its start day only.]
- Completing an overdue item in Today: [Completion is logged today in the done list; the card keeps its original due date.]
- Today's empty days show "Nothing planned": keep, or collapse runs of empty days? [Keep single lines; collapse 3+ empty days into one "Nothing planned until Thu 29 Oct" line.]
