import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'db_harness.dart';

void main() {
  setUpAll(initCore);

  dbTest(
    'a legacy database Markdown file still draws without migrating its kind',
    (tester) async {
      await pumpDb(tester, mobile: false);
      await core(tester, (v) => v.flush());
      final file = Directory('${db.vault.root}/pages')
          .listSync()
          .whereType<File>()
          .singleWhere((f) => f.path.contains(db.host));
      final markdown = file.readAsStringSync();
      expect(markdown, contains('<!-- database -->'));
      // An outside edit forces the old file syntax through the real importer.
      file.writeAsStringSync('$markdown\nLegacy file fixture.\n');
      await core(
        tester,
        (v) => v.run((native) => native.rescan(), writes: true),
      );
      final before = await core(tester, (v) => v.loadPage(db.host));
      expect(before.blocks.singleWhere((b) => b.id == 'db1').kind, 'database');
      await openPage(tester, 'Planning');
      expect(byKey('inline-db-${db.projects}'), findsOneWidget);
      expect(byKey('row-Website redesign'), findsOneWidget);
      final after = await core(tester, (v) => v.loadPage(db.host));
      expect(after.blocks.singleWhere((b) => b.id == 'db1').kind, 'database');
      expect(
        after.blocks.singleWhere((b) => b.id == 'db1').target,
        db.projects,
      );
    },
  );
}
