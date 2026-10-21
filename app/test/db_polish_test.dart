// Database polish: a row of an inline database opening on top, breadcrumbs of
// a row page, the select editor over its cell, a new filter rule's editor
// at its chip, and an empty page while a default template is set.
//
// Run with NOTES_BRIDGE_LIB pointing at the built notes_bridge library.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/ui/database/db_icons.dart';
import 'package:notes_app/shell/navigation.dart' show PageStackScope;
import 'package:notes_app/addons/database/database_view.dart';
import 'package:notes_app/ui/database/values.dart';
import 'package:notes_app/ui/ui.dart' show MenuSurface;

import 'db_harness.dart';

Finder editableText(String text) =>
    find.byWidgetPredicate((w) => w is EditableText && w.controller.text == text);

void main() {
  setUpAll(initCore);

  dbTest('a row of an inline database opens on top at full width', (tester) async {
    await pumpDb(tester, mobile: false);
    await openPage(tester, 'Planning');
    final before = tester.getTopRight(editableText('Notes after the database.')).dx;
    await hover(tester, byKey('title-Website redesign'));
    await tapKey(tester, 'open-Website redesign');
    final row = await rowNamed(tester, 'Website redesign');
    final stack = PageStackScope.of(tester.element(find.byType(DatabaseView).first));
    expect(stack.topId, row.id);
    final sheet = tester.getRect(byKey('stack-sheet-${row.id}'));
    expect(sheet.width, greaterThanOrEqualTo(1440 - 240 - 12 - 58 - 12));
    // The page below keeps its text column; nothing squeezes it.
    expect(tester.getTopRight(editableText('Notes after the database.').first).dx, before);
    await closeRowSheet(tester);
    expect(tester.getTopRight(editableText('Notes after the database.')).dx, before);
  });

  dbTest('a row page shows its database in the breadcrumbs', (tester) async {
    await pumpDb(tester, mobile: false);
    await openView(tester, 'Table');
    await hover(tester, byKey('title-Pricing page'));
    await tapKey(tester, 'open-Pricing page');
    expect(byKey('crumb-${db.projects}'), findsOneWidget);
    expect(find.descendant(of: byKey('crumb-${db.projects}'), matching: find.text(projectsTitle)), findsOneWidget);
    // With the database glyph.
    expect(find.descendant(of: byKey('crumb-${db.projects}'), matching: find.byType(DbIcon)), findsOneWidget);
  });

  dbTest('the select editor opens over its cell', (tester) async {
    await pumpDb(tester, mobile: false);
    await openView(tester, 'Table');
    final cell = tester.getRect(byKey('cell-Help center-Priority'));
    await tapKey(tester, 'cell-Help center-Priority');
    // The picker's surface starts at the cell's top-left.
    final surface = tester.getRect(find.ancestor(of: byKey('option-query'), matching: find.byType(MenuSurface)));
    expect((surface.left - cell.left).abs(), lessThan(2));
    expect((surface.top - cell.top).abs(), lessThan(2));
    expect(surface.width, greaterThanOrEqualTo(cell.width));
    await tapKey(tester, 'opt-${db.optionId('Priority', 'High')}');
    expect((await rowNamed(tester, 'Help center')).valueOf(db.schema['Priority']!).text, db.optionId('Priority', 'High'));
  });

  dbTest('a new filter rule opens its editor at its chip', (tester) async {
    await pumpDb(tester, mobile: false);
    await openView(tester, 'Table');
    await tapKey(tester, 'db-filter');
    await tapKey(tester, 'pick-Status');
    final chip = tester.getRect(byKey('bar-rule-0'));
    final option = tester.getRect(byKey('filter-option-Done'));
    expect(option.top, greaterThan(chip.bottom));
    expect((option.left - chip.left).abs(), lessThan(40));
  });

  dbTest('with a default template, the menu still makes an empty page', (tester) async {
    await pumpDb(tester, mobile: false);
    await openView(tester, 'Table');
    final store = dbState(tester).store;
    final t = await tester.runAsync(() => store.newTemplate());
    await core(tester, (v) async {
      await v.renamePage(t!, 'Weekly task');
      await v.run((x) => x.setIcon(id: t, icon: '🗓'), writes: true);
      await v.run((x) => x.setProperty(page: t, key: db.schema['Status']!.key, value: Values.status(db.optionId('Status', 'In progress'))),
          writes: true);
      await v.apply(t, [
        BOp(op: OpKind.insert, id: newId(), kind: Kinds.todo, text: 'Write the summary', marks: const [], checked: false),
      ]);
    });
    await settle(tester);
    await tapKey(tester, 'db-new-menu');
    expect(byKey('template-default'), findsOneWidget);
    await tapKey(tester, 'template-empty');
    await closeRowSheet(tester);
    final rows = await rowsOf(tester, db.projects);
    expect(rows.length, projectRows.length + 1);
    final added = rows.firstWhere((r) => !projectRows.contains(r.title));
    final page = await core(tester, (v) => v.loadPage(added.id));
    // (The editor gives an empty page one empty paragraph.)
    expect(page.blocks.where((b) => b.text.isNotEmpty || b.kind != Kinds.paragraph), isEmpty);
    expect(page.properties, isEmpty);
    expect(page.icon, isNull);
    // The default template stays.
    expect(await core(tester, (v) => v.run((x) => x.defaultTemplate(collection: db.projects))), t);
  });
}
