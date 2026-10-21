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
      final expected = {
        'Plain': 'document',
        'Database': 'database',
        'Unknown': ' mod:board 雪 ',
      };
      expect(rows.length, expected.length);
      for (final row in rows) {
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
      for (final row in await vault.listPages(offset: 0, limit: 10)) {
        expect((await vault.loadPage(row.id)).pageType, expected[row.title]);
      }
    } finally {
      vault?.dispose();
      await dir.delete(recursive: true);
    }
  });
}
