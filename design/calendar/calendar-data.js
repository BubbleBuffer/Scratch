// Sample data for the plain HTML calendar mockup.
// Palette names follow app/lib/ui/tokens.dart spaceColorNames:
//   apricot #DE945F, duvet #2F3138 light / #999DA6 dark (theme var --duvet), sky #CBD5DF, sun #F0D29C, moss #697A6B (dot) / #5F7062 (patch)
// Load with an ordinary <script> tag; reads as window.CALENDAR_DATA.
(function () {
  // "today" marks calendars pulled into the Today list. Agent stays out of it.
  // "database" marks calendars whose rows come from a database (Due-date rows).
  const calendars = [
    { id: 'personal', name: 'Personal', palette: 'apricot', visible: true, today: true, database: false },
    { id: 'work', name: 'Work', palette: 'sky', visible: true, today: true, database: false },
    { id: 'agent', name: 'Agent', palette: 'duvet', visible: true, today: false, database: false },
    { id: 'projects', name: 'Projects', palette: 'moss', visible: true, today: true, database: true },
    { id: 'reading', name: 'Reading list', palette: 'sun', visible: true, today: true, database: true },
  ];

  const items = [
    // September
    {
      id: 'i01', title: 'Renew passport photos', date: '2026-09-02', calendar: 'personal', kind: 'task',
      done: true, time: null, duration: null, assignee: 'me',
      body: 'Passport office wants 2x2 colour photos taken in the last six months. The pharmacy booth on Elm does them for $12.',
      snippet: 'Pharmacy booth on Elm, $12.', noteId: null,
    },
    {
      id: 'i02', title: 'Due: Thinking in Systems, ch. 1-4', date: '2026-09-09', calendar: 'reading', kind: 'db',
      done: true, time: null, duration: null, assignee: 'me',
      body: 'Due-date row from the Reading list database. Status moved to Done after the book group.',
      snippet: 'Reading list · Status: Done', noteId: null,
    },
    {
      id: 'i03', title: 'Dentist cleaning', date: '2026-09-15', calendar: 'personal', kind: 'task',
      done: true, time: '09:30', duration: 45, assignee: 'me',
      body: 'Ask about the sensitivity on the lower left. Bring the insurance card.',
      snippet: 'Dr. Okafor, 6th floor.', noteId: null,
    },
    {
      id: 'i04', title: 'Cancel the old streaming plan', date: '2026-09-22', calendar: 'personal', kind: 'task',
      done: true, time: null, duration: null, assignee: 'me',
      body: 'Keep the family plan, cancel the solo one. Check the September bill first.',
      snippet: 'Keep family, cancel solo.', noteId: null,
    },
    {
      id: 'i05', title: 'Due: Website launch copy', date: '2026-09-28', calendar: 'projects', kind: 'db',
      done: true, time: null, duration: null, assignee: 'me',
      body: 'Projects row. Copy reviewed; the final pass on the pricing page is done.',
      snippet: 'Projects · Owner: me', noteId: null,
    },

    // October before today
    {
      id: 'i06', title: 'Quarterly budget review', date: '2026-10-01', calendar: 'work', kind: 'task',
      done: true, time: '10:00', duration: 60, assignee: 'me',
      body: 'Export the Q3 sheet, compare it to plan, and flag anything more than 10% over.',
      snippet: 'Q3 sheet vs plan.', noteId: null,
    },
    {
      id: 'i07', title: 'File tax extension paperwork', date: '2026-10-03', calendar: 'personal', kind: 'task',
      done: true, time: null, duration: null, assignee: 'me',
      body: 'The paperwork is filed. Keep a copy with the annual budget notes.',
      snippet: 'Needs the March 1099 totals.', noteId: null,
    },
    {
      id: 'i08', title: "Summarize last week's reading notes", date: '2026-10-05', calendar: 'agent', kind: 'task',
      done: true, time: null, duration: null, assignee: 'agent',
      body: 'Agent pulled the highlights from the Thinking in Systems notes into one page.',
      snippet: 'One page of highlights.', noteId: null,
    },
    {
      id: 'i09', title: 'Due: Kitchen faucet quote', date: '2026-10-07', calendar: 'projects', kind: 'db',
      done: true, time: null, duration: null, assignee: 'me',
      body: 'Kitchen Co. quote is saved. Compare it with the second quote before Friday.',
      snippet: 'Projects · $240 with labour', noteId: null,
    },
    {
      id: 'i10', title: 'Send invoice to Harbour Design', date: '2026-10-09', calendar: 'work', kind: 'task',
      done: false, time: null, duration: null, assignee: 'me',
      body: 'Invoice 0412, 14 hours at the agreed rate. Copy Dana on the email.',
      snippet: 'Invoice 0412, copy Dana.', noteId: null,
    },
    {
      id: 'i11', title: "Draft weekly review from this week's notes", date: '2026-10-13', calendar: 'agent', kind: 'task',
      done: false, time: null, duration: null, assignee: 'agent',
      body: 'Gather Monday to Friday notes into a draft review. Leave the final edits to me.',
      snippet: 'Draft from Mon-Fri notes.', noteId: null,
    },

    // Today, 2026-10-12 (8 items)
    {
      id: 'i12', title: 'Water the plants', date: '2026-10-12', calendar: 'personal', kind: 'task',
      done: false, time: null, duration: null, assignee: 'me',
      body: 'The fern and the window basil. The basil is thirsty on Tuesdays.',
      snippet: 'Fern and basil.', noteId: null,
    },
    {
      id: 'i13', title: 'Standup: list blockers', date: '2026-10-12', calendar: 'work', kind: 'task',
      done: false, time: '09:15', duration: 15, assignee: 'me',
      body: 'Three lines: the sync bug, the onboarding copy, and waiting on the design review.',
      snippet: 'Three blockers.', noteId: null,
    },
    {
      id: 'i14', title: 'Pay power bill', date: '2026-10-12', calendar: 'personal', kind: 'task',
      done: true, time: null, duration: null, assignee: 'me',
      body: 'Autopay failed last month. Pay it directly this time and keep the receipt.',
      snippet: 'Autopay failed last month.', noteId: null,
    },
    {
      id: 'i15', title: 'Review onboarding copy draft', date: '2026-10-12', calendar: 'work', kind: 'task',
      done: false, time: '11:00', duration: 45, assignee: 'me',
      body: 'Leave comments in the doc, not in chat. Focus on the empty-state lines.',
      snippet: 'Empty-state lines first.', noteId: null,
    },
    {
      id: 'i16', title: 'Collect Lisbon links', date: '2026-10-12', calendar: 'agent', kind: 'task',
      done: false, time: '16:00', duration: 60, assignee: 'agent',
      body: 'Agent gathers the tabs tagged Lisbon into the trip plan note.',
      snippet: 'Tabs tagged Lisbon.', noteId: null,
    },
    {
      id: 'i17', title: 'Due: Q4 roadmap sign-off', date: '2026-10-12', calendar: 'projects', kind: 'db',
      done: false, time: null, duration: null, assignee: 'me',
      body: 'Projects row. Sign-off needed from Priya before planning starts on Monday.',
      snippet: 'Projects · Needs Priya', noteId: null,
    },
    {
      id: 'i18', title: 'Lunch with Sam', date: '2026-10-12', calendar: 'personal', kind: 'task',
      done: false, time: '12:30', duration: 60, assignee: 'me',
      body: 'The noodle place, not the other one. Sam is bringing the drill back.',
      snippet: 'Noodle place.', noteId: null,
    },
    {
      id: 'i19', title: 'Call the landlord about the hallway light', date: '2026-10-12', calendar: 'personal', kind: 'todo',
      done: false, time: null, duration: null, assignee: 'me',
      body: 'The light flickers when the fridge kicks on. Ask for an electrician on Thursday.',
      snippet: 'Electrician on Thursday?', noteId: 'apartment-repairs',
    },

    // October 14-16
    {
      id: 'i20', title: '1:1 notes', date: '2026-10-14', calendar: 'work', kind: 'mention',
      done: false, time: '10:30', duration: 30, assignee: 'me',
      body: 'From the 1:1 notes: the hiring loop is booked for two weeks out.',
      snippet: 'Roadmap and hiring loop.', noteId: 'one-on-one',
    },
    {
      id: 'i21', title: 'Sprint carryover review', date: '2026-10-14', calendar: 'work', kind: 'task',
      done: false, time: '15:00', duration: 30, assignee: 'me',
      body: 'Dev has the sync test flake. Agreed to timebox it to Wednesday.',
      snippet: 'Sync test flake, timeboxed.', noteId: null,
    },
    {
      id: 'i22', title: 'Renew gym membership', date: '2026-10-14', calendar: 'personal', kind: 'task',
      done: false, time: null, duration: null, assignee: 'me',
      body: 'The annual price went up. Check the family plan before paying.',
      snippet: 'Check the family plan.', noteId: null,
    },
    {
      id: 'i23', title: 'Trip plan', date: '2026-10-16', calendar: 'personal', kind: 'mention',
      done: false, time: null, duration: null, assignee: 'me',
      body: 'Compare travel options for the coast. Keep Friday free for planning.',
      snippet: 'Keep @Fri 16 Oct free for planning before the trip.', noteId: 'trip-plan',
    },

    // Later
    {
      id: 'i24', title: 'Find three ferry options from Thursday', date: '2026-10-20', calendar: 'agent', kind: 'task',
      done: false, time: null, duration: null, assignee: 'agent',
      body: 'Agent checked the timetables for the 22nd. Results go into the trip plan note.',
      snippet: 'Timetables for the 22nd.', noteId: null,
    },
    {
      id: 'i25', title: 'Due: Designing Data-Intensive Applications', date: '2026-10-22', calendar: 'reading', kind: 'db',
      done: false, time: null, duration: null, assignee: 'me',
      body: 'Reading list row, due before the book group meets.',
      snippet: 'Reading list · Book group', noteId: null,
    },
    {
      id: 'i26', title: 'Dentist follow-up', date: '2026-10-27', calendar: 'personal', kind: 'task',
      done: false, time: '14:00', duration: 30, assignee: 'me',
      body: 'Check the filling on the lower left. Bring the insurance card.',
      snippet: 'Lower left filling.', noteId: null,
    },
    {
      id: 'i27', title: 'Submit expense report', date: '2026-11-03', calendar: 'work', kind: 'task',
      done: false, time: null, duration: null, assignee: 'me',
      body: 'Attach the receipts from the September conference. Two are still missing.',
      snippet: 'Two receipts missing.', noteId: null,
    },
    {
      id: 'i28', title: 'Pick a colour for the hallway', date: '2026-11-05', calendar: 'personal', kind: 'todo',
      done: false, time: null, duration: null, assignee: 'me',
      body: 'Three samples from the hardware store. Test them on the north wall first.',
      snippet: 'Three samples to test.', noteId: null,
    },
  ];

  // Generated rows (g-prefixed IDs) fill the 43 days from Mon 21 Sep to Mon 2 Nov 2026.
  // Rows before today default to done; rows from today on default to open.
  const row = (id, date, calendar, kind, title, snippet, body, extra = {}) => ({
    id, title, date, calendar, kind,
    done: date < '2026-10-12', time: null, duration: null, assignee: 'me',
    body, snippet, noteId: null, ...extra,
  });

  items.push(
    // 21-30 Sep
    row('g01', '2026-09-21', 'personal', 'task', 'Buy birthday card for Mum', 'Card and stamp.', 'The one with the lighthouse. Post it by Thursday so it arrives on the day.'),
    row('g02', '2026-09-21', 'work', 'task', 'Expense the conference taxi', 'Airport receipt missing.', 'Upload the taxi receipts to the expense tool. The airport one is still in the email.'),
    row('g03', '2026-09-22', 'work', 'task', 'Post the stand-up notes', 'Posted to the channel.', 'Release date unchanged, design review moved to Friday.'),
    row('g04', '2026-09-23', 'projects', 'db', 'Due: Garden shed plan', 'Projects · Owner: me', 'Projects row. Measurements done, the shed sketch is saved in the doc.'),
    row('g05', '2026-09-23', 'personal', 'task', 'Pottery class', 'Studio on Fern Street.', 'Bring the apron. The glaze test tiles go home with you.', { time: '18:30', duration: 60 }),
    row('g06', '2026-09-24', 'reading', 'db', 'Due: The Overstory, part one', 'Reading list · Status: Done', 'Reading list row. Part one finished and the notes are saved.'),
    row('g07', '2026-09-24', 'work', 'task', 'Renew code signing certificate', 'Expires in November.', 'The renewal request went in. The new certificate goes into the build vault once it arrives.'),
    row('g08', '2026-09-25', 'personal', 'task', 'Pick up dry cleaning', 'Ticket is in the blue wallet.', 'Two shirts and the wool coat.'),
    row('g09', '2026-09-25', 'work', 'todo', 'Ask Priya about the design review slot', '1:1 notes.', 'From the 1:1 notes. Priya will check the calendar on Monday.', { noteId: 'one-on-one' }),
    row('g10', '2026-09-26', 'personal', 'task', 'Farmers market run', 'Eggs, greens, one loaf.', 'Eggs from the stall by the fountain, greens, and the sourdough if it is still warm.', { time: '10:00', duration: 90 }),
    row('g11', '2026-09-26', 'personal', 'task', 'Charge the bike light', 'USB-C cable in the drawer.', 'The front light only holds charge for an hour now. Replace the battery if it drops again.'),
    row('g12', '2026-09-27', 'personal', 'task', 'Call Grandma back', 'Ask about the knee.', 'She asked about the knee appointment. Call after dinner.'),
    row('g13', '2026-09-27', 'projects', 'db', 'Due: Mobile onboarding spec', 'Projects · Owner: Dev', 'Projects row. Spec is in the doc and signed off by design.'),
    row('g14', '2026-09-28', 'work', 'task', 'Weekly planning', 'Agenda in the doc.', 'Agree the three goals for next week and assign the copy review.', { time: '09:00', duration: 30 }),
    row('g15', '2026-09-29', 'work', 'task', 'Order new monitor stand', 'Returned the wrong size.', 'The first stand was too short. Order the adjustable one and keep the receipt.'),
    row('g16', '2026-09-29', 'personal', 'todo', 'Ask the landlord about the radiator', 'Apartment repairs.', 'The radiator in the back bedroom knocks at night. Ask whether it can be bled before winter.', { noteId: 'apartment-repairs' }),
    row('g17', '2026-09-30', 'personal', 'task', 'Move the savings transfer', 'Monthly, before the 5th.', 'Move the monthly amount to the savings account before the bills go out.'),
    row('g18', '2026-09-30', 'work', 'task', 'Vendor check-in', 'Q3 invoice questions.', 'Ask about the late invoice and confirm the renewal date.', { time: '14:00', duration: 45 }),
    // 1-11 Oct
    row('g76', '2026-10-01', 'personal', 'task', 'Change the heater filter', 'Spare filter in the hall cupboard.', 'The filter is on the list every quarter. The spare is already in the cupboard.'),
    row('g19', '2026-10-02', 'personal', 'task', 'Print the lease copy', 'Two copies, one signed.', 'The landlord needs one signed copy back by the end of the month.'),
    row('g20', '2026-10-02', 'reading', 'db', 'Due: Piranesi', 'Reading list · Status: Done', 'Reading list row. Finished early, so the book group gets it on the 9th.'),
    row('g77', '2026-10-03', 'work', 'task', 'Archive the Q3 sheet', 'Copy in the shared folder.', 'Move the Q3 sheet and the plan comparison to the archive folder.'),
    row('g21', '2026-10-04', 'personal', 'task', 'Clean out the fridge', 'Use the yoghurt first.', 'Check the dates and toss anything past the label date.'),
    row('g22', '2026-10-04', 'work', 'task', 'Review the quarterly sheet draft', 'Two comments left.', 'Two comments on the revenue tab. Send them back before lunch.'),
    row('g23', '2026-10-05', 'work', 'task', 'Sprint planning', 'Scope for the sync fix.', 'Scope the sync fix and the onboarding copy for the next sprint.', { time: '09:30', duration: 30 }),
    row('g24', '2026-10-05', 'personal', 'task', 'Order printer ink', 'Black and yellow.', 'Compatible cartridges from the shop on Pine, not the big box.'),
    row('g25', '2026-10-06', 'personal', 'task', 'Morning run', 'Five kilometre loop.', 'The river loop at an easy pace.', { time: '07:30', duration: 40 }),
    row('g26', '2026-10-06', 'work', 'task', 'Write up the Q3 retro', 'Three themes, two actions.', 'Keep it to one page: three themes and two actions.'),
    row('g27', '2026-10-07', 'personal', 'task', 'Post the parcel', 'Return label is in the box.', 'Use the post office on Elm, not the locker.'),
    row('g28', '2026-10-08', 'work', 'task', 'Design review prep', 'Two screens, one question.', 'Pick the two onboarding screens to review and write the open question.', { time: '13:00', duration: 30 }),
    row('g29', '2026-10-08', 'personal', 'task', 'Renew bus pass', 'Online, before the 10th.', 'Renew online, then add the card to the phone wallet.'),
    row('g30', '2026-10-09', 'projects', 'db', 'Due: Bike repair estimate', 'Projects · $85', 'Projects row. Shop estimate saved; they can do it next week.'),
    row('g31', '2026-10-10', 'personal', 'task', 'Buy stamps', 'Book of twenty.', 'Two books, the first class ones.'),
    row('g32', '2026-10-10', 'personal', 'task', 'Brunch with the Ortiz family', 'Corner table.', 'The corner table at 11. Bring the photo album.', { time: '11:00', duration: 60 }),
    row('g33', '2026-10-11', 'personal', 'task', 'Return library books', 'Three books, due Saturday.', 'Three books are overdue at the central branch. Drop them in the slot and keep the receipt.', { done: false }),
    row('g34', '2026-10-11', 'personal', 'task', 'Sort the spare keys', 'Label the two new ones.', 'Label the spare keys by door and put them in the hall tray.'),
    // 13-21 Oct
    row('g35', '2026-10-13', 'work', 'task', 'Design review', 'Onboarding screens.', 'Walk through the two onboarding screens with design. Bring the open question from the list.', { time: '09:00', duration: 30 }),
    row('g36', '2026-10-13', 'personal', 'task', 'Book a table for Friday', 'Party of four, 19:30.', 'The Harbour place. Ask for the window if it is free.'),
    row('g37', '2026-10-14', 'personal', 'todo', 'Write the stock paragraph', 'Thinking in Systems, ch. 5.', 'Chapter 5 asks for one paragraph on the stock in my own week. Draft it before the book group.', { noteId: 'reading-notes' }),
    row('g38', '2026-10-15', 'personal', 'todo', 'Electrician visit, Thursday morning', 'Hallway light, morning slot.', 'The landlord confirmed the morning slot. Be home before 10:00 and clear the hallway.', { noteId: 'apartment-repairs' }),
    row('g39', '2026-10-15', 'work', 'task', 'Send the onboarding copy', 'After the review.', 'Send the final copy to the design channel once the comments are in.', { time: '16:30', duration: 30 }),
    row('g40', '2026-10-16', 'personal', 'task', 'Pick the ferry tickets', 'Friday is planning day.', 'Look at the 08:10 and 10:40 sailings to Cascais before booking.'),
    row('g41', '2026-10-16', 'work', 'task', 'Hiring loop debrief', 'Panel notes due.', 'Two candidates in the loop. Notes are due before the debrief starts.', { time: '11:30', duration: 30 }),
    row('g42', '2026-10-17', 'personal', 'task', 'Pick up the cake order', 'Lemon, eight inches.', 'The bakery on Fern. Confirm the pickup time by phone.'),
    row('g43', '2026-10-17', 'personal', 'task', 'Garden centre trip', 'Bulbs and compost.', 'Tulip bulbs for the pots by the door, and two bags of compost.', { time: '10:00', duration: 60 }),
    row('g44', '2026-10-18', 'personal', 'task', 'Wash the bedding', 'Cold wash.', 'Wash the bedding on cold and dry it on the line if the weather holds.'),
    row('g45', '2026-10-18', 'reading', 'db', 'Due: Sapiens, chapters 1-6', 'Reading list · Book group', 'Reading list row. Chapters 1 to 6 are due before the book group meets.'),
    row('g46', '2026-10-19', 'personal', 'todo', 'Book the ferry to Cascais', 'Still open in Trip plan.', 'Choose the 10:40 sailing and keep the return on the 17:20. The Sintra day depends on it.', { noteId: 'trip-plan' }),
    row('g47', '2026-10-19', 'work', 'task', 'Review the sync test fix', 'Timeboxed to Wednesday.', 'Dev has the sync fix ready for review. The timebox stays Wednesday.', { time: '15:00', duration: 30 }),
    row('g48', '2026-10-20', 'projects', 'db', 'Due: Brand refresh checklist', 'Projects · Owner: Priya', 'Projects row. Logos go to review first, then the colour pass.'),
    row('g49', '2026-10-20', 'personal', 'task', 'Renew library card', 'Online, takes two minutes.', 'Expiry is the end of the month. The renewal link is in the library app.'),
    // 21-22 Oct: next-week examples
    row('g50', '2026-10-21', 'personal', 'todo', 'Check in for the Lisbon flight', 'Flights booked for the 22nd.', 'Online check-in opens 24 hours before. Save the boarding passes to the wallet.', { noteId: 'trip-plan' }),
    row('g51', '2026-10-21', 'personal', 'task', 'Pack for the coast', 'Five nights, one bag.', 'Sun hat and the walking shoes go in the outside pocket.', { time: '18:00', duration: 60 }),
    row('g52', '2026-10-21', 'work', 'task', 'Onboarding copy sign-off', 'Priya approves the empty states.', 'Sign-off meeting for the empty-state lines. Bring the comments from the doc.', { time: '09:30', duration: 45 }),
    row('g53', '2026-10-22', 'personal', 'task', 'Flight to Lisbon', 'Gate B, 07:10.', 'The flight leaves at 07:10 from gate B. Leave for the airport at 05:30.', { time: '07:10', duration: 170 }),
    row('g54', '2026-10-22', 'personal', 'todo', 'Confirm the dinner reservation', 'First night in Lisbon.', 'Still open in Trip plan: one dinner reservation. Book it for the first night.', { noteId: 'trip-plan' }),
    // 23-31 Oct
    row('g57', '2026-10-24', 'personal', 'task', 'Morning swim', 'Lane three.', 'Early swim before the day gets busy.', { time: '08:00', duration: 45 }),
    row('g58', '2026-10-24', 'projects', 'db', 'Due: Kitchen tile samples', 'Projects · Needs photos', 'Projects row. Photos of the three samples go in the shared folder.'),
    row('g59', '2026-10-25', 'personal', 'task', 'Call the bank about the card', 'Replacement card on its way.', 'Confirm the replacement card arrived and set the new PIN.'),
    row('g60', '2026-10-25', 'work', 'todo', 'Ask Dev for the timebox update', '1:1 notes.', 'From the 1:1 notes. Get the status before standup.', { noteId: 'one-on-one' }),
    row('g61', '2026-10-26', 'work', 'task', 'Monday planning', 'Three goals.', 'Pick three goals for the week and set the review slot.', { time: '10:00', duration: 30 }),
    row('g62', '2026-10-26', 'personal', 'task', 'Pay the water bill', 'Direct debit failed.', 'Pay it directly and keep the receipt in the folder.'),
    row('g63', '2026-10-27', 'work', 'task', 'Expense the hotel invoice', 'Upload the PDF.', 'The hotel PDF is in the travel folder. Attach it before the expense report goes in.'),
    row('g64', '2026-10-28', 'personal', 'task', 'Dance class', 'Studio B.', 'Bring the soft shoes.', { time: '17:30', duration: 45 }),
    row('g65', '2026-10-28', 'reading', 'todo', 'Draft three questions for the book group', 'From the Thinking in Systems notes.', 'Three questions on feedback loops for the group.', { noteId: 'reading-notes' }),
    row('g66', '2026-10-29', 'personal', 'task', 'Replace the bike light battery', 'Battery is in the drawer.', 'The replacement battery from last month.'),
    row('g67', '2026-10-29', 'work', 'task', 'Design review follow-up', 'Action items.', 'Write up the action items from the review and send them to the channel.', { time: '15:30', duration: 30 }),
    row('g68', '2026-10-30', 'projects', 'db', 'Due: Q4 budget draft', 'Projects · Owner: me', 'Projects row. Draft is due before the planning sign-off next week.'),
    row('g69', '2026-10-30', 'personal', 'task', 'Send the tax forms', 'To the accountant.', 'Scan and send the forms. Keep the originals in the blue folder.'),
    row('g70', '2026-10-31', 'personal', 'task', 'Check the hallway bulb', 'Bring the spare.', 'If the electrician is not needed, change the bulb first.'),
    row('g71', '2026-10-31', 'personal', 'todo', 'Compare the second faucet quote', 'Second quote needed.', 'Kitchen Co. is $240 with labour. Get one more quote before Friday.', { noteId: 'apartment-repairs' }),
    // 1-2 Nov
    row('g72', '2026-11-01', 'personal', 'task', 'Weekend market', 'Apples and bread.', 'The usual stalls. Apples for the week and a loaf.', { time: '09:00', duration: 60 }),
    row('g73', '2026-11-01', 'reading', 'db', 'Due: Designing Data-Intensive Applications, ch. 4', 'Reading list · In progress', 'Reading list row. Chapter 4 notes are due for the next book group.'),
    row('g74', '2026-11-02', 'work', 'task', 'Weekly planning', 'Agenda in the doc.', 'Set the week: review, copy sign-off, and the expense report.', { time: '09:30', duration: 30 }),
    row('g75', '2026-11-02', 'personal', 'task', 'Pay the car insurance', 'Due this week.', 'Autopay runs on the 4th. Check it went through.'),
  );

  const notes = [
    {
      id: 'trip-plan',
      title: 'Trip plan',
      body: 'Lisbon and the coast, five nights, late October.\n\nFlights are booked for the 22nd.\nStill open: the ferry to Cascais, one dinner reservation, and a day in Sintra. Keep it loose.',
    },
    {
      id: 'one-on-one',
      title: '1:1 notes',
      body: 'Priya: Q4 roadmap, hiring loop in two weeks, ask about the design review slot.\n\nDev: sprint carryover, the sync test flake, timebox it to Wednesday.',
    },
    {
      id: 'apartment-repairs',
      title: 'Apartment repairs',
      body: 'Hallway light flickers. The landlord says an electrician can come Thursday morning.\n\nKitchen faucet: Kitchen Co. quote is $240 with labour. Compare one more before Friday.',
    },
    {
      id: 'reading-notes',
      title: 'Thinking in Systems',
      body: 'Ch. 5, stocks and flows: a stock is what you can measure at a moment, and flows are what change it.\n\nWrite one paragraph on the stock in my own week that I keep ignoring.',
    },
  ];

  window.CALENDAR_DATA = { calendars, items, notes };
})();
