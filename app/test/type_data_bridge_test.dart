import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';

import 'harness.dart';

void main() {
  setUpAll(initCore);

  test('typed creation and type-data operations cross the real bridge', () async {
    final dir = Directory.systemTemp.createTempSync('notes_type_data_bridge_');
    Vault? vault;
    try {
      final v = vault = await Vault.open(dir.path);
      var writes = 0;
      v.onWrite = () => writes++;
      final page = await v.createPage(title: 'Typed', pageType: 'mod:example');
      expect((await v.loadPage(page)).pageType, 'mod:example');
      expect((await v.listPages(offset: 0, limit: 10)).single.pageType, 'mod:example');
      expect(writes, 1);
      final plain = await v.createPage(title: 'Plain');
      expect((await v.loadPage(plain)).pageType, documentType);
      await v.setPageType(page, 'mod:converted');
      expect((await v.loadPage(page)).pageType, 'mod:converted');
      expect(jsonDecode(await v.typeData(page, 'example')), <String, dynamic>{});
      expect(jsonDecode(await v.typeDataAll(page)), <String, dynamic>{});

      const markdown = '\n# Readable 雪\r\n\r\n- ready-made body  \n';
      expect(await v.typeMarkdownBody(page), isNull);
      final beforeBody = writes;
      await v.setTypeMarkdownBody(page, markdown);
      expect(writes, beforeBody + 1);
      expect(await v.typeMarkdownBody(page), markdown);
      expect(jsonDecode(await v.typeDataAll(page)), {r'$markdown': markdown});
      await v.setTypeMarkdownBody(page, null);
      expect(await v.typeMarkdownBody(page), isNull);
      expect(jsonDecode(await v.typeDataAll(page)), <String, dynamic>{});
      await v.setTypeMarkdownBody(page, '');
      expect(await v.typeMarkdownBody(page), '');
      await v.setTypeMarkdownBody(page, markdown);

      await v.typeDataSet(page, 'example', ['settings', 'enabled'], 'true');
      await v.typeDataSet(page, 'example', ['settings', 'count'], '3');
      await v.typeDataSet(page, 'example', ['items'], '[]');
      await v.typeDataListInsert(page, 'example', ['items'], '{"id":"a","title":"First","nested":[null,2]}');
      await v.typeDataListInsert(page, 'example', ['items'], '{"id":"b","title":"Second"}', index: 0);
      await v.typeDataSet(page, 'example', ['items', 'a', 'title'], '"Renamed 雪"');
      await v.typeDataListMove(page, 'example', ['items'], const TypeDataListItem(id: 'a'), 0);
      final data = jsonDecode(await v.typeData(page, 'example')) as Map<String, dynamic>;
      expect(data['settings'], {'enabled': true, 'count': 3});
      expect(data['items'], [
        {
          'id': 'a',
          'title': 'Renamed 雪',
          'nested': [null, 2],
        },
        {'id': 'b', 'title': 'Second'},
      ]);
      await v.typeDataListMove(page, 'example', ['items'], const TypeDataListItem(index: 0), 1);
      await v.typeDataListDelete(page, 'example', ['items'], const TypeDataListItem(id: 'b'));
      await v.typeDataListDelete(page, 'example', ['items'], const TypeDataListItem(index: 0));
      await v.typeDataListInsert(page, 'example', ['items'], '"string-id"');
      await v.typeDataListDelete(page, 'example', ['items'], const TypeDataListItem(id: 'string-id'));
      await v.typeDataDelete(page, 'example', ['settings', 'count']);
      await v.typeDataSet(page, 'unknown-addon', ['opaque'], '{"version":7,"values":[false,null,"雪"]}');
      final all = jsonDecode(await v.typeDataAll(page)) as Map<String, dynamic>;
      expect(all['example'], {
        'settings': {'enabled': true},
        'items': [],
      });
      expect(all['unknown-addon'], {
        'opaque': {
          'version': 7,
          'values': [false, null, '雪'],
        },
      });
      expect(all[r'$markdown'], markdown);

      final beforeFailures = writes;
      await expectLater(v.typeDataSet(page, 'example', ['settings', 'enabled', 'bad'], '0'), throwsA(anything));
      await expectLater(v.typeDataSet(page, 'example', ['bad'], 'invalid json'), throwsA(anything));
      await expectLater(v.typeDataListDelete(page, 'example', ['items'], const TypeDataListItem()), throwsA(anything));
      await expectLater(
        v.typeDataListDelete(page, 'example', ['items'], const TypeDataListItem(index: 0, id: 'a')),
        throwsA(anything),
      );
      expect(writes, beforeFailures);
      expect(jsonDecode(await v.typeDataAll(page)), all);
      final changes = await v.pollChanges(0);
      expect(changes.changes.any((c) => c.kind == ChangeKind.pageChanged && c.page == page), isTrue);
      await v.flush();
      v.dispose();
      vault = await Vault.open(dir.path);
      expect((await vault.loadPage(page)).pageType, 'mod:converted');
      expect(jsonDecode(await vault.typeDataAll(page)), all);
      expect(await vault.typeMarkdownBody(page), markdown);
    } finally {
      vault?.dispose();
      await dir.delete(recursive: true);
    }
  });
}
