// Folder mounts on the real core, in temp folders only: Settings → Vault
// mounts a folder, its Markdown files become pages, an edit in the app is
// saved to the file, an edit to the file outside shows up through the
// bridge's poll, and "Remove" unmounts without touching the files.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/settings/settings_screen.dart';
import 'package:notes_app/shell/home.dart';

import 'db_harness.dart';

Future<List<String>> _texts(WidgetTester tester, String id) async =>
    [for (final b in (await core(tester, (v) => v.loadPage(id))).blocks) b.text];

void main() {
  setUpAll(initCore);

  dbTest('mount a folder in settings: file, page, edit, file, outside edit', (tester) async {
    await pumpDb(tester, mobile: false);
    final folder = Directory.systemTemp.createTempSync('notes_mount_test_');
    addTearDown(() {
      try {
        folder.deleteSync(recursive: true);
      } catch (_) {}
    });
    final file = File('${folder.path}${Platform.pathSeparator}Ideas.md')..writeAsStringSync('Grow tomatoes.\n');

    showSettings(tester.element(find.byType(HomeScreen)), section: SettingsSection.storage);
    await settle(tester);
    await typeInto(tester, 'mount-path-field', 'not/absolute');
    await tapKey(tester, 'mount-add');
    expect(byKey('mount-error'), findsOneWidget);
    await typeInto(tester, 'mount-path-field', folder.path);
    await tapKey(tester, 'mount-add');
    await settle(tester);
    expect(byKey('mount-error'), findsNothing);
    expect(byKey('mount-0'), findsOneWidget);

    // The folder is a page; its file is a child page.
    final mounts = db.vault.raw.listMounts();
    expect(mounts.single.folder, isNotEmpty);
    final root = mounts.single.page;
    final ideas = (await core(tester, (v) => v.listChildren(root))).single;
    expect(ideas.title, 'Ideas');
    expect(await _texts(tester, ideas.id), ['Grow tomatoes.']);

    // An edit in the app is saved to the file.
    await core(tester, (v) async {
      await v.apply(ideas.id, [BOp(op: OpKind.insert, id: newId(), kind: Kinds.paragraph, text: 'Water daily.', marks: const [], checked: false)]);
      await v.flush();
    });
    expect(file.readAsStringSync(), contains('Water daily.'));
    expect(file.readAsStringSync(), isNot(contains('id:')));

    // An edit outside is picked up by the poll, without a rescan call.
    file.writeAsStringSync('Edited outside.\n');
    var texts = <String>[];
    final deadline = Stopwatch()..start();
    while (!texts.contains('Edited outside.') && deadline.elapsed < const Duration(seconds: 12)) {
      texts = await _texts(tester, ideas.id);
    }
    expect(texts, ['Edited outside.']);

    // Remove: the mount goes, the page and the file stay.
    await tapKey(tester, 'mount-remove-0');
    await settle(tester);
    expect(byKey('mount-0'), findsNothing);
    expect(db.vault.raw.listMounts(), isEmpty);
    expect(file.readAsStringSync(), 'Edited outside.\n');
    expect(await _texts(tester, ideas.id), ['Edited outside.']);
  });
}
