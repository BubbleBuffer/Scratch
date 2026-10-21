// Shell interactions: the sidebar (drag to move, page menu, favorites,
// resize, collapse), home (Today, page visits, mobile dock),
// search paths, the trash, version history, markdown import and export, and
// Move to. Checked against what reached the real core.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/editor/block_row.dart' show BlockRow;
import 'package:notes_app/features/getting_started.dart';
import 'package:notes_app/features/history/history_panel.dart';
import 'package:notes_app/features/import/import_panel.dart';
import 'package:notes_app/shell/home.dart';

import 'harness.dart';

Finder key(String k) => find.byKey(ValueKey(k));
Finder row(String id) => key('tree-$id');
Finder fieldIn(Finder f) =>
    find.descendant(of: f, matching: find.byType(EditableText));

Future<TestGesture> hoverAt(WidgetTester tester, Finder target) async {
  final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
  await g.addPointer(location: Offset.zero);
  await g.moveTo(tester.getCenter(target));
  await settle(tester, min: Duration.zero);
  return g;
}

/// Opens the "•••" menu of a sidebar row ([prefix] 'tree' or 'fav').
Future<void> openRowMenu(
  WidgetTester tester,
  String id, {
  String prefix = 'tree',
}) async {
  final g = await hoverAt(tester, key('$prefix-$id'));
  await tapAndSettle(tester, key('$prefix-more-$id'));
  await g.removePointer();
}

/// Drags sidebar row [id] with the mouse to [to] (global).
Future<void> dragRow(WidgetTester tester, String id, Offset to) async {
  final from = tester.getCenter(row(id));
  final g = await tester.startGesture(from, kind: PointerDeviceKind.mouse);
  await tester.pump();
  for (var i = 1; i <= 8; i++) {
    await g.moveTo(Offset.lerp(from, to, i / 8)!);
    await tester.pump(const Duration(milliseconds: 16));
  }
  await g.up();
  await settle(tester);
}

Future<List<String>> titlesUnder(WidgetTester tester, String? parent) async =>
    (await core(
      tester,
      (v) => v.listChildren(parent),
    )).map((p) => p.title).toList();

BuildContext homeContext(WidgetTester tester) =>
    tester.element(find.byType(HomeScreen));

void main() {
  setUpAll(initCore);

  group('sidebar', () {
    appTest(
      'dragging a row onto another nests it; near an edge puts it beside',
      (tester) async {
        final s = await pumpApp(tester, mobile: false);
        final budget = s.pages['Budget']!,
            garden = s.pages['Garden plan']!,
            weekly = s.pages['Weekly review']!;

        // Onto the middle of "Garden plan": inside, with a highlight while dragging.
        final from = tester.getCenter(row(budget));
        final g = await tester.startGesture(
          from,
          kind: PointerDeviceKind.mouse,
        );
        final to = tester.getCenter(row(garden));
        for (var i = 1; i <= 8; i++) {
          await g.moveTo(Offset.lerp(from, to, i / 8)!);
          await tester.pump(const Duration(milliseconds: 16));
        }
        expect(key('tree-drop-inside'), findsOneWidget);
        await g.up();
        await settle(tester);
        expect(await titlesUnder(tester, garden), ['Budget']);
        expect(row(budget), findsOneWidget, reason: 'the new parent expands');

        // Onto the lower edge of "Weekly review": after it, at the top level.
        final r = tester.getRect(row(weekly));
        await dragRow(tester, budget, Offset(r.center.dx, r.bottom - 3));
        final top = await titlesUnder(tester, null);
        expect(top.indexOf('Budget'), top.indexOf('Weekly review') + 1);
        expect(await titlesUnder(tester, garden), isEmpty);

        // Onto the upper edge of "Weekly review": before it.
        final r2 = tester.getRect(row(weekly));
        await dragRow(tester, budget, Offset(r2.center.dx, r2.top + 3));
        final top2 = await titlesUnder(tester, null);
        expect(top2.indexOf('Budget'), top2.indexOf('Weekly review') - 1);
      },
    );

    appTest('a page cannot be dropped into its own subtree', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final recipes = s.pages['Recipes']!, soups = s.pages['Soups']!;
      await tapAndSettle(tester, key('tree-toggle-$recipes'));
      await dragRow(tester, recipes, tester.getCenter(row(soups)));
      expect((await pageOf(tester, recipes)).parent, isNull);
      expect(await titlesUnder(tester, recipes), recipeChildren);
    });

    appTest('the page menu adds inside, duplicates and moves to the trash', (
      tester,
    ) async {
      final s = await pumpApp(tester, mobile: false);
      final budget = s.pages['Budget']!, garden = s.pages['Garden plan']!;

      await openRowMenu(tester, budget);
      for (final k in [
        'add',
        'rename',
        'duplicate',
        'move',
        'favorite',
        'trash',
      ]) {
        expect(key('tree-menu-$k'), findsOneWidget, reason: k);
      }
      await tapAndSettle(tester, key('tree-menu-add'));
      expect(key('page-title'), findsOneWidget);
      await tapAndSettle(tester, key('back'));
      expect((await core(tester, (v) => v.listChildren(budget))).length, 1);

      final before = (await titlesUnder(tester, null)).length;
      await openRowMenu(tester, garden);
      await tapAndSettle(tester, key('tree-menu-duplicate'));
      await settleUntil(
        tester,
        () => find.textContaining('Garden plan').evaluate().length >= 3,
      );
      expect((await titlesUnder(tester, null)).length, before + 1);
      expect(
        key('capture-field'),
        findsOneWidget,
        reason: 'duplicate stays on home',
      );

      await openRowMenu(tester, garden);
      await tapAndSettle(tester, key('tree-menu-trash'));
      expect(row(garden), findsNothing);
      expect((await pageOf(tester, garden)).inTrash, isTrue);
    });

    appTest('favorites: add from the menu, show above Pages, remove again', (
      tester,
    ) async {
      final s = await pumpApp(tester, mobile: false);
      final budget = s.pages['Budget']!;
      expect(key('sidebar-favorites'), findsNothing);
      await openRowMenu(tester, budget);
      expect(find.text('Add to Favorites'), findsOneWidget);
      await tapAndSettle(tester, key('tree-menu-favorite'));
      await settleUntil(tester, () => key('fav-$budget').evaluate().isNotEmpty);
      expect(key('sidebar-favorites'), findsOneWidget);
      expect(
        tester.getTopLeft(key('fav-$budget')).dy,
        lessThan(
          tester
              .getTopLeft(
                find.descendant(
                  of: key('sidebar'),
                  matching: find.text('Pages'),
                ),
              )
              .dy,
        ),
      );
      expect((await pageOf(tester, budget)).favorite, isTrue);

      // The favorite row opens the page and has the same menu.
      await openRowMenu(tester, budget, prefix: 'fav');
      expect(find.text('Remove from Favorites'), findsOneWidget);
      await tapAndSettle(tester, key('tree-menu-favorite'));
      await settleUntil(tester, () => key('fav-$budget').evaluate().isEmpty);
      expect(key('sidebar-favorites'), findsNothing);
      expect((await pageOf(tester, budget)).favorite, isFalse);
    });

    appTest('the edge resizes the sidebar within 200 to 480', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      double width() => tester.getSize(key('sidebar')).width;
      expect(width(), 240);
      await tester.drag(
        key('sidebar-resize'),
        const Offset(100, 0),
        kind: PointerDeviceKind.mouse,
      );
      await settle(tester);
      expect(width(), closeTo(340, 1));
      expect(s.app.tree.sidebarWidth, closeTo(340, 1));
      await tester.drag(
        key('sidebar-resize'),
        const Offset(600, 0),
        kind: PointerDeviceKind.mouse,
      );
      await settle(tester);
      expect(width(), 480);
      await tester.drag(
        key('sidebar-resize'),
        const Offset(-600, 0),
        kind: PointerDeviceKind.mouse,
      );
      await settle(tester);
      expect(width(), 200);
      // The width is kept when a page opens.
      await tapAndSettle(tester, row(s.pages['Budget']!));
      expect(width(), 200);
    });

    appTest(
      'collapse hides the sidebar; the menu button and Ctrl+\\ bring it back',
      (tester) async {
        await pumpApp(tester, mobile: false);
        expect(key('sidebar-open'), findsNothing);
        final g = await hoverAt(tester, key('sidebar-search'));
        await tapAndSettle(tester, key('sidebar-collapse'));
        await g.removePointer();
        expect(key('sidebar'), findsNothing);
        await tapAndSettle(tester, key('sidebar-open'));
        expect(key('sidebar'), findsOneWidget);
        expect(key('sidebar-open'), findsNothing);

        await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
        await tester.sendKeyEvent(LogicalKeyboardKey.backslash);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
        await settle(tester);
        expect(key('sidebar'), findsNothing);
        await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
        await tester.sendKeyEvent(LogicalKeyboardKey.backslash);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
        await settle(tester);
        expect(key('sidebar'), findsOneWidget);
      },
    );

    appTest('Move to excludes the page and its children and moves it', (
      tester,
    ) async {
      final s = await pumpApp(tester, mobile: false);
      final recipes = s.pages['Recipes']!, budget = s.pages['Budget']!;
      await openRowMenu(tester, recipes);
      await tapAndSettle(tester, key('tree-menu-move'));
      expect(key('move-to'), findsOneWidget);
      expect(key('move-to-root'), findsOneWidget);
      expect(key('move-pick-$recipes'), findsNothing);
      expect(key('move-pick-${s.pages['Soups']}'), findsNothing);
      await tester.enterText(fieldIn(key('move-to-field')), 'soups');
      await settle(tester);
      expect(key('move-pick-${s.pages['Soups']}'), findsNothing);
      await tester.enterText(fieldIn(key('move-to-field')), 'budget');
      await settle(tester);
      await tapAndSettle(tester, key('move-pick-$budget'));
      expect(key('move-to'), findsNothing);
      expect((await pageOf(tester, recipes)).parent, budget);
      expect(row(recipes), findsOneWidget, reason: 'the target expands');

      // Back to the top level.
      await openRowMenu(tester, recipes);
      await tapAndSettle(tester, key('tree-menu-move'));
      await tapAndSettle(tester, key('move-to-root'));
      expect((await pageOf(tester, recipes)).parent, isNull);
    });
  });

  group('home', () {
    appTest(
      'Today keeps page visits in newest-first order and sidebar pages reopen',
      (tester) async {
        final s = await pumpApp(tester, mobile: false);
        expect(key('today-date-strip'), findsOneWidget);
        expect(key('today-selected-heading'), findsOneWidget);
        expect(find.text('Recent'), findsNothing);
        final budget = s.pages['Budget']!;
        final weekly = s.pages['Weekly review']!;
        await tapAndSettle(tester, row(budget));
        await tapAndSettle(tester, key('back'));
        await tapAndSettle(tester, row(weekly));
        await tapAndSettle(tester, key('back'));
        expect(s.app.tree.visited.take(2), [weekly, budget]);
        expect(key('today-date-strip'), findsOneWidget);
        await tapAndSettle(tester, row(budget));
        expect(editableWith('Some text about budget.'), findsOneWidget);
        expect(s.app.tree.visited.first, budget);
      },
    );

    appTest('search results show the page path', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await expandAllPages(tester);
      final soups = s.pages['Soups']!;
      await tester.enterText(fieldIn(key('search-field')), 'soups');
      await settle(tester);
      expect(key('hit-$soups'), findsOneWidget);
      expect(tester.widget<Text>(key('hit-path-$soups')).data, 'Recipes');
      // A top-level page has no path line.
      await tester.enterText(fieldIn(key('search-field')), 'budget');
      await settle(tester);
      final budget = s.pages['Budget']!;
      expect(key('hit-$budget'), findsOneWidget);
      expect(key('hit-path-$budget'), findsNothing);
      expect(
        find.descendant(
          of: key('hit-$budget'),
          matching: find.text('Some text about budget.'),
        ),
        findsOneWidget,
      );
    });

    appTest('mobile dock: tree, search with paths, trash, new page', (
      tester,
    ) async {
      final s = await pumpApp(tester, mobile: true);
      expect(key('mobile-dock'), findsOneWidget);
      for (final k in [
        'nav-pages',
        'nav-today',
        'nav-calendar',
        'nav-search',
        'mobile-create',
      ]) {
        expect(key(k), findsOneWidget, reason: k);
      }
      // Home is the only root screen, so the dock has no Home button.
      expect(key('nav-home'), findsNothing);
      expect(key('nav-trash'), findsNothing);
      expect(key('home-settings'), findsNothing);
      expect(key('today-selected-heading'), findsOneWidget);

      await tapAndSettle(tester, key('nav-search'));
      final breads = s.pages['Breads']!;
      await tester.enterText(fieldIn(key('search-field')), 'breads');
      await settle(tester);
      expect(tester.widget<Text>(key('hit-path-$breads')).data, 'Recipes');
      await tapAndSettle(tester, key('hit-$breads'));
      expect(key('page-title'), findsOneWidget);
      await tapAndSettle(tester, key('back'));
      // The last step back lands on Home.
      expect(key('page-title'), findsNothing);
      expect(key('mobile-dock'), findsOneWidget);
      expect(key('today-selected-heading'), findsOneWidget);

      await core(tester, (v) => v.deletePage(s.pages['Budget']!));
      await openMobileTree(tester);
      await tester.ensureVisible(key('pages-trash'));
      await tapAndSettle(tester, key('pages-trash'));
      expect(key('trash-panel'), findsOneWidget);
      expect(key('trash-row-${s.pages['Budget']}'), findsOneWidget);
      await tapAndSettle(tester, key('screen-back'));
      expect(key('trash-panel'), findsNothing);
      // Trash returns to the tree panel, which is still open.
      expect(key('pages-list'), findsOneWidget);
      await closeMobileTreeWithBack(tester);
      expect(key('pages-list'), findsNothing);

      await openMobileTree(tester);
      await tapAndSettle(tester, key('new-page'));
      expect(key('page-title'), findsOneWidget);
    });

    appTest('mobile tree closes with Android back', (tester) async {
      await pumpApp(tester, mobile: true, fresh: true);
      await openMobileTree(tester);
      expect(key('pages-list'), findsOneWidget);
      await closeMobileTreeWithBack(tester);
      expect(key('pages-list'), findsNothing);
      expect(key('mobile-dock'), findsOneWidget);
    });

    appTest('mobile tree closes with a scrim tap', (tester) async {
      await pumpApp(tester, mobile: true, fresh: true);
      await openMobileTree(tester);
      expect(key('pages-list'), findsOneWidget);
      await closeMobileTreeWithScrim(tester);
      expect(key('pages-list'), findsNothing);
      expect(key('today-selected-heading'), findsOneWidget);
    });

    appTest('a page opened from the tree returns to Home, not the tree', (
      tester,
    ) async {
      await pumpApp(tester, mobile: true, fresh: true);
      await openMobileTree(tester);
      await tapAndSettle(
        tester,
        find.descendant(
          of: key('pages-list'),
          matching: find.text(gettingStartedTitle),
        ),
      );
      expect(
        tester.widget<EditableText>(key('page-title')).controller.text,
        gettingStartedTitle,
      );
      await tapAndSettle(tester, key('back'));
      expect(key('page-title'), findsNothing);
      expect(key('pages-list'), findsNothing);
      expect(key('capture-field'), findsOneWidget);
      expect(key('mobile-dock'), findsOneWidget);
    });
  });

  group('trash', () {
    appTest('restore puts the page back in the tree; the old parent shows', (
      tester,
    ) async {
      final s = await pumpApp(tester, mobile: false);
      final soups = s.pages['Soups']!, budget = s.pages['Budget']!;
      await core(tester, (v) => v.deletePage(soups));
      await core(tester, (v) => v.deletePage(budget));
      await settleUntil(tester, () => row(budget).evaluate().isEmpty);
      expect(row(budget), findsNothing);
      await tapAndSettle(tester, key('sidebar-trash'));
      expect(key('trash-panel'), findsOneWidget);
      expect(key('trash-row-$soups'), findsOneWidget);
      expect(
        tester.widget<Text>(key('trash-path-$soups')).data,
        contains('Recipes'),
      );
      await tapAndSettle(tester, key('trash-restore-$budget'));
      await settleUntil(
        tester,
        () => key('trash-row-$budget').evaluate().isEmpty,
      );
      expect(key('trash-row-$budget'), findsNothing);
      expect((await pageOf(tester, budget)).inTrash, isFalse);
      expect(row(budget), findsOneWidget);
    });

    appTest('delete forever asks first', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final budget = s.pages['Budget']!;
      await core(tester, (v) => v.deletePage(budget));
      await tapAndSettle(tester, key('sidebar-trash'));
      await tapAndSettle(tester, key('trash-delete-$budget'));
      expect(key('confirm-ok'), findsOneWidget);
      await tapAndSettle(tester, key('confirm-cancel'));
      expect(key('trash-row-$budget'), findsOneWidget);
      await tapAndSettle(tester, key('trash-delete-$budget'));
      await tapAndSettle(tester, key('confirm-ok'));
      await settleUntil(
        tester,
        () => key('trash-row-$budget').evaluate().isEmpty,
      );
      expect(key('trash-empty'), findsOneWidget);
      expect(await core(tester, (v) => v.listTrash()), isEmpty);
    });

    appTest('search filters the trash', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final budget = s.pages['Budget']!, garden = s.pages['Garden plan']!;
      await core(tester, (v) => v.deletePage(budget));
      await core(tester, (v) => v.deletePage(garden));
      await tapAndSettle(tester, key('sidebar-trash'));
      expect(key('trash-row-$budget'), findsOneWidget);
      expect(key('trash-row-$garden'), findsOneWidget);
      await tester.enterText(fieldIn(key('trash-search')), 'gard');
      await settle(tester);
      expect(key('trash-row-$budget'), findsNothing);
      expect(key('trash-row-$garden'), findsOneWidget);
    });
  });

  group('history', () {
    appTest('a version previews read-only and restores', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final garden = s.pages['Garden plan']!;
      // A second device edits the page, so there are two versions.
      final other = Directory.systemTemp.createTempSync('notes_other_');
      await tester.runAsync(() async {
        final b = await Vault.open(
          '${other.path}${Platform.pathSeparator}vault',
        );
        final a = s.vault;
        await b.run(
          (x) async => x.importUpdates(
            page: garden,
            bytes: await a.run(
              (y) => y.exportUpdates(page: garden, since: const []),
            ),
          ),
        );
        final blocks = (await b.loadPage(garden)).blocks;
        final first = blocks.first;
        await b.apply(garden, [
          BOp(
            op: OpKind.setText,
            id: first.id,
            kind: first.kind,
            text: 'Edited on the phone.',
            marks: const [],
            checked: false,
          ),
        ]);
        final back = await b.run(
          (x) => x.exportUpdates(page: garden, since: const []),
        );
        await a.run(
          (x) => x.importUpdates(page: garden, bytes: back),
          writes: true,
        );
        await b.flush();
        b.dispose();
      });
      await settle(tester);
      expect(
        (await pageOf(tester, garden)).blocks.first.text,
        'Edited on the phone.',
      );

      showVersionHistory(homeContext(tester), garden);
      await settle(tester);
      expect(key('history-panel'), findsOneWidget);
      expect(key('history-version-0'), findsOneWidget);
      expect(key('history-version-1'), findsOneWidget);
      expect(
        find.descendant(
          of: key('history-preview'),
          matching: find.text('Edited on the phone.'),
        ),
        findsOneWidget,
      );
      await tapAndSettle(tester, key('history-version-1'), waitForBridge: true);
      expect(
        find.descendant(
          of: key('history-preview'),
          matching: find.text('Some text about garden plan.'),
        ),
        findsOneWidget,
      );
      // The real block widgets, all read-only.
      final editables = find.descendant(
        of: key('history-preview'),
        matching: find.byType(EditableText),
      );
      expect(editables, findsWidgets);
      for (final e in tester.widgetList<EditableText>(editables)) {
        expect(e.readOnly, isTrue);
      }
      expect(
        find.descendant(
          of: key('history-preview'),
          matching: find.byType(BlockRow),
        ),
        findsWidgets,
      );
      // Showing an old version changes nothing.
      expect(
        (await pageOf(tester, garden)).blocks.first.text,
        'Edited on the phone.',
      );
      await tapAndSettle(tester, key('history-restore'));
      expect(key('history-panel'), findsNothing);
      expect(
        (await pageOf(tester, garden)).blocks.first.text,
        'Some text about garden plan.',
      );
      try {
        other.deleteSync(recursive: true);
      } catch (_) {}
    });

    appTest('mobile history is a full screen with the list on top', (
      tester,
    ) async {
      final s = await pumpApp(tester, mobile: true);
      showVersionHistory(homeContext(tester), s.pages['Budget']!);
      await settle(tester);
      expect(key('history-panel'), findsOneWidget);
      expect(tester.getSize(key('history-panel')).width, 375);
      expect(
        tester.getTopLeft(key('history-version-0')).dy,
        lessThan(tester.getTopLeft(key('history-preview')).dy),
      );
      await tapAndSettle(tester, key('screen-back'));
      expect(key('history-panel'), findsNothing);
    });
  });

  group('import and export', () {
    appTest('importing a folder of two markdown files makes two pages', (
      tester,
    ) async {
      final s = await pumpApp(tester, mobile: false);
      final dir = Directory.systemTemp.createTempSync('notes_import_');
      File('${dir.path}${Platform.pathSeparator}Alpha note.md')
          .writeAsStringSync('# Alpha note\n\nFirst file.\n');
      File('${dir.path}${Platform.pathSeparator}Beta note.md')
          .writeAsStringSync('Second file with a [[Alpha note]] link.\n');
      showImport(homeContext(tester));
      await settle(tester);
      expect(key('import-panel'), findsOneWidget);
      await tester.enterText(
        fieldIn(key('import-path')),
        '${dir.path}${Platform.pathSeparator}missing',
      );
      await tapAndSettle(tester, key('import-run'));
      expect(
        tester.widget<Text>(key('import-result')).data,
        'That folder does not exist.',
      );
      await tester.enterText(fieldIn(key('import-path')), dir.path);
      await tester.tap(key('import-run'));
      await settleUntil(
        tester,
        () =>
            key('import-result').evaluate().isNotEmpty &&
            (tester.widget<Text>(key('import-result')).data ?? '').startsWith(
              'Imported',
            ),
      );
      expect(
        tester.widget<Text>(key('import-result')).data,
        startsWith('Imported 2 pages'),
      );
      final top = await titlesUnder(tester, null);
      expect(top, containsAll(['Alpha note', 'Beta note']));
      expect(s.pages.length, greaterThan(0));
      try {
        dir.deleteSync(recursive: true);
      } catch (_) {}
    });

    appTest('export writes the page as markdown to the typed path', (
      tester,
    ) async {
      final s = await pumpApp(tester, mobile: false);
      final dir = Directory.systemTemp.createTempSync('notes_export_');
      final path =
          '${dir.path}${Platform.pathSeparator}sub${Platform.pathSeparator}garden.md';
      showExport(homeContext(tester), s.pages['Garden plan']!);
      await settle(tester);
      final def = tester
          .widget<EditableText>(fieldIn(key('export-path')))
          .controller
          .text;
      expect(
        def,
        endsWith(
          '${Platform.pathSeparator}Documents${Platform.pathSeparator}Garden plan.md',
        ),
      );
      await tester.enterText(fieldIn(key('export-path')), path);
      await tester.tap(key('export-run'));
      await settleUntil(
        tester,
        () =>
            File(path).existsSync() &&
            key('export-result').evaluate().isNotEmpty,
      );
      final expected = await core(
        tester,
        (v) => pageToMarkdown(v, s.pages['Garden plan']!),
      );
      expect(File(path).readAsStringSync(), expected);
      expect(expected, startsWith('---\n'));
      expect(expected, endsWith('Some text about garden plan.\n'));
      expect(
        tester.widget<Text>(key('export-result')).data,
        'Exported to $path',
      );
      try {
        dir.deleteSync(recursive: true);
      } catch (_) {}
    });

    appTest(
      'both Export panel save paths preserve type data and the readable body',
      (tester) async {
        final s = await pumpApp(tester, mobile: false);
        final page = await core(
          tester,
          (v) => v.createPage(title: 'Readable', pageType: 'mod:example'),
        );
        const body = '\n# Readable 雪\r\n\r\n- ready-made body  \n\n';
        await core(
          tester,
          (v) => v.typeDataSet(page, 'example', ['value'], '"opaque # data"'),
        );
        await core(tester, (v) => v.setTypeMarkdownBody(page, body));
        await core(tester, (v) => v.flush());
        final coreFile = Directory('${s.vault.root}/pages')
            .listSync()
            .whereType<File>()
            .singleWhere((f) => f.path.endsWith('-$page.md'));
        final expected = coreFile.readAsStringSync();
        final dir = Directory.systemTemp.createTempSync('notes_export_body_');
        addTearDown(() => dir.deleteSync(recursive: true));
        final typedPath = '${dir.path}/typed.md';
        final chosenPath = '${dir.path}/chosen.md';

        showExport(homeContext(tester), page);
        await settle(tester);
        await tester.enterText(fieldIn(key('export-path')), typedPath);
        await tester.tap(key('export-run'));
        await settleUntil(
          tester,
          () =>
              File(typedPath).existsSync() &&
              (tester.widget<Text>(key('export-result')).data ?? '').startsWith(
                'Exported',
              ),
        );
        expect(File(typedPath).readAsBytesSync(), utf8.encode(expected));
        final annotation = expected
            .split('\n')
            .singleWhere((line) => line.startsWith('type-data: '));
        expect(jsonDecode(annotation.substring('type-data: '.length)), {
          r'$markdown': true,
          'example': {'value': 'opaque # data'},
        });
        expect(expected.substring(expected.indexOf('\n---\n', 4) + 5), body);

        dialogs.savePath = chosenPath;
        await tester.tap(key('export-choose'));
        await settleUntil(
          tester,
          () =>
              File(chosenPath).existsSync() &&
              (tester.widget<Text>(key('export-result')).data ?? '') ==
                  'Exported to $chosenPath',
        );
        expect(File(chosenPath).readAsBytesSync(), utf8.encode(expected));
        expect(dialogs.calls, contains('save:Readable.md'));
      },
    );
  });
}
