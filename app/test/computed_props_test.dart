// Files, formula and rollup properties on the real core: values computed by
// the core show in cells (errors too), formulas and rollups are set up from
// the property menu, and files are added from a cell.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/ui/database/config.dart';
import 'package:notes_app/ui/database/values.dart';

import 'db_harness.dart';

Future<List<PropDef>> _schema(WidgetTester tester, String collection) async {
  await settle(tester);
  return core(tester, (v) => v.run((x) => x.getSchema(collection: collection)));
}

/// Adds [defs] to Projects and shows them in the Table view.
Future<Map<String, PropDef>> _addToProjects(WidgetTester tester, List<PropDef> defs) async {
  final old = await _schema(tester, db.projects);
  final saved = await core(tester, (v) => v.run((x) => x.setSchema(collection: db.projects, schema: [...old, ...defs]), writes: true));
  final views = await core(tester, (v) => v.run((x) => x.listViews(collection: db.projects)));
  final table = views.firstWhere((v) => v.name == 'Table');
  final added = [for (final d in saved) if (!old.any((o) => o.key == d.key)) d.key];
  await core(
      tester,
      (v) => v.run((x) => x.updateView(collection: db.projects, view: table.copyWith(visibleProps: [...added, ...table.visibleProps])),
          writes: true));
  await settleUntil(tester, () => defs.every((d) => byKey('col-${d.name}').evaluate().isNotEmpty));
  return {for (final d in saved) d.name: d};
}

PropDef _def(String name, PropKind kind, [String config = '']) =>
    PropDef(key: '', name: name, kind: kind, options: const [], config: config);

Future<PropValue> _value(WidgetTester tester, String row, PropDef d) async => (await rowNamed(tester, row)).valueOf(d);

void main() {
  setUpAll(initCore);

  dbTest('formula: typed in the property menu, computed by the core, errors in the cell', (tester) async {
    await pumpDb(tester, mobile: false);
    await openView(tester, 'Table');
    final by = await _addToProjects(tester, [_def('Double', PropKind.formula)]);
    final d = by['Double']!;
    // Estimate of "API migration" is 10 (fixture: (i % 5 + 1) * 2 with i = 4).
    await tapKey(tester, 'col-Double');
    await tapKey(tester, 'prop-edit');
    await tapKey(tester, 'prop-formula');
    await typeInto(tester, 'formula-field', 'prop("Estimate") * 2');
    await tapKey(tester, 'formula-done');
    await dismiss(tester);
    final s = await _schema(tester, db.projects);
    expect(PropConfig(s.firstWhere((x) => x.key == d.key).config).formula, 'prop("Estimate") * 2');
    final v = await _value(tester, 'API migration', d);
    expect(v.kind, ValueKind.number);
    expect(v.number, 20);
    expect(find.text('20'), findsWidgets);
    // A failing formula: the message shows in the cells.
    final bad = d.copyWith(config: (PropConfig(d.config)..formula = 'prop("Estimate") / 0').encode());
    await core(tester, (v) => v.run((x) => x.setSchema(collection: db.projects, schema: [for (final x in s) x.key == d.key ? bad : x]), writes: true));
    await settle(tester);
    expect((await _value(tester, 'API migration', d)).kind, ValueKind.error);
    expect(byKey('computed-error'), findsWidgets);
    expect(find.text('Division by zero'), findsWidgets);
  });

  dbTest('rollup: relation, property and calculation from the property menu', (tester) async {
    await pumpDb(tester, mobile: false);
    // A "Milestones" database with points, linked from Projects.
    final ms = await core(tester, (v) async {
      final id = await v.createPage(title: 'Milestones');
      await v.run((x) => x.makeCollection(page: id), writes: true);
      return id;
    });
    final msSchema = await core(tester, (v) => v.run((x) => x.setSchema(collection: ms, schema: [_def('Points', PropKind.number)]), writes: true));
    final points = msSchema.first.key;
    final ids = <String>[];
    for (final (title, n) in [('Alpha', 3.0), ('Beta', 5.0)]) {
      final r = await core(tester, (v) => v.createPage(parent: ms, title: title));
      await core(tester, (v) => v.run((x) => x.setProperty(page: r, key: points, value: Values.number(n)), writes: true));
      ids.add(r);
    }
    await openView(tester, 'Table');
    final by = await _addToProjects(tester, [
      _def('Milestones', PropKind.relation, jsonEncode({'relation': ms})),
      _def('Total', PropKind.rollup),
    ]);
    final rel = by['Milestones']!, total = by['Total']!;
    final row = (await rowNamed(tester, 'Pricing page')).id;
    await core(tester, (v) => v.run((x) => x.setProperty(page: row, key: rel.key, value: Values.relation(ids)), writes: true));
    // Set up: relation, property, sum.
    await tapKey(tester, 'col-Total');
    await tapKey(tester, 'prop-edit');
    await tapKey(tester, 'rollup-relation');
    await tapKey(tester, 'rollup-choice-Milestones');
    await tapKey(tester, 'rollup-property');
    await tapKey(tester, 'rollup-choice-Points');
    await tapKey(tester, 'rollup-function');
    await tapKey(tester, 'rollup-choice-Sum');
    await dismiss(tester);
    final cfg = PropConfig((await _schema(tester, db.projects)).firstWhere((x) => x.key == total.key).config);
    expect((cfg.rollupRelation, cfg.rollupProperty, cfg.rollupFunction), (rel.key, points, 'sum'));
    expect((await _value(tester, 'Pricing page', total)).number, 8);
    expect(find.text('8'), findsWidgets);
    // A change on the related page updates the rollup on screen.
    await core(tester, (v) => v.run((x) => x.setProperty(page: ids[0], key: points, value: Values.number(10)), writes: true));
    await settleUntil(tester, () => find.text('15').evaluate().isNotEmpty);
    expect(find.text('15'), findsWidgets);
  });

  dbTest('files: add a file to a row from its cell', (tester) async {
    await pumpDb(tester, mobile: false);
    await openView(tester, 'Table');
    final by = await _addToProjects(tester, [_def('Attachments', PropKind.files)]);
    final d = by['Attachments']!;
    final file = File('${db.dir.path}${Platform.pathSeparator}brief.pdf');
    await tester.runAsync(() => file.writeAsBytes(utf8.encode('%PDF brief')));
    await tapKey(tester, 'cell-Pricing page-Attachments');
    await tapKey(tester, 'file-add');
    await typeInto(tester, 'picker-field', file.path);
    await submit(tester);
    await settleUntil(tester, () => find.text('brief.pdf').evaluate().isNotEmpty);
    expect(find.text('brief.pdf'), findsWidgets);
    await dismiss(tester);
    final v = await _value(tester, 'Pricing page', d);
    expect(v.kind, ValueKind.files);
    expect(v.ids, hasLength(1));
    final info = await core(tester, (x) => x.run((r) => r.attachmentInfo(id: v.ids.first)));
    expect(info.name, 'brief.pdf');
  });
}
