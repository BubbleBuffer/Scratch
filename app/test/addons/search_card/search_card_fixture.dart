// Shared set-up for the search card tests and the golden shots: seeds pages
// and query blocks through the real core and opens pages in the app.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/search_card/query_card.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/services/addon_services.dart' show openPage;
import 'package:notes_app/src/rust/api/vault.dart';
import 'package:notes_app/ui/database/values.dart';

import '../../addon_harness.dart';

export '../../addon_harness.dart';

Finder inCard(String blockId) => find.byKey(ValueKey('query-card-$blockId'));

/// The page type and add-on id of a task. The card's tests do not import the
/// Tasks add-on (the import rule), so they write its status directly.
const taskPageType = 'task';
const _tasksAddon = 'tasks';

/// Makes [page] a task that is not done yet, as the Tasks add-on stores it.
Future<void> makeTask(WidgetTester tester, String page) => core(
  tester,
  (v) => v.typeDataSet(page, _tasksAddon, const ['status'], jsonEncode('todo')),
);

/// A task page tagged #work with [status] ("todo" or "done"); its id.
Future<String> seedWorkTask(
  WidgetTester tester,
  String title, {
  String status = 'todo',
}) async {
  final task = await seedPage(tester, title, pageType: taskPageType);
  await setTaskStatus(tester, task, status);
  await core(
    tester,
    (v) => v.run(
      (x) => x.setProperty(
        page: task,
        key: 'tags',
        value: Values.multi(['work']),
      ),
      writes: true,
    ),
  );
  return task;
}

/// Sets [page]'s task status, as the Tasks add-on stores it ("todo" or "done").
Future<void> setTaskStatus(WidgetTester tester, String page, String status) =>
    core(
      tester,
      (v) => v.typeDataSet(page, _tasksAddon, const ['status'], jsonEncode(status)),
    );

/// The status stored for [page]'s task, or null when it has none.
Future<String?> taskStatus(WidgetTester tester, String page) async {
  final raw = await core(tester, (v) => v.typeData(page, _tasksAddon));
  final data = jsonDecode(raw);
  return data is Map ? data['status'] as String? : null;
}

Future<String> seedPage(
  WidgetTester tester,
  String title, {
  String? pageType,
  String? body,
}) async {
  final id = await core(
    tester,
    (v) => v.createPage(title: title, pageType: pageType),
  );
  if (body != null) {
    await core(
      tester,
      (v) => v.apply(id, [
        BOp(
          op: OpKind.insert,
          id: newId(),
          kind: 'paragraph',
          text: body,
          marks: const [],
          checked: false,
        ),
      ]),
    );
  }
  return id;
}

/// A query block at the end of [page], with [query] as its text.
Future<String> seedQueryBlock(
  WidgetTester tester,
  String page,
  String query,
) async {
  final id = newId();
  await core(
    tester,
    (v) => v.apply(page, [
      BOp(
        op: OpKind.insert,
        id: id,
        kind: 'query',
        text: query,
        marks: const [],
        checked: false,
      ),
    ]),
  );
  return id;
}

Future<void> openPageNamed(WidgetTester tester, String id, String title) async {
  unawaited(openPage(tester.element(find.byType(Overlay).first), id, title));
  await settle(tester);
}

Future<String> blockText(WidgetTester tester, String page, String block) async {
  final blocks = await blocksOf(tester, page);
  return blocks.firstWhere((b) => b.id == block).text;
}

/// Opens [page] with a query card for [query] and waits for its results.
Future<String> openDashboard(
  WidgetTester tester, {
  required String query,
  String dashboard = 'Dashboard',
}) async {
  final page = await seedPage(tester, dashboard);
  final block = await seedQueryBlock(tester, page, query);
  await openPageNamed(tester, page, dashboard);
  await settleUntil(tester, () => inCard(block).evaluate().isNotEmpty);
  return block;
}

/// A bridge that records which handles are live: a handle counts from its
/// first delivered update until it is unwatched.
class RecordingBridge extends QueryBridge {
  RecordingBridge();

  final live = <BigInt>{};

  @override
  Stream<LiveQueryUpdate> watch(NotesVault raw, String query, int limit) =>
      super.watch(raw, query, limit).map((update) {
        live.add(update.handle);
        return update;
      });

  @override
  Future<void> unwatch(NotesVault raw, BigInt handle) {
    live.remove(handle);
    return super.unwatch(raw, handle);
  }
}

/// Appends [blocks] to [page] in order, as (kind, text) pairs.
Future<List<String>> appendBlocks(
  WidgetTester tester,
  String page,
  List<(String, String)> blocks,
) async {
  final ids = <String>[];
  final ops = <BOp>[];
  String? after;
  for (final (kind, text) in blocks) {
    final id = newId();
    ops.add(
      BOp(
        op: OpKind.insert,
        id: id,
        after: after,
        kind: kind,
        text: text,
        marks: const [],
        checked: false,
      ),
    );
    ids.add(id);
    after = id;
  }
  await core(tester, (v) => v.apply(page, ops));
  return ids;
}

/// The mockup's page for the screenshots: the host page "Week 42" with three
/// query cards under headings, over the mockup's sample data. The queries are
/// the real operator syntax (see wiki/architecture/search-queries.md):
///  - open work: `#work type:task -status:done`, four open task pages tagged #work;
///  - on the board: `in:[[Sprint board]] status:doing`, rows with Status;
///  - touched this week: `updated:week`, minus the host page itself.
/// [boardEmpty] moves every Doing row to Todo, for the empty-state shot.
Future<({String page, String tasks, String board, String week})>
seedSearchCardPage(WidgetTester tester, {bool boardEmpty = false}) async {
  for (final title in [
    'Draft the migration note',
    "Review Mia's index patch",
    'Book a room for the retro',
    'Renew the code-signing cert',
  ]) {
    final task = await seedPage(tester, title, pageType: taskPageType);
    await makeTask(tester, task);
    await core(
      tester,
      (v) => v.run(
        (x) => x.setProperty(
          page: task,
          key: 'tags',
          value: Values.multi(['work']),
        ),
        writes: true,
      ),
    );
  }

  final board = await seedPage(tester, 'Sprint board');
  await core(
    tester,
    (v) => v.run((x) => x.makeCollection(page: board), writes: true),
  );
  final schema = await core(
    tester,
    (v) => v.run(
      (x) => x.setSchema(
        collection: board,
        schema: const [
          PropDef(
            key: '',
            name: 'Status',
            kind: PropKind.select,
            options: [
              SelectOption(id: '', name: 'Todo', color: 'gray'),
              SelectOption(id: '', name: 'Doing', color: 'blue'),
              SelectOption(id: '', name: 'Done', color: 'green'),
            ],
            config: '',
          ),
        ],
      ),
      writes: true,
    ),
  );
  final status = schema.single;
  String optionOf(String name) =>
      status.options.firstWhere((o) => o.name == name).id;
  for (final (title, state) in [
    ('Index generation counter', 'Doing'),
    ('Feed event on commit', 'Doing'),
    ('Result stream over the bridge', 'Doing'),
    ('Write the beta notes', 'Todo'),
    ('Cut the beta branch', 'Done'),
  ]) {
    final shown = boardEmpty && state == 'Doing' ? 'Todo' : state;
    final row = await core(
      tester,
      (v) => v.createPage(parent: board, title: title),
    );
    await core(
      tester,
      (v) => v.run(
        (x) => x.setProperty(
          page: row,
          key: status.key,
          value: Values.select(optionOf(shown)),
        ),
        writes: true,
      ),
    );
  }

  // Pages touched this week (every page is, as it is created now). Some carry
  // their own icon, as a real page does; the rest show the generic one. The
  // icons are letters: the golden font has no emoji or symbol glyphs here.
  final icons = {'Sprint board': 'S', 'Lisbon trip': 'L', 'Reading list': 'R'};
  for (final title in [
    'Sync rewrite',
    'Lisbon trip',
    'Groceries',
    'Week 41',
    'Release checklist',
    'Reading list',
  ]) {
    final page = await seedPage(tester, title);
    final icon = icons[title];
    if (icon != null) {
      await core(tester, (v) => v.setIcon(page, icon));
    }
  }
  await core(tester, (v) => v.setIcon(board, icons['Sprint board']));

  final page = await seedPage(tester, 'Week 42');
  final ids = await appendBlocks(tester, page, [
    (
      Kinds.paragraph,
      'Ship the sync rewrite to beta by Friday. The lists below are searches, '
          'so they stay current as tasks and the board change.',
    ),
    (Kinds.heading2, 'Open work'),
    ('query', '#work type:task -status:done'),
    (Kinds.heading2, 'On the board'),
    ('query', 'in:[[Sprint board]] status:doing'),
    (Kinds.heading2, 'Touched this week'),
    ('query', 'updated:week -"Week 42" sort:updated'),
  ]);
  await openPageNamed(tester, page, 'Week 42');
  await settleUntil(
    tester,
    () =>
        inCard(ids[2]).evaluate().isNotEmpty &&
        inCard(ids[4]).evaluate().isNotEmpty &&
        inCard(ids[6]).evaluate().isNotEmpty,
  );
  // Rows arrive with their page facts (icon, status, task state); wait for the
  // ones the shots show.
  await settleUntil(
    tester,
    () =>
        find
            .descendant(of: inCard(ids[6]), matching: find.text('Sync rewrite'))
            .evaluate()
            .isNotEmpty &&
        find
            .descendant(
              of: inCard(ids[4]),
              matching: find.text(boardEmpty ? 'Nothing in Doing.' : 'Doing'),
            )
            .evaluate()
            .isNotEmpty,
  );
  // settleUntil gives up silently; a missing row or line fails here, loudly.
  expect(
    find.descendant(
      of: inCard(ids[4]),
      matching: find.text(boardEmpty ? 'Nothing in Doing.' : 'Doing'),
    ),
    findsWidgets,
  );
  return (page: page, tasks: ids[2], board: ids[4], week: ids[6]);
}

// ---- the inline chip (Form B) ----

/// A paragraph holding [text], with a query mark for [query] over
/// [start]..[end]. Returns the paragraph's block id.
Future<String> seedChip(
  WidgetTester tester,
  String page,
  String text, {
  required String query,
  required int start,
  required int end,
}) async {
  final id = newId();
  await core(
    tester,
    (v) => v.apply(page, [
      BOp(
        op: OpKind.insert,
        id: id,
        kind: Kinds.paragraph,
        text: text,
        marks: [
          BMark(start: start, end: end, kind: BMarkKind.query, url: query),
        ],
        checked: false,
      ),
    ]),
  );
  return id;
}

/// Opens [title] and waits until the paragraph reading [text] is drawn with
/// its chip's label in.
Future<void> openChipPage(
  WidgetTester tester,
  String page,
  String title, {
  required String text,
  required String query,
}) async {
  await openPageNamed(tester, page, title);
  await settleUntil(
    tester,
    () =>
        editableWith(text).evaluate().isNotEmpty &&
        chipLabelOf(tester, text, query) != null,
  );
}

/// Taps the chip's first character, which sits inside its mark.
Future<void> tapChip(WidgetTester tester, String text) async {
  final box = tester.getRect(editableWith(text));
  await tester.tapAt(box.topLeft + const Offset(8, 8));
  await settle(tester);
}
