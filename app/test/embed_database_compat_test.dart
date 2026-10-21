import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/registry/embed_block.dart';

import 'db_harness.dart';
import 'harness.dart' as editor show editableWith, focusAt, typeText;

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

  dbTest('the Database slash item inserts an embed targeting a new database', (
    tester,
  ) async {
    await pumpDb(tester, mobile: false);
    await openPage(tester, 'Planning');
    final field = editor.editableWith('Notes after the database.');
    await editor.focusAt(tester, field, 'Notes after the database.'.length);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await settle(tester);
    await editor.typeText(tester, '/database');
    await tapKey(tester, 'slash-database');
    await settle(tester);
    final editorState = tester.state<PageEditorState>(
      find.byType(PageScreen).last,
    );
    final node = editorState.doc!.flat.singleWhere((n) => n.kind == embedKind);
    expect(node.target, isNotNull);
    expect(byKey('page-picker'), findsNothing);
    final saved = await core(tester, (v) => v.loadPage(db.host));
    final embed = saved.blocks.singleWhere((b) => b.kind == embedKind);
    expect(embed.text, isEmpty);
    expect(embed.target, node.target);
    final target = await core(tester, (v) => v.loadPage(embed.target!));
    expect(target.pageType, 'database');
    expect(target.parent, db.host);
    expect(byKey('inline-db-${embed.target}'), findsOneWidget);
    // Opening the old inline block did not migrate it.
    expect(saved.blocks.singleWhere((b) => b.id == 'db1').kind, 'database');
  });
}
