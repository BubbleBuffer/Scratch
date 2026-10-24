import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/state/pages_store.dart' show PageMeta;

import 'harness.dart';

void main() {
  setUpAll(initCore);

  test('page types cross the bridge in loaded pages and index rows', () async {
    final dir = Directory.systemTemp.createTempSync('notes_page_type_bridge_');
    final source = Directory('${dir.path}/source')..createSync();
    File('${source.path}/Plain.md').writeAsStringSync('Plain body\n');
    File('${source.path}/Database.md')
        .writeAsStringSync('---\ncollection: true\n---\nDatabase body\n');
    File('${source.path}/Unknown.md')
        .writeAsStringSync('---\ntype: " mod:board 雪 "\n---\nUnknown body\n');
    Vault? vault;
    try {
      vault = await Vault.open('${dir.path}/vault');
      await vault.run((v) => v.importMarkdown(dir: source.path), writes: true);
      await vault.flush();
      final rows = await vault.listPages(offset: 0, limit: 10);
      // This device's node record is an ordinary page of type node under the
      // hidden Nodes page. It is not part of this test, so it is left out by ID.
      final nodes = {for (final row in rows) if (row.pageType == 'node') row.id};
      expect(nodes, hasLength(1));
      final imported = rows.where((row) => !nodes.contains(row.id)).toList();
      final expected = {
        'Plain': 'document',
        'Database': 'database',
        'Unknown': ' mod:board 雪 ',
      };
      expect(imported.length, expected.length);
      for (final row in imported) {
        expect(row.pageType, expected[row.title]);
        expect(row.isDatabase, row.title == 'Database');
        final page = await vault.loadPage(row.id);
        expect(page.pageType, expected[row.title]);
        expect(page.isDatabase, row.title == 'Database');
        expect(
          PageMeta.of(page).withPrefs(page.prefs).pageType,
          expected[row.title],
        );
        expect(page.properties.where((p) => p.key == 'type'), isEmpty);
      }
      vault.dispose();
      vault = null;
      vault = await Vault.open('${dir.path}/vault');
      final reopened = await vault.listPages(offset: 0, limit: 10);
      for (final row in reopened.where((r) => !nodes.contains(r.id))) {
        expect((await vault.loadPage(row.id)).pageType, expected[row.title]);
      }
    } finally {
      vault?.dispose();
      await dir.delete(recursive: true);
    }
  });
}
