// A board's readable Markdown, through the real vault. The body is one
// `## <column>` heading per column, in stored order, and under it one
// `- [[<id>|<title>]]` page link per card. The exported page carries the marker
// `"$markdown": true` beside the board's data and has no blocks.
//
// The roundtrip test builds a board whose order is not the default: the Done
// column is moved to the front, and two columns hold several cards with
// reorders in them. It is then read back from its own file in two ways, a
// front-matter spacing rewrite followed by a rescan (the parser path the core
// test uses) and a reopen of the vault, and the stored board and the export
// must match exactly each time.

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'kanban_fixture.dart';

/// Rewrites the `type-data:` line of [file] with extra spaces after the colon,
/// the front-matter-only edit that forces the rescan parser to read the file
/// again. The body bytes are untouched.
Future<void> spacingRewrite(WidgetTester tester, File file, String exported) =>
    tester.runAsync(
      () => file.writeAsString(
        exported.replaceFirst('type-data: ', 'type-data:   '),
      ),
    );

/// Points the desktop pointer at the header of [column], where the column's
/// move controls appear on hover.
Future<void> tapColumnControl(
  WidgetTester tester,
  String column,
  Finder control,
) async {
  final pointer = await tester.createGesture(kind: PointerDeviceKind.mouse);
  await pointer.addPointer(location: Offset.zero);
  try {
    final box = tester.getRect(columnNamed(column));
    await pointer.moveTo(Offset(box.center.dx, box.top + 24));
    await settle(tester);
    await tapAndSettle(tester, control);
  } finally {
    await pointer.removePointer();
  }
}

/// The id of the child page of [board] titled [title].
Future<String> childTitled(
  WidgetTester tester,
  String board,
  String title,
) async {
  final children = await core(tester, (v) => v.listChildren(board));
  return children.firstWhere((p) => p.title == title).id;
}

void main() {
  setUpAll(initCore);

  appTest(
    'the readable body lists each column and its linked cards in order, after each board change',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final board = await newKanbanBoard(tester);
      await expectOutline(tester, board, [
        headingLine('To do'),
        headingLine('Doing'),
        headingLine('Done'),
      ]);

      // A new card in To do (create).
      await tapAndSettle(tester, addButton('To do'));
      await typeCardTitle(tester, 'Plan trip');
      final plan = await onlyChildOf(tester, board);
      await expectOutline(tester, board, [
        headingLine('To do'),
        cardLinkLine(plan, 'Plan trip'),
        headingLine('Doing'),
        headingLine('Done'),
      ]);

      // An existing note added through the picker goes to the end of To do.
      await addExisting(tester, 'To do', garden, 'Garden plan');
      await expectOutline(tester, board, [
        headingLine('To do'),
        cardLinkLine(plan, 'Plan trip'),
        cardLinkLine(garden, 'Garden plan'),
        headingLine('Doing'),
        headingLine('Done'),
      ]);

      // A drag to the end of Doing moves the note across columns.
      await dragCard(
        tester,
        'Garden plan',
        tester.getCenter(addButton('Doing')),
      );
      await expectOutline(tester, board, [
        headingLine('To do'),
        cardLinkLine(plan, 'Plan trip'),
        headingLine('Doing'),
        cardLinkLine(garden, 'Garden plan'),
        headingLine('Done'),
      ]);

      // Removing a card from the board leaves its note, and the body drops it.
      await openCardMenu(tester, 'Garden plan');
      await tapAndSettle(tester, find.byKey(removeCardKey));
      await expectOutline(tester, board, [
        headingLine('To do'),
        cardLinkLine(plan, 'Plan trip'),
        headingLine('Doing'),
        headingLine('Done'),
      ]);
      final top = await core(tester, (v) => v.listChildren(null));
      expect(
        top.any((row) => row.id == garden),
        isTrue,
        reason: 'the note stays',
      );
    },
  );

  appTest(
    'a board with a non-default column order and reordered cards reads back identically after a rescan and after a reopen',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final board = await newKanbanBoard(tester);

      // Done moves to the front: left twice, from the end of [To do, Doing, Done].
      final done = columnIdOf(await storedKanban(tester, board), 'Done');
      for (var i = 0; i < 2; i++) {
        await tapColumnControl(
          tester,
          'Done',
          find.byKey(ValueKey('board-column-left-$done')),
        );
      }
      expect(storedTitles(await storedKanban(tester, board)), [
        'Done',
        'To do',
        'Doing',
      ]);

      // Three cards in To do, then Charlie dragged in front of Alpha.
      for (final title in ['Alpha', 'Bravo', 'Charlie']) {
        await tapAndSettle(tester, addButton('To do'));
        await typeCardTitle(tester, title);
      }
      final alpha = await childTitled(tester, board, 'Alpha');
      await dragCard(
        tester,
        'Charlie',
        tester.getCenter(find.byKey(ValueKey('drop-$alpha'))),
      );
      final bravo = await childTitled(tester, board, 'Bravo');
      final charlie = await childTitled(tester, board, 'Charlie');

      // Done holds an existing note and a new card; the new card is dragged in
      // front of the note.
      await addExisting(tester, 'Done', garden, 'Garden plan');
      await tapAndSettle(tester, addButton('Done'));
      await typeCardTitle(tester, 'Delta');
      final delta = await childTitled(tester, board, 'Delta');
      await dragCard(
        tester,
        'Delta',
        tester.getCenter(find.byKey(ValueKey('drop-$garden'))),
      );

      final before = await storedKanban(tester, board);
      expect(storedTitles(before), ['Done', 'To do', 'Doing']);
      expect(storedCards(before, 'To do'), [charlie, alpha, bravo]);
      expect(storedCards(before, 'Done'), [delta, garden]);
      expect(storedCards(before, 'Doing'), isEmpty);
      final beforeBody = await core(tester, (v) => v.typeMarkdownBody(board));
      expect(outlineOf(beforeBody!), [
        headingLine('Done'),
        cardLinkLine(delta, 'Delta'),
        cardLinkLine(garden, 'Garden plan'),
        headingLine('To do'),
        cardLinkLine(charlie, 'Charlie'),
        cardLinkLine(alpha, 'Alpha'),
        cardLinkLine(bravo, 'Bravo'),
        headingLine('Doing'),
      ]);

      final exported = await core(tester, (v) => v.exportMarkdown(board));
      await core(tester, (v) => v.flush());
      final file = await pageFileOf(tester, board);
      expect(await tester.runAsync(() => file.readAsString()), exported);

      // Rescan path: a front-matter-only edit, then a rescan that reads no content.
      await spacingRewrite(tester, file, exported);
      expect(
        await core(tester, (v) => v.run((r) => r.rescan(), writes: true)),
        0,
      );
      expect(await storedKanban(tester, board), before);
      expect(await core(tester, (v) => v.typeMarkdownBody(board)), beforeBody);
      expect(await core(tester, (v) => v.exportMarkdown(board)), exported);
      expect((await core(tester, (v) => v.loadPage(board))).blocks, isEmpty);

      // Reopen path: the same page id, the same board, the same bytes.
      await restartApp(tester, restoreLastPage: false);
      expect((await core(tester, (v) => v.loadPage(board))).id, board);
      expect((await core(tester, (v) => v.loadPage(board))).blocks, isEmpty);
      expect(await storedKanban(tester, board), before);
      expect(await core(tester, (v) => v.typeMarkdownBody(board)), beforeBody);
      // The vault leaves a content-equal file as the user wrote it, so the
      // front-matter spacing edit stays on disk; exports are byte-identical.
      final reopenedFile = await pageFileOf(tester, board);
      expect(
        await tester.runAsync(() => reopenedFile.readAsString()),
        exported.replaceFirst('type-data: ', 'type-data:   '),
      );
      final firstExport = await core(tester, (v) => v.exportMarkdown(board));
      expect(firstExport, exported);
      final secondExport = await core(tester, (v) => v.exportMarkdown(board));
      expect(
        secondExport,
        firstExport,
        reason: 'a second export is byte-identical',
      );

      // And the rescan path once more on the reopened vault.
      await spacingRewrite(tester, reopenedFile, exported);
      expect(
        await core(tester, (v) => v.run((r) => r.rescan(), writes: true)),
        0,
      );
      expect(await storedKanban(tester, board), before);
      expect(await core(tester, (v) => v.exportMarkdown(board)), exported);
    },
  );
}
