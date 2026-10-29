// A fresh vault's introduction and the two starter organizer databases.

import 'dart:convert';

import '../core/clock.dart';
import '../core/models.dart';
import '../core/vault.dart';
import '../ui/database/values.dart';

const gettingStartedTitle = 'Getting started';

BOp _block(String id, String? after, String kind, String text, {bool checked = false}) =>
    BOp(op: OpKind.insert, id: id, after: after, kind: kind, text: text, marks: const [], checked: checked);

List<BOp> gettingStartedOps() {
  final spec = <(String, String, bool)>[
    (Kinds.paragraph, 'Everything here is saved on this device as you type.', false),
    (Kinds.heading2, 'The basics', false),
    (Kinds.todo, 'Click anywhere and just start typing', false),
    (Kinds.todo, 'Type / for headings, lists, to-dos, quotes, code and dividers', false),
    (Kinds.todo, 'Markdown works too: # for a heading, - for a list, [] for a to-do', false),
    (Kinds.todo, 'Create a page with New page in the sidebar, or Ctrl+N', false),
    (Kinds.todo, 'Put pages inside pages with the + next to a page in the sidebar', false),
    (Kinds.heading2, 'Link your pages', false),
    (Kinds.todo, 'Type @ or [[ and a page name to link it.', false),
    (Kinds.todo, 'Pages that link here are listed at the bottom.', false),
    (Kinds.todo, 'Pages open on top of each other. Click the edge on the left, or press Esc, to go back.', false),
    (Kinds.heading2, 'Pages inside pages', false),
    (Kinds.paragraph, 'Any page can hold other pages. Tasks and Calendar are pages too: each task and event is a page inside them.', false),
    (Kinds.todo, 'Type /view to list the pages inside a page. Choose List, Table, Board or Calendar in the view’s menu.', false),
    (Kinds.todo, 'Switch between a page’s views with the tabs above them.', false),
    (Kinds.todo, 'Add a property such as Status or Due to one page, then add it to the pages next to it.', false),
    (Kinds.heading2, 'Plan your day with Today', false),
    (
      Kinds.paragraph,
      'Home brings together your events, tasks due and overdue, and checklists in progress. Select a day in the week strip to see its plan.',
      false,
    ),
    (
      Kinds.paragraph,
      'Give any to-do a date by typing @today or @tomorrow, or choose Set date in its block menu. It will appear in Today alongside Tasks and Calendar.',
      false,
    ),
    (
      Kinds.paragraph,
      'Use the Add a task or note field on Home to capture into today’s daily note. Choose Task for a dated to-do, or Note for a paragraph. Open, under the field, takes you to that page.',
      false,
    ),
  ];
  final ops = <BOp>[];
  String? after;
  for (final (kind, text, checked) in spec) {
    final id = newId();
    ops.add(_block(id, after, kind, text, checked: checked));
    after = id;
  }
  return ops;
}

PropDef _def(String name, PropKind kind, {List<SelectOption> options = const [], String config = ''}) =>
    PropDef(key: '', name: name, kind: kind, options: options, config: config);

/// The starter Tasks schema and views, also used by the demo seed.
Future<String> createTasksDatabase(Vault vault, {String? parent, bool sampleRows = true}) async {
  final id = await vault.createPage(parent: parent, title: 'Tasks');
  await vault.run((v) => v.makeCollection(page: id), writes: true);
  final options = [
    SelectOption(id: newId(), name: 'To do', color: 'gray'),
    SelectOption(id: newId(), name: 'Doing', color: 'blue'),
    SelectOption(id: newId(), name: 'Done', color: 'green'),
  ];
  final schema = await vault.run(
    (v) => v.setSchema(
      collection: id,
      schema: [
        _def(
          'Status',
          PropKind.status,
          options: options,
          config: jsonEncode({
            'groups': {options[0].id: 'todo', options[1].id: 'inProgress', options[2].id: 'complete'},
          }),
        ),
        _def('Due', PropKind.date),
      ],
    ),
    writes: true,
  );
  final status = schema.firstWhere((d) => d.name == 'Status');
  final due = schema.firstWhere((d) => d.name == 'Due');
  final views = await vault.run((v) => v.listViews(collection: id));
  await vault.run(
    (v) => v.updateView(
      collection: id,
      view: views.first.copyWith(name: 'Table', visibleProps: [status.key, due.key]),
    ),
    writes: true,
  );
  await vault.run(
    (v) => v.addView(
      collection: id,
      view: ViewDef(id: '', name: 'Board', kind: ViewKind.board, sorts: const [], groupBy: status.key, visibleProps: [due.key], config: ''),
    ),
    writes: true,
  );
  await vault.setIcon(id, '☑️');
  if (sampleRows) {
    final today = Day.of(clockNow());
    for (final (title, delta) in const [('Plan today', 0), ('Make time for tomorrow', 1)]) {
      final row = await vault.createPage(parent: id, title: title);
      await vault.run((v) => v.setProperty(page: row, key: status.key, value: Values.status(status.options.first.id)), writes: true);
      await vault.run((v) => v.setProperty(page: row, key: due.key, value: Values.date(today.add(delta).ms)), writes: true);
    }
  }
  return id;
}

/// Calendar comes first so opening the starter database shows its events.
Future<String> createCalendarDatabase(Vault vault, {String? parent, bool sampleRows = true}) async {
  final id = await vault.createPage(parent: parent, title: 'Calendar');
  await vault.run((v) => v.makeCollection(page: id), writes: true);
  final schema = await vault.run((v) => v.setSchema(collection: id, schema: [_def('When', PropKind.date)]), writes: true);
  final when = schema.single.key;
  final views = await vault.run((v) => v.listViews(collection: id));
  await vault.run(
    (v) => v.updateView(
      collection: id,
      view: views.first.copyWith(name: 'Table', visibleProps: [when]),
    ),
    writes: true,
  );
  final calendar = await vault.run(
    (v) => v.addView(
      collection: id,
      view: ViewDef(id: '', name: 'Calendar', kind: ViewKind.calendar, sorts: const [], dateProp: when, visibleProps: const [], config: ''),
    ),
    writes: true,
  );
  await vault.run((v) => v.moveView(collection: id, viewId: calendar, index: 0), writes: true);
  await vault.setIcon(id, '📅');
  if (sampleRows) {
    final now = clockNow();
    final row = await vault.createPage(parent: id, title: 'A little time for yourself');
    final start = DateTime(now.year, now.month, now.day, 18);
    await vault.run(
      (v) => v.setProperty(page: row, key: when, value: Values.date(start.millisecondsSinceEpoch, hasTime: true)),
      writes: true,
    );
  }
  return id;
}

/// [createTasks] is false only when SEED_TASKS already supplied that database.
Future<String> createGettingStarted(Vault vault, {bool createTasks = true}) async {
  final id = await vault.createPage(title: gettingStartedTitle);
  await vault.apply(id, gettingStartedOps());
  if (createTasks) await createTasksDatabase(vault);
  await createCalendarDatabase(vault);
  return id;
}

/// Boot-time starter pages for a fresh vault. Without a hub they are created
/// now; with one configured they wait for [seedStarterPagesAfterPull].
Future<bool> seedStarterPagesAtBoot(Vault vault, {required bool syncing, bool createTasks = true}) async {
  if (syncing) return false;
  await createGettingStarted(vault, createTasks: createTasks);
  await vault.flush();
  return true;
}

/// Whether the sidebar shows any page. Hidden system pages (nodes, calendars,
/// feedback) are not top-level, and device node records are children of the
/// nodes page, so none of them count.
Future<bool> _hasVisiblePages(Vault vault) async => (await vault.listChildren(null, limit: 1)).isNotEmpty;

/// Creates the starter pages once [firstPull] succeeds, and only if the vault
/// still shows no pages, so synced pages are never joined by a second set. A
/// pull that fails seeds nothing this launch.
Future<bool> seedStarterPagesAfterPull(Vault vault, Future<bool> firstPull, {bool createTasks = true}) async {
  if (!await firstPull) return false;
  await vault.flush();
  if (await _hasVisiblePages(vault)) return false;
  await createGettingStarted(vault, createTasks: createTasks);
  await vault.flush();
  return true;
}
