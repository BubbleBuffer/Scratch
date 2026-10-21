// Database names of untitled pages: a row goes by the first line of its body
// in the table, a calendar entry shows it, and a relation chip does too.
//
// Run with NOTES_BRIDGE_LIB pointing at the built notes_bridge library.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/database/views/common.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/ui/database/values.dart';

import 'db_harness.dart';

const line = 'Groceries for Sunday';

/// An untitled page under [parent] whose body starts with [line].
Future<String> untitledPage(WidgetTester tester, String parent) async {
  final id = await core(tester, (v) => v.createPage(parent: parent, title: ''));
  await core(
    tester,
    (v) => v.apply(id, [
      BOp(
        op: OpKind.insert,
        id: newId(),
        kind: Kinds.paragraph,
        text: line,
        marks: const [],
        checked: false,
      ),
    ]),
  );
  return id;
}

/// Adds a relation property "Related" to Projects and shows it in the Table.
Future<PropDef> addRelation(WidgetTester tester) async {
  final old = await core(
    tester,
    (v) => v.run((x) => x.getSchema(collection: db.projects)),
  );
  final saved = await core(
    tester,
    (v) => v.run(
      (x) => x.setSchema(
        collection: db.projects,
        schema: [
          ...old,
          PropDef(
            key: '',
            name: 'Related',
            kind: PropKind.relation,
            options: const [],
            config: '',
          ),
        ],
      ),
      writes: true,
    ),
  );
  final added = saved.firstWhere((d) => !old.any((o) => o.key == d.key));
  final views = await core(
    tester,
    (v) => v.run((x) => x.listViews(collection: db.projects)),
  );
  final table = views.firstWhere((v) => v.name == 'Table');
  await core(
    tester,
    (v) => v.run(
      (x) => x.updateView(
        collection: db.projects,
        view: table.copyWith(visibleProps: [added.key, ...table.visibleProps]),
      ),
      writes: true,
    ),
  );
  return added;
}

void main() {
  setUpAll(initCore);

  dbTest('an untitled row goes by its first line in the table', (tester) async {
    await pumpDb(tester, mobile: false);
    await untitledPage(tester, db.projects);
    await openView(tester, 'Table');
    final inTitle = find.descendant(
      of: find.byType(RowTitle),
      matching: find.text(line),
    );
    await settleUntil(tester, () => inTitle.evaluate().isNotEmpty);
    expect(inTitle, findsOneWidget);
    expect(
      find.descendant(
        of: find.byType(RowTitle),
        matching: find.text('Untitled'),
      ),
      findsNothing,
    );
  });

  dbTest('a calendar entry of an untitled row shows its first line', (
    tester,
  ) async {
    await pumpDb(tester, mobile: false);
    final id = await untitledPage(tester, db.projects);
    // October 4, 2026 is a Sunday.
    final due = db.schema['Due']!.key;
    await core(
      tester,
      (v) => v.run(
        (x) => x.setProperty(
          page: id,
          key: due,
          value: Values.date(Day(2026, 10, 4).ms),
        ),
        writes: true,
      ),
    );
    await openView(tester, 'Calendar');
    await settleUntil(tester, () => find.text(line).evaluate().isNotEmpty);
    expect(find.text(line), findsWidgets);
    expect(find.text('Untitled'), findsNothing);
  });

  dbTest('a relation chip shows an untitled linked page by its first line', (
    tester,
  ) async {
    await pumpDb(tester, mobile: false);
    final page = await untitledPage(tester, db.host);
    final rel = await addRelation(tester);
    final row = (await rowNamed(tester, 'Pricing page')).id;
    await core(
      tester,
      (v) => v.run(
        (x) => x.setProperty(
          page: row,
          key: rel.key,
          value: Values.relation([page]),
        ),
        writes: true,
      ),
    );
    await openView(tester, 'Table');
    final chip = byKey('cell-Pricing page-Related');
    final inChip = find.descendant(of: chip, matching: find.text(line));
    await settleUntil(tester, () => inChip.evaluate().isNotEmpty);
    expect(inChip, findsOneWidget);
    expect(
      find.descendant(of: chip, matching: find.text('Untitled')),
      findsNothing,
    );
  });
}
