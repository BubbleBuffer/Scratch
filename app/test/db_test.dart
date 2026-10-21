// Database behaviour on the real core: property and schema editing, cell
// editing, board drags, calendar and timeline moves, filters, sorts and
// groups, row open and peek, templates and the inline database.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/ui/database/config.dart';
import 'package:notes_app/addons/database/database_view.dart';
import 'package:notes_app/ui/database/values.dart';

import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;

import 'db_harness.dart';

Future<List<PropDef>> schemaOf(WidgetTester tester) async {
  await settle(tester);
  return core(tester, (v) => v.run((x) => x.getSchema(collection: db.projects)));
}

Future<ViewDef> viewNamed(WidgetTester tester, String name) async {
  await settle(tester);
  final vs = await core(tester, (v) => v.run((x) => x.listViews(collection: db.projects)));
  return vs.firstWhere((v) => v.name == name);
}

Future<PropValue> valueOf(WidgetTester tester, String row, String prop) async {
  final r = await rowNamed(tester, row);
  return r.valueOf(db.schema[prop]!);
}

Future<void> escape(WidgetTester tester) => dismiss(tester);

void main() {
  setUpAll(initCore);

  group('properties and schema', () {
    dbTest('rename a property from its column menu', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'col-Estimate');
      await typeInto(tester, 'prop-name', 'Points');
      await escape(tester);
      final s = await schemaOf(tester);
      expect(s.map((d) => d.name), contains('Points'));
      expect(s.map((d) => d.name), isNot(contains('Estimate')));
      expect(byKey('col-Points'), findsOneWidget);
    });

    dbTest('add a property from the table header and change its type', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tester.dragFrom(visibleCenter(tester, byKey('table-rows')), const Offset(-1200, 0));
      await settle(tester);
      await tapKey(tester, 'table-add-prop');
      await tapKey(tester, 'type-number');
      var s = await schemaOf(tester);
      final added = s.firstWhere((d) => d.kind == PropKind.number && d.name != 'Estimate');
      expect((await viewNamed(tester, 'Table')).visibleProps, contains(added.key));
      // The new property's menu opened in edit mode: change its type.
      await tapKey(tester, 'prop-type');
      await tapKey(tester, 'type-select');
      s = await schemaOf(tester);
      expect(s.firstWhere((d) => d.key == added.key).kind, PropKind.select);
    });

    dbTest('hide and delete a property', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'col-Priority');
      await tapKey(tester, 'prop-hide');
      expect((await viewNamed(tester, 'Table')).visibleProps, isNot(contains(db.schema['Priority']!.key)));
      expect(byKey('col-Priority'), findsNothing);
      await tapKey(tester, 'col-Tags');
      await tapKey(tester, 'prop-edit');
      await tapKey(tester, 'edit-delete');
      final s = await schemaOf(tester);
      expect(s.map((d) => d.name), isNot(contains('Tags')));
    });

    dbTest('add an option to a select property', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'cell-Pricing page-Tags');
      await typeInto(tester, 'option-query', 'Urgent');
      await submit(tester);
      await escape(tester);
      final s = await schemaOf(tester);
      final tags = s.firstWhere((d) => d.name == 'Tags');
      final urgent = tags.options.firstWhere((o) => o.name == 'Urgent');
      final v = await valueOf(tester, 'Pricing page', 'Tags');
      expect(v.ids, [urgent.id]);
    });

    dbTest('page properties: edit a value and hide empty ones', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      // Open the row as a sheet.
      await hover(tester, byKey('title-Pricing page'));
      await tapKey(tester, 'open-Pricing page');
      expect(byKey('page-properties'), findsOneWidget);
      expect(byKey('prop-label-Phone'), findsOneWidget);
      await tapKey(tester, 'prop-value-Approved');
      expect((await valueOf(tester, 'Pricing page', 'Approved')).checked, isTrue);
      // "Hide when empty" on Phone moves it behind "1 more property".
      await tapKey(tester, 'prop-label-Phone');
      await tapKey(tester, 'vis-${PropVisibility.hideEmpty.name}');
      await escape(tester);
      expect(byKey('prop-label-Phone'), findsNothing);
      expect(byKey('props-more'), findsOneWidget);
      await tapKey(tester, 'props-more');
      expect(byKey('prop-label-Phone'), findsOneWidget);
    });
  });

  group('cells', () {
    dbTest('edit a number cell', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'cell-API migration-Estimate');
      await typeInto(tester, 'cell-editor', '42');
      await escape(tester);
      expect((await valueOf(tester, 'API migration', 'Estimate')).number, 42);
      expect(find.text('42'), findsWidgets);
    });

    dbTest('pick a select option and a date', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'cell-Help center-Priority');
      await tapKey(tester, 'opt-${db.optionId('Priority', 'High')}');
      expect((await valueOf(tester, 'Help center', 'Priority')).text, db.optionId('Priority', 'High'));
      await tapKey(tester, 'cell-Help center-Due');
      await tapKey(tester, 'day-2026-10-30');
      await escape(tester);
      final v = await valueOf(tester, 'Help center', 'Due');
      expect(Day.ofMs(v.startMs, v.hasTime), const Day(2026, 10, 30));
    });

    dbTest('rename a row from its title cell', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'title-Team offsite');
      await typeInto(tester, 'cell-editor', 'Team retreat');
      await escape(tester);
      final rows = await rowsOf(tester, db.projects);
      expect(rows.map((r) => r.title), contains('Team retreat'));
    });
  });

  group('board', () {
    dbTest('drag a card to another group sets the property', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Board');
      await dragTo(tester, byKey('card-Quarterly report'), tester.getCenter(byKey('board-new-In progress')));
      final v = await valueOf(tester, 'Quarterly report', 'Status');
      expect(v.text, db.optionId('Status', 'In progress'));
      expect(find.descendant(of: byKey('board-col-In progress'), matching: byKey('card-Quarterly report')), findsOneWidget);
    });

    dbTest('group the board by another property', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Board');
      await tapKey(tester, 'db-settings');
      await tapKey(tester, 'settings-group');
      await tapKey(tester, 'group-by-Priority');
      await escape(tester);
      expect((await viewNamed(tester, 'Board')).groupBy, db.schema['Priority']!.key);
      expect(byKey('board-col-High'), findsOneWidget);
      expect(byKey('board-col-Not started'), findsNothing);
    });

    dbTest('new card in a column gets the group value', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Board');
      await tapKey(tester, 'board-new-Done');
      await escape(tester);
      await closeRowSheet(tester);
      final rows = await rowsOf(tester, db.projects);
      expect(rows.length, projectRows.length + 1);
      final added = rows.firstWhere((r) => !projectRows.contains(r.title));
      expect(added.valueOf(db.schema['Status']!).text, db.optionId('Status', 'Done'));
    });
  });

  group('calendar and timeline', () {
    dbTest('drag an entry to another day reschedules it', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Calendar');
      await dragTo(tester, byKey('entry-Quarterly report'), tester.getCenter(byKey('day-2026-10-9')));
      final v = await valueOf(tester, 'Quarterly report', 'Due');
      expect(Day.ofMs(v.startMs, v.hasTime), const Day(2026, 10, 9));
      expect(v.endMs, isNull);
    });

    dbTest('moving a range keeps its length', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Calendar');
      // Mobile app launch runs Oct 4 - 8; grab it on its first day.
      final bar = byKey('entry-Mobile app launch');
      final grab = tester.getTopLeft(bar) + const Offset(30, 10);
      final g = await tester.startGesture(grab);
      await tester.pump(const Duration(milliseconds: 50));
      final to = tester.getCenter(byKey('day-2026-10-11'));
      for (var i = 1; i <= 10; i++) {
        await g.moveTo(Offset.lerp(grab, to, i / 10)!);
        await tester.pump(const Duration(milliseconds: 16));
      }
      await g.up();
      await settle(tester);
      final v = await valueOf(tester, 'Mobile app launch', 'Due');
      expect(Day.ofMs(v.startMs, false), const Day(2026, 10, 11));
      expect(Day.ofMs(v.endMs!, false), const Day(2026, 10, 15));
    });

    dbTest('move a timeline bar by two days', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Timeline');
      final bar = byKey('bar-Design system');
      final start = tester.getCenter(bar);
      // Month zoom: 40 px per day.
      await tester.dragFrom(start, const Offset(80, 0));
      await settle(tester);
      final v = await valueOf(tester, 'Design system', 'Due');
      expect(Day.ofMs(v.startMs, false), const Day(2026, 10, 14));
      expect(Day.ofMs(v.endMs!, false), const Day(2026, 10, 19));
    });

    dbTest('resize a timeline bar from its end', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Timeline');
      await hover(tester, byKey('bar-Design system'));
      await tester.dragFrom(tester.getCenter(byKey('bar-end-Design system')), const Offset(120, 0));
      await settle(tester);
      final v = await valueOf(tester, 'Design system', 'Due');
      expect(Day.ofMs(v.startMs, false), const Day(2026, 10, 12));
      expect(Day.ofMs(v.endMs!, false), const Day(2026, 10, 20));
    });

    dbTest('calendar queries only the visible month', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Calendar');
      final st = dbState(tester).store;
      expect(st.rangeFrom, lessThanOrEqualTo(const Day(2026, 9, 27).ms));
      expect(st.rangeTo, lessThan(const Day(2026, 11, 15).ms));
      await tapKey(tester, 'calendar-next');
      expect(st.rangeFrom, greaterThan(const Day(2026, 10, 20).ms));
      expect(byKey('entry-Help center'), findsNothing);
    });
  });

  group('filters, sorts and search', () {
    dbTest('a status filter narrows the table', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'db-filter');
      await tapKey(tester, 'pick-Status');
      await tapKey(tester, 'filter-option-Done');
      await escape(tester);
      final st = dbState(tester).store;
      expect(st.main.total, 4);
      expect(byKey('row-Website redesign'), findsOneWidget);
      expect(byKey('row-Quarterly report'), findsNothing);
      expect(Filters.count((await viewNamed(tester, 'Table')).filter), 1);
    });

    dbTest('advanced filter with an Or', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'db-filter');
      await tapKey(tester, 'pick-Status');
      await tapKey(tester, 'filter-option-Done');
      await tapKey(tester, 'rule-more');
      await tapKey(tester, 'rule-advanced');
      await tapKey(tester, 'adv-add-rule');
      await tapKey(tester, 'adv-join-0');
      var f = (await viewNamed(tester, 'Table')).filter!;
      expect(f.kind, FilterKind.or);
      expect(f.children.length, 2);
      // The new rule has no value yet, so it is ignored.
      expect(dbState(tester).store.main.total, 4);
      await tapKey(tester, 'adv-remove-1');
      f = (await viewNamed(tester, 'Table')).filter!;
      expect(f.children.length, 1);
      expect(dbState(tester).store.main.total, 4);
      await escape(tester);
      expect(byKey('bar-advanced'), findsOneWidget);
    });

    dbTest('sort descending by due date', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'col-Due');
      await tapKey(tester, 'prop-sort-desc');
      final st = dbState(tester).store;
      expect(st.main.rowAt(0)!.title, 'Help center');
      expect((await viewNamed(tester, 'Table')).sorts.single.descending, isTrue);
      expect(byKey('bar-sort'), findsOneWidget);
    });

    dbTest('search narrows by title', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'db-search');
      await tester.enterText(byKey('db-search-field'), 'page');
      await settle(tester);
      expect(dbState(tester).store.main.total, 1);
      expect(byKey('row-Pricing page'), findsOneWidget);
    });
  });

  group('views', () {
    dbTest('add, rename, duplicate and delete a view', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await tapKey(tester, 'add-view');
      await tapKey(tester, 'layout-board');
      await escape(tester);
      var vs = await core(tester, (v) => v.run((x) => x.listViews(collection: db.projects)));
      expect(vs.length, 7);
      final added = vs.last;
      expect(added.kind, ViewKind.board);
      expect(added.groupBy, db.schema['Status']!.key);
      await tapKey(tester, 'tab-${added.name}');
      await tester.enterText(byKey('view-name'), 'Kanban');
      await press(tester, LogicalKeyboardKey.enter);
      await escape(tester);
      vs = await core(tester, (v) => v.run((x) => x.listViews(collection: db.projects)));
      expect(vs.map((v) => v.name), contains('Kanban'));
      await tapKey(tester, 'tab-Kanban');
      await tapKey(tester, 'view-duplicate');
      vs = await core(tester, (v) => v.run((x) => x.listViews(collection: db.projects)));
      expect(vs.length, 8);
      await tapKey(tester, 'tabs-more');
      await settle(tester);
      if (byKey('more-view-Kanban').evaluate().isNotEmpty) await tapKey(tester, 'more-view-Kanban');
      await tapKey(tester, 'tab-Kanban');
      await tapKey(tester, 'view-delete');
      vs = await core(tester, (v) => v.run((x) => x.listViews(collection: db.projects)));
      expect(vs.length, 7);
    });

    dbTest('change a view layout keeps its settings', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'List');
      await tapKey(tester, 'db-settings');
      await tapKey(tester, 'settings-layout');
      await tapKey(tester, 'layout-gallery');
      await escape(tester);
      final v = await viewNamed(tester, 'List');
      expect(v.kind, ViewKind.gallery);
      expect(byKey('gallery-rows'), findsOneWidget);
    });

    dbTest('mobile view switcher', (tester) async {
      await pumpDb(tester, mobile: true);
      await openView(tester, 'Calendar', mobile: true);
      expect(byKey('mday-2026-10-14'), findsOneWidget);
      await tapKey(tester, 'view-switcher');
      await tapKey(tester, 'sheet-view-List');
      if (byKey('views-done').evaluate().isNotEmpty) await tapKey(tester, 'views-done');
      expect(byKey('list-rows'), findsOneWidget);
    });
  });

  group('rows', () {
    dbTest('a row opens as a sheet on top of its database', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      await hover(tester, byKey('title-Quarterly report'));
      await tapKey(tester, 'open-Quarterly report');
      final row = await rowNamed(tester, 'Quarterly report');
      final stack = nav.PageStackScope.of(tester.element(find.byType(DatabaseView).first));
      expect(stack.pages.map(stack.idOf), [db.projects, row.id]);
      // No peek: nothing narrows the database or sits over the sheet.
      expect(byKey('side-peek'), findsNothing);
      expect(byKey('center-peek'), findsNothing);
      expect(byKey('peek-next'), findsNothing);
      final sheet = tester.getRect(byKey('stack-sheet-${row.id}'));
      expect(sheet.left, 240 + 12 + 58);
      expect(sheet.top, 12);
      expect(sheet.right, 1440);
      // The row sheet's breadcrumb leads back to its database.
      expect(find.descendant(of: byKey('stack-sheet-${row.id}'), matching: byKey('crumb-${db.projects}')), findsOneWidget);
      await tapKey(tester, 'crumb-${db.projects}');
      expect(stack.pages.map(stack.idOf), [db.projects]);
    });

    dbTest('every view opens rows as sheets, whatever "Open in" says', (tester) async {
      await pumpDb(tester, mobile: false);
      final row = await rowNamed(tester, 'Pricing page');
      for (final (view, key) in [
        ('Board', 'card-Pricing page'),
        ('Gallery', 'card-Pricing page'),
        ('List', 'list-Pricing page'),
      ]) {
        await openView(tester, view);
        await tapKey(tester, key);
        final stack = nav.PageStackScope.of(tester.element(find.byType(DatabaseView).first));
        expect(stack.topId, row.id, reason: view);
        await closeRowSheet(tester);
      }
    });

    dbTest('a card opens the row as a full page on mobile', (tester) async {
      await pumpDb(tester, mobile: true);
      await openView(tester, 'Board', mobile: true);
      await tapKey(tester, 'card-Quarterly report');
      expect(byKey('side-peek'), findsNothing);
      expect(byKey('page-properties'), findsOneWidget);
      expect(find.text('Quarterly report'), findsWidgets);
    });

    dbTest('desktop view settings leave out "Open pages in"', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Gallery');
      await tapKey(tester, 'db-settings');
      expect(byKey('settings-agenda'), findsOneWidget);
      expect(byKey('settings-open-in'), findsNothing);
      await escape(tester);
    });

    dbTest('New uses the default template', (tester) async {
      await pumpDb(tester, mobile: false);
      await openView(tester, 'Table');
      // A template with a status and a block of content.
      final store = dbState(tester).store;
      final t = await tester.runAsync(() => store.newTemplate());
      await core(tester, (v) async {
        await v.renamePage(t!, 'Weekly task');
        await v.run((x) => x.setProperty(page: t, key: db.schema['Status']!.key, value: Values.status(db.optionId('Status', 'In progress'))),
            writes: true);
        await v.apply(t, [
          BOp(op: OpKind.insert, id: newId(), kind: Kinds.todo, text: 'Write the summary', marks: const [], checked: false),
        ]);
      });
      await settle(tester);
      await tapKey(tester, 'db-new-menu');
      expect(byKey('template-default'), findsOneWidget);
      expect(find.text('Weekly task'), findsWidgets);
      await escape(tester);
      await tapKey(tester, 'db-new');
      await closeRowSheet(tester);
      final rows = await rowsOf(tester, db.projects);
      expect(rows.length, projectRows.length + 1);
      final added = rows.firstWhere((r) => !projectRows.contains(r.title));
      expect(added.valueOf(db.schema['Status']!).text, db.optionId('Status', 'In progress'));
      final page = await core(tester, (v) => v.loadPage(added.id));
      expect(page.blocks.map((b) => b.text), contains('Write the summary'));
    });
  });

  group('inline database', () {
    dbTest('shows inside a page and adds rows', (tester) async {
      await pumpDb(tester, mobile: false);
      await openPage(tester, 'Planning');
      expect(find.text('Notes after the database.'), findsOneWidget);
      expect(byKey('row-Website redesign'), findsOneWidget);
      await tapKey(tester, 'table-new');
      await settle(tester);
      final rows = await rowsOf(tester, db.projects);
      expect(rows.length, projectRows.length + 1);
      // The new row's title is being edited.
      expect(byKey('cell-editor'), findsOneWidget);
      await escape(tester);
      await closeRowSheet(tester);
      // Switch the inline database to its board.
      await tapKey(tester, 'tab-Board');
      expect(byKey('board-col-Done'), findsOneWidget);
    });

    dbTest('turns a new page into a full-page database', (tester) async {
      await pumpDb(tester, mobile: false);
      final id = await core(tester, (v) => v.createPage(title: ''));
      await settle(tester);
      nav.openPage(tester.element(find.byType(HomeScreen)), id, '', isNew: true);
      await settle(tester);
      await tapKey(tester, 'start-database');
      await settleUntil(tester, () => byKey('table-new').evaluate().isNotEmpty);
      final schema = await core(tester, (v) => v.run((x) => x.getSchema(collection: id)));
      expect(schema.map((d) => d.name), contains('Tags'));
      expect(byKey('start-database'), findsNothing);
      await tapKey(tester, 'table-new');
      expect(await rowsOf(tester, id), hasLength(1));
    });
  });
}
