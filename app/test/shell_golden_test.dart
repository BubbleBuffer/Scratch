// Golden screenshots of the shell: the sidebar (favorites, an expanded
// nested tree, the row menu open), Today with the All pages disclosure, the
// trash, version history, the import dialog, Move to and search results
// with paths. 1440x900 (desktop) and 375x812 (mobile), light and dark.
// Regenerate with: flutter test --update-goldens test/shell_golden_test.dart

@Tags(['golden'])
library;

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/blocks/attachment.dart' show clearAttachmentCache;
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/features/history/history_panel.dart';
import 'package:notes_app/features/import/import_panel.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/move_to.dart';

import 'harness.dart';
import 'pages_fixture.dart' show testPng, settleImages;

Future<void> shot(String name) async {
  await expectLater(
    find.byType(Directionality).first,
    matchesGoldenFile('../screenshots/$name.png'),
  );
}

Finder key(String k) => find.byKey(ValueKey(k));
Finder fieldIn(Finder f) =>
    find.descendant(of: f, matching: find.byType(EditableText));
BuildContext homeContext(WidgetTester tester) =>
    tester.element(find.byType(HomeScreen));

/// Gives "Weekly review" a cover and marks "Budget" as a favorite.
Future<void> decorate(WidgetTester tester, TestVault s) async {
  clearAttachmentCache();
  final png = (await tester.runAsync(
    () => testPng(const [Color(0xFFF2D5C4), Color(0xFFB9D3E8)]),
  ))!;
  final cover = await core(
    tester,
    (v) => v.run(
      (x) => x.addAttachmentBytes(name: 'cover.png', bytes: png),
      writes: true,
    ),
  );
  await core(
    tester,
    (v) => v.run(
      (x) => x.setCover(id: s.pages['Weekly review']!, cover: cover),
      writes: true,
    ),
  );
  await core(tester, (v) => v.setFavorite(s.pages['Budget']!, true));
  await core(tester, (v) => v.setFavorite(s.pages['Recipes']!, true));
  // The tree and All pages list keep newest-edit order and cover metadata.
  final weekly = s.pages['Weekly review']!;
  bool done() =>
      s.app.tree.favorites.length == 2 &&
      s.app.pages.rowAt(0)?.id == s.pages['Recipes'] &&
      s.app.pages.rowAt(2)?.id == weekly &&
      s.app.pages.meta(weekly)?.cover == cover;
  await settleUntil(tester, done);
  expect(done(), isTrue, reason: 'home shows the decorated pages');
  await settleImages(tester);
}

/// A second device edits page [id], so it has two versions.
Future<void> secondVersion(WidgetTester tester, TestVault s, String id) async {
  final other = Directory.systemTemp.createTempSync('notes_other_');
  await tester.runAsync(() async {
    final b = await Vault.open('${other.path}${Platform.pathSeparator}vault');
    final a = s.vault;
    final mine = await a.run((y) => y.exportUpdates(page: id, since: const []));
    await b.run((x) => x.importUpdates(page: id, bytes: mine));
    final first = (await b.loadPage(id)).blocks.first;
    await b.apply(id, [
      BOp(
        op: OpKind.setText,
        id: first.id,
        kind: first.kind,
        text: 'Tomatoes, beans and herbs by the south fence.',
        marks: const [],
        checked: false,
      ),
      BOp(
        op: OpKind.insert,
        id: newId(),
        after: first.id,
        kind: Kinds.todo,
        text: 'Order seeds',
        marks: const [],
        checked: false,
      ),
    ]);
    final back = await b.run((x) => x.exportUpdates(page: id, since: const []));
    await a.run((x) => x.importUpdates(page: id, bytes: back), writes: true);
    await b.flush();
    b.dispose();
  });
  // The home list behind the history shows the page as the newest edit.
  await settleUntil(tester, () => s.app.pages.rowAt(0)?.id == id);
  expect(
    s.app.pages.rowAt(0)?.id,
    id,
    reason: 'home lists the edited page first',
  );
  await settle(tester);
  try {
    other.deleteSync(recursive: true);
  } catch (_) {}
}

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';

      if (!mobile) {
        appTest('sidebar $tag', (tester) async {
          final s = await pumpApp(tester, mobile: mobile, dark: dark);
          await decorate(tester, s);
          final recipes = s.pages['Recipes']!;
          await tapAndSettle(tester, key('tree-toggle-$recipes'));
          // Soups has no subpages: it shows no chevron to open.
          final garden = s.pages['Garden plan']!;
          final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
          await g.addPointer(location: Offset.zero);
          await g.moveTo(tester.getCenter(key('tree-$garden')));
          await settle(tester, min: Duration.zero);
          await tapAndSettle(tester, key('tree-more-$garden'));
          await shot('$tag-sidebar');
          await g.removePointer();
        });
      } else {
        appTest('favorites $tag', (tester) async {
          final s = await pumpApp(tester, mobile: mobile, dark: dark);
          await decorate(tester, s);
          await tapAndSettle(tester, key('nav-pages'));
          await tester.ensureVisible(find.text('Favorites'));
          await tester.pump();
          await settle(tester);
          await shot('$tag-favorites');
        });
      }

      appTest('Today page disclosure $tag', (tester) async {
        final s = await pumpApp(tester, mobile: mobile, dark: dark);
        await decorate(tester, s);
        await expandAllPages(tester);
        await settle(tester);
        await shot('$tag-today-pages');
      });

      appTest('trash $tag', (tester) async {
        final s = await pumpApp(tester, mobile: mobile, dark: dark);
        await core(tester, (v) => v.deletePage(s.pages['Soups']!));
        await core(tester, (v) => v.deletePage(s.pages['Budget']!));
        await core(tester, (v) => v.deletePage(s.pages['Travel checklist']!));
        // Wait until the tree and page list have dropped them.
        await settleUntil(
          tester,
          () =>
              find.text('Travel checklist').evaluate().isEmpty &&
              find.text('Budget').evaluate().isEmpty,
        );
        if (mobile) {
          await tapAndSettle(tester, key('nav-pages'));
          await tester.ensureVisible(key('pages-trash'));
          await settle(tester);
          await tapAndSettle(tester, key('pages-trash'));
        } else {
          await tapAndSettle(tester, key('sidebar-trash'));
        }
        await shot('$tag-trash');
      });

      appTest('history $tag', (tester) async {
        final s = await pumpApp(tester, mobile: mobile, dark: dark);
        final garden = s.pages['Garden plan']!;
        await secondVersion(tester, s, garden);
        // Keep the displayed version time fixed within the harness day.
        versionTimeOf = (_) => DateTime(2026, 10, 6, 9, 42);
        addTearDown(() => versionTimeOf = DateTime.fromMillisecondsSinceEpoch);
        showVersionHistory(homeContext(tester), garden);
        await settle(tester);
        await tapAndSettle(
          tester,
          key('history-version-1'),
          waitForBridge: true,
        );
        final oldText = find.descendant(
          of: key('history-preview'),
          matching: editableWith('Some text about garden plan.'),
        );
        expect(oldText, findsOneWidget);
        expect(
          editableWith('Tomatoes, beans and herbs by the south fence.'),
          findsNothing,
        );
        expect(editableWith('Order seeds'), findsNothing);
        await shot('$tag-history');
      });

      appTest('import $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        final dir = Directory.systemTemp.createTempSync('notes_golden_import_');
        File('${dir.path}${Platform.pathSeparator}Alpha.md')
            .writeAsStringSync('# Alpha\n\nFirst file.\n');
        File('${dir.path}${Platform.pathSeparator}Beta.md')
            .writeAsStringSync('Second file.\n');
        showImport(homeContext(tester));
        await settle(tester);
        await tester.enterText(fieldIn(key('import-path')), dir.path);
        await tester.tap(key('import-run'));
        await settleUntil(
          tester,
          () => (tester.widget<Text>(key('import-result')).data ?? '')
              .startsWith('Imported'),
        );
        // The real temp path differs per machine: show a fixed one.
        await tester.enterText(
          fieldIn(key('import-path')),
          r'C:\Users\me\Documents\Obsidian vault',
        );
        await settle(tester);
        FocusManager.instance.primaryFocus?.unfocus();
        await settle(tester);
        final offset =
            editState(tester, fieldIn(key('import-path'))).renderEditable.offset
                as ScrollPosition;
        // Pin the field scroll after replacing a unique path; caret reveal
        // otherwise leaves a fractional offset from the previous contents.
        offset.jumpTo(offset.maxScrollExtent);
        await tester.pump();
        await shot('$tag-import');
        try {
          dir.deleteSync(recursive: true);
        } catch (_) {}
      });

      appTest('move to $tag', (tester) async {
        final s = await pumpApp(tester, mobile: mobile, dark: dark);
        final soups = s.pages['Soups']!;
        if (mobile) {
          showMoveTo(homeContext(tester), soups);
        } else {
          await tapAndSettle(tester, key('tree-toggle-${s.pages['Recipes']}'));
          final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
          await g.addPointer(location: Offset.zero);
          await g.moveTo(tester.getCenter(key('tree-$soups')));
          await settle(tester, min: Duration.zero);
          await tapAndSettle(tester, key('tree-more-$soups'));
          await g.removePointer();
          await tapAndSettle(tester, key('tree-menu-move'));
        }
        await settle(tester);
        await shot('$tag-move-to');
      });

      appTest('search paths $tag', (tester) async {
        final s = await pumpApp(tester, mobile: mobile, dark: dark);
        for (final t in recipeChildren) {
          await core(
            tester,
            (v) => v.apply(s.pages[t]!, [
              BOp(
                op: OpKind.insert,
                id: newId(),
                kind: Kinds.paragraph,
                text: 'Some text about ${t.toLowerCase()}.',
                marks: const [],
                checked: false,
              ),
            ]),
          );
          // Equal-score hits are ordered by edit time; the core stamps edits
          // in strictly increasing milliseconds, so this order is stable.
        }
        await settle(tester);
        await core(tester, (v) => v.flush());
        // Search filters the page disclosure. The feed can reload PagesStore
        // before block edits reach the index, so pin this fixture's list to
        // the acknowledged writes before capturing it.
        await finishAsync(tester, s.app.pages.reload());
        await settle(tester);
        if (mobile) {
          await tapAndSettle(tester, key('nav-search'));
        } else {
          await expandAllPages(tester);
        }
        await tester.enterText(fieldIn(key('search-field')), 'some text');
        await settle(tester);
        FocusManager.instance.primaryFocus?.unfocus();
        await settle(tester);
        await shot('$tag-search-paths');
      });
    }
  }
}
