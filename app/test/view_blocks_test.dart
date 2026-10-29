// Pages with view blocks: a page lists its child pages through view blocks
// shown as tabs under its body. Former databases open as pages, and adding
// a property to a child page asks before every sibling gets it.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/database/database_view.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/features/getting_started.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/shell/addons.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;

import 'db_harness.dart';

ViewDef _view(String name, ViewKind kind) =>
    ViewDef(id: '', name: name, kind: kind, sorts: const [], visibleProps: const [], config: '');

/// A plain page "Trip" with the child pages Day 1 and Day 2 and the view
/// blocks [views] (ids v1, v2, ...) after a paragraph. Returns its id.
Future<String> _trip(WidgetTester tester, List<ViewDef> views) => core(tester, (v) async {
      final trip = await v.createPage(title: 'Trip');
      for (final t in ['Day 1', 'Day 2']) {
        await v.createPage(parent: trip, title: t);
      }
      await v.apply(trip, [
        BOp(op: OpKind.insert, id: 'p1', kind: Kinds.paragraph, text: 'Plans for the trip.', marks: const [], checked: false),
      ]);
      var after = 'p1';
      for (var i = 0; i < views.length; i++) {
        final id = 'v${i + 1}';
        await v.run((x) => x.insertViewBlock(pageId: trip, id: id, after: after, view: views[i]), writes: true);
        after = id;
      }
      return trip;
    });

Future<void> _open(WidgetTester tester, String id, String title) async {
  nav.openPage(tester.element(find.byType(HomeScreen)), id, title);
  await settle(tester);
}

Future<List<PropDef>> _schema(WidgetTester tester, String page) =>
    core(tester, (v) => v.run((x) => x.getSchema(collection: page)));

Future<String> _child(WidgetTester tester, String parent, String title) async {
  final rows = await core(tester, (v) => v.listChildren(parent));
  return rows.firstWhere((p) => p.title == title).id;
}

void main() {
  setUpAll(initCore);

  test('the / menu offers View and no Database', () {
    registerBuiltins();
    final items = blockRegistry.filter('');
    final view = items.where((t) => t.menuLabel == 'View').single;
    expect(view.hint, 'List the pages inside this page.');
    expect(items.map((t) => t.menuLabel), isNot(contains('Database')));
    expect(blockRegistry.filter('database').map((t) => t.menuLabel), isNot(contains('Database')));
  });

  dbTest('a view block lists the child pages of a plain page', (tester) async {
    await pumpDb(tester, mobile: false);
    final trip = await _trip(tester, [_view('List', ViewKind.list)]);
    await _open(tester, trip, 'Trip');
    expect(find.text('Plans for the trip.'), findsOneWidget);
    expect(byKey('tab-List'), findsOneWidget);
    expect(byKey('add-view'), findsOneWidget);
    expect(byKey('list-Day 1'), findsOneWidget);
    expect(byKey('list-Day 2'), findsOneWidget);
    expect(await _schema(tester, trip), isEmpty);
    // "+ New page" under the view makes a child of the page.
    await tapKey(tester, 'list-new');
    final children = await core(tester, (v) => v.listChildren(trip));
    expect(children, hasLength(3));
  });

  dbTest('adjacent view blocks are one group of tabs', (tester) async {
    await pumpDb(tester, mobile: false);
    final trip = await _trip(tester, [_view('List', ViewKind.list), _view('Table', ViewKind.table)]);
    await _open(tester, trip, 'Trip');
    expect(find.byType(DatabaseView), findsOneWidget);
    expect(byKey('tab-List'), findsOneWidget);
    expect(byKey('tab-Table'), findsOneWidget);
    expect(byKey('list-Day 1'), findsOneWidget);
    await tapKey(tester, 'tab-Table');
    expect(byKey('title-Day 1'), findsOneWidget);
    expect(byKey('list-Day 1'), findsNothing);
    // Remembered per group, by block id.
    expect(viewBlockTabs['v1'], 'v2');
  });

  dbTest('changing a layout writes the block text', (tester) async {
    await pumpDb(tester, mobile: false);
    final trip = await _trip(tester, [_view('List', ViewKind.list)]);
    await _open(tester, trip, 'Trip');
    await tapKey(tester, 'tab-List');
    await tapKey(tester, 'view-layout-table');
    final views = await viewBlocksOf(tester, trip);
    expect(views.single.kind, ViewKind.table);
    expect(await core(tester, (v) => v.run((x) => x.listViews(collection: trip))), isEmpty);
    expect(byKey('title-Day 1'), findsOneWidget);
  });

  dbTest('"+ Add view" adds a List view block after the group', (tester) async {
    await pumpDb(tester, mobile: false);
    final trip = await _trip(tester, [_view('Table', ViewKind.table)]);
    await _open(tester, trip, 'Trip');
    await tapKey(tester, 'add-view');
    final views = await viewBlocksOf(tester, trip);
    expect(views.map((v) => v.kind), [ViewKind.table, ViewKind.list]);
    expect(byKey('list-Day 1'), findsOneWidget);
  });

  dbTest('adding a property asks, and Add gives it to every page under the parent', (tester) async {
    await pumpDb(tester, mobile: false);
    final trip = await _trip(tester, [_view('Table', ViewKind.table)]);
    final day1 = await _child(tester, trip, 'Day 1');
    await _open(tester, day1, 'Day 1');
    await tapKey(tester, 'props-add');
    await tapKey(tester, 'type-${PropKind.text.name}');
    expect(find.text('Add Text to all 2 pages under Trip?'), findsOneWidget);
    expect(find.text('They get an empty Text, and new pages under Trip start with it.'), findsOneWidget);
    await tapKey(tester, 'confirm-ok');
    await dismiss(tester);
    final schema = await _schema(tester, trip);
    expect(schema.map((d) => d.name), ['Text']);
    // The parent's view shows it; it has no saved views to migrate.
    final views = await viewBlocksOf(tester, trip);
    expect(views.single.visibleProps, contains(schema.single.key));
    expect(views, hasLength(1));
    expect(await core(tester, (v) => v.run((x) => x.listViews(collection: trip))), isEmpty);
    expect(byKey('prop-label-Text'), findsOneWidget);
  });

  dbTest('Cancel in the add-property question adds nothing', (tester) async {
    await pumpDb(tester, mobile: false);
    final trip = await _trip(tester, [_view('Table', ViewKind.table)]);
    final day1 = await _child(tester, trip, 'Day 1');
    await _open(tester, day1, 'Day 1');
    await tapKey(tester, 'props-add');
    await tapKey(tester, 'type-${PropKind.text.name}');
    await tapKey(tester, 'confirm-cancel');
    expect(await _schema(tester, trip), isEmpty);
    final page = await core(tester, (v) => v.run((x) => x.loadPage(id: trip)));
    expect(page.pageType, documentType);
    expect(byKey('prop-label-Text'), findsNothing);
  });

  dbTest('a top-level page offers no property', (tester) async {
    await pumpDb(tester, mobile: false);
    final trip = await _trip(tester, const []);
    await _open(tester, trip, 'Trip');
    expect(byKey('props-add'), findsNothing);
  });

  dbTest('a former database opens as a page with its body and tabs', (tester) async {
    await pumpDb(tester, mobile: false);
    await core(tester, (v) => v.apply(db.projects, [
          BOp(op: OpKind.insert, id: 'intro', kind: Kinds.paragraph, text: 'What we are building.', marks: const [], checked: false),
        ]));
    await openPage(tester, projectsTitle);
    expect(find.byWidgetPredicate((w) => w is PageScreen && w.pageId == db.projects), findsOneWidget);
    expect(find.text('What we are building.'), findsOneWidget);
    expect(byKey('tab-Table'), findsOneWidget);
    expect(byKey('tab-Board'), findsOneWidget);
    final editor = tester.state<PageEditorState>(
      find.byWidgetPredicate((w) => w is PageScreen && w.pageId == db.projects),
    );
    expect(editor.doc!.byId.values.where((n) => n.kind == 'view'), isNotEmpty);
  });

  dbTest('a database page without view blocks lists its children', (tester) async {
    await pumpDb(tester, mobile: false);
    final reading = await core(tester, (Vault v) async {
      final id = await v.createPage(title: 'Reading');
      await v.run((x) => x.makeCollection(page: id), writes: true);
      for (final view in await v.run((x) => x.listViews(collection: id))) {
        await v.run((x) => x.deleteView(collection: id, viewId: view.id), writes: true);
      }
      await v.createPage(parent: id, title: 'Book A');
      return id;
    });
    await _open(tester, reading, 'Reading');
    expect(byKey('list-Book A'), findsOneWidget);
    expect(byKey('page-title'), findsWidgets);
    // Not saved as a block.
    expect(await viewBlocksOf(tester, reading), isEmpty);
  });

  test('Getting started has no Databases heading', () async {
    final dir = await Directory.systemTemp.createTemp('notes_gs_');
    final vault = await Vault.open('${dir.path}/vault');
    try {
      final id = await createGettingStarted(vault);
      final page = await vault.run((v) => v.loadPage(id: id));
      expect(
        page.blocks.where((b) => b.kind.startsWith('heading') && b.text.trim() == 'Databases'),
        isEmpty,
      );
    } finally {
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });
}
