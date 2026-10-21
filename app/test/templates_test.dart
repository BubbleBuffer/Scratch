// Page templates on the real core: a new child page offers its parent's
// templates and takes the one picked, a template says so and can become
// the default, and a database's "New ▾" menu creates rows from any of its
// templates.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;

import 'db_harness.dart';

/// A template of [parent] titled [title] with one paragraph [text].
Future<String> _template(Vault v, String parent, String title, String text) async {
  final id = await v.run((x) => x.createTemplate(parent: parent, title: title), writes: true);
  await v.apply(id, [BOp(op: OpKind.insert, id: newId(), kind: Kinds.paragraph, text: text, marks: const [], checked: false)]);
  return id;
}

Future<List<String>> _texts(WidgetTester tester, String id) async =>
    [for (final b in (await core(tester, (v) => v.loadPage(id))).blocks) b.text];

void main() {
  setUpAll(initCore);

  dbTest('a new child page offers the templates of its parent', (tester) async {
    await pumpDb(tester, mobile: false);
    final (parent, standup) = await core(tester, (v) async {
      final p = await v.createPage(title: 'Meetings');
      final s = await _template(v, p, 'Standup', 'Yesterday, today, blockers');
      await _template(v, p, 'Retro', 'What went well');
      return (p, s);
    });
    // Templates are not children in the sidebar.
    final kids = await core(tester, (v) => v.listChildren(parent));
    expect(kids, isEmpty);
    // "Add a page inside" Meetings.
    nav.createPage(tester.element(find.byType(HomeScreen)), parent: parent);
    await settle(tester);
    expect(byKey('page-starters'), findsOneWidget);
    expect(byKey('template-pick-Retro'), findsOneWidget);
    await tapKey(tester, 'template-pick-Standup');
    await settleUntil(tester, () => find.text('Yesterday, today, blockers').evaluate().isNotEmpty);
    // In the body, and as the untitled page's stand-in name in the rail and
    // breadcrumb.
    expect(find.text('Yesterday, today, blockers'), findsWidgets);
    // The page has content now: the starters are gone.
    expect(byKey('page-starters'), findsNothing);
    final page = (await core(tester, (v) => v.listChildren(parent))).single;
    expect(await _texts(tester, page.id), ['Yesterday, today, blockers']);

    // The template itself: a banner; make it the default.
    nav.openPage(tester.element(find.byType(HomeScreen, skipOffstage: false)), standup, 'Standup');
    await settle(tester);
    expect(byKey('template-banner'), findsOneWidget);
    await tapKey(tester, 'template-make-default');
    await settle(tester);
    expect(await core(tester, (v) => v.run((x) => x.defaultTemplate(collection: parent))), standup);
    // New children start from the default; no offer on a page with content.
    final made = await core(tester, (v) => v.createPage(parent: parent, title: 'Monday'));
    expect(await _texts(tester, made), ['Yesterday, today, blockers']);
  });

  dbTest('database New menu creates rows from any template', (tester) async {
    await pumpDb(tester, mobile: false);
    await openView(tester, 'Table');
    await core(tester, (v) async {
      await _template(v, db.projects, 'Bug', 'Steps to reproduce');
      await _template(v, db.projects, 'Feature', 'Problem to solve');
    });
    await settle(tester);
    await tapKey(tester, 'db-new-menu');
    expect(byKey('template-item-Bug'), findsOneWidget);
    await tapKey(tester, 'template-item-Feature');
    await settle(tester);
    final rows = await rowsOf(tester, db.projects);
    // Templates are no rows; the new row is.
    expect(rows.map((r) => r.title), isNot(contains('Bug')));
    expect(rows.length, projectRows.length + 1);
    final added = rows.firstWhere((r) => !projectRows.contains(r.title));
    expect(await _texts(tester, added.id), ['Problem to solve']);
  });
}
