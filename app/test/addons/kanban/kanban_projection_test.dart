// The readable body follows the board on every board change: adding, renaming,
// moving and deleting columns (through the phone's column controls), and adding
// and moving cards. A card's note renamed elsewhere shows in the body only at
// the next local board change. A card whose note is missing or trashed is left
// out of the body at the next local board change.

import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'kanban_fixture.dart';

/// Adds the existing page [id] to the end of [column] through the picker,
/// searching for [search], and waits until the stored order holds it.
Future<void> pickUntilStored(
  WidgetTester tester,
  String board,
  String column,
  String id,
  String search,
) async {
  await tapAndSettle(tester, addButton(column));
  await tapAndSettle(tester, find.text(addExistingLabel));
  await tester.enterText(
    find.descendant(
      of: find.byKey(const ValueKey('page-picker-field')),
      matching: find.byType(EditableText),
    ),
    search,
  );
  await settle(tester);
  await tapAndSettle(tester, find.byKey(ValueKey('page-pick-$id')));
  for (var i = 0; i < 40; i++) {
    final cards = storedCards(await storedKanban(tester, board), column);
    if (cards.contains(id)) break;
    await settle(tester, min: Duration.zero);
  }
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  appTest(
    'phone column add, rename, move and delete each rewrite the body, and a deleted column moves its cards to the first remaining column',
    (tester) async {
      await pumpApp(tester, mobile: true);
      final board = await newKanbanBoard(tester);
      await expectOutline(tester, board, [
        headingLine('To do'),
        headingLine('Doing'),
        headingLine('Done'),
      ]);

      // Add: a new column at the end.
      await revealAndTap(
        tester,
        find.byKey(const ValueKey('board-add-column')),
      );
      await expectOutline(tester, board, [
        headingLine('To do'),
        headingLine('Doing'),
        headingLine('Done'),
        headingLine('New column'),
      ]);
      final added = columnIdOf(await storedKanban(tester, board), 'New column');

      // Rename it.
      await revealAndTap(tester, columnMenu('New column'));
      await revealAndTap(tester, find.byKey(const ValueKey('group-rename')));
      await tester.enterText(
        find.byKey(ValueKey('board-column-title-$added')),
        'Ideas',
      );
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await settle(tester);
      await expectOutline(tester, board, [
        headingLine('To do'),
        headingLine('Doing'),
        headingLine('Done'),
        headingLine('Ideas'),
      ]);

      // Move Ideas one place left, then To do one place right.
      await revealAndTap(
        tester,
        find.byKey(ValueKey('board-column-left-$added')),
      );
      await expectOutline(tester, board, [
        headingLine('To do'),
        headingLine('Doing'),
        headingLine('Ideas'),
        headingLine('Done'),
      ]);
      final todo = columnIdOf(await storedKanban(tester, board), 'To do');
      await revealAndTap(
        tester,
        find.byKey(ValueKey('board-column-right-$todo')),
      );
      await expectOutline(tester, board, [
        headingLine('Doing'),
        headingLine('To do'),
        headingLine('Ideas'),
        headingLine('Done'),
      ]);

      // A card in Doing, then delete Doing: its card moves to the end of the
      // first remaining column, To do.
      await revealAndTap(tester, addButton('Doing'));
      await typeCardTitle(tester, 'Budget');
      final budget = await onlyChildOf(tester, board);
      await expectOutline(tester, board, [
        headingLine('Doing'),
        cardLinkLine(budget, 'Budget'),
        headingLine('To do'),
        headingLine('Ideas'),
        headingLine('Done'),
      ]);

      await revealAndTap(tester, columnMenu('Doing'));
      await revealAndTap(tester, find.byKey(const ValueKey('group-delete')));
      await revealAndTap(tester, find.byKey(const ValueKey('confirm-ok')));
      await expectOutline(tester, board, [
        headingLine('To do'),
        cardLinkLine(budget, 'Budget'),
        headingLine('Ideas'),
        headingLine('Done'),
      ]);
      expect(storedCards(await storedKanban(tester, board), 'To do'), [budget]);
    },
  );

  appTest(
    'a note renamed elsewhere leaves the body showing the old title until the next board change',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final board = await newKanbanBoard(tester);
      await addExisting(tester, 'To do', garden, 'Garden plan');
      await expectOutline(tester, board, [
        headingLine('To do'),
        cardLinkLine(garden, 'Garden plan'),
        headingLine('Doing'),
        headingLine('Done'),
      ]);

      await core(tester, (v) => v.renamePage(garden, 'Garden plan v2'));
      await settleUntil(
        tester,
        () => cardIn('To do', 'Garden plan v2').evaluate().isNotEmpty,
      );
      // The board shows the new title; the readable body has not been rewritten.
      final stale = await core(tester, (v) => v.typeMarkdownBody(board));
      expect(outlineOf(stale!), [
        headingLine('To do'),
        cardLinkLine(garden, 'Garden plan'),
        headingLine('Doing'),
        headingLine('Done'),
      ], reason: 'the body refreshes at the next local board write');

      // The next board change refreshes it.
      await tapAndSettle(tester, addButton('Done'));
      await typeCardTitle(tester, 'Tidy up');
      final tidy = await onlyChildOf(tester, board);
      await expectOutline(tester, board, [
        headingLine('To do'),
        cardLinkLine(garden, 'Garden plan v2'),
        headingLine('Doing'),
        headingLine('Done'),
        cardLinkLine(tidy, 'Tidy up'),
      ]);
    },
  );

  appTest(
    'a card whose note does not exist, or is trashed, is left out at the next board change',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final board = await newKanbanBoard(tester);
      await tapAndSettle(tester, addButton('To do'));
      await typeCardTitle(tester, 'Plan trip');
      final plan = await onlyChildOf(tester, board);
      await tapAndSettle(tester, addButton('To do'));
      await typeCardTitle(tester, 'Pack bags');
      final pack = (await core(
        tester,
        (v) => v.listChildren(board),
      )).firstWhere((p) => p.title == 'Pack bags').id;

      // A stored id with no page behind it, appended after the real cards.
      const ghost = 'ghost-note-that-does-not-exist';
      final todo = columnIdOf(await storedKanban(tester, board), 'To do');
      await core(
        tester,
        (v) => v.typeDataListInsert(board, kanbanAddonId, [
          'cards',
          todo,
        ], jsonEncode(ghost)),
      );
      // A card trashed on its own.
      await core(tester, (v) => v.deletePage(pack));

      // The next local board change rewrites the body without either card.
      await tapAndSettle(tester, addButton('Done'));
      await typeCardTitle(tester, 'Tidy up');
      final tidy = (await core(
        tester,
        (v) => v.listChildren(board),
      )).firstWhere((p) => p.title == 'Tidy up').id;
      await expectOutline(tester, board, [
        headingLine('To do'),
        cardLinkLine(plan, 'Plan trip'),
        headingLine('Doing'),
        headingLine('Done'),
        cardLinkLine(tidy, 'Tidy up'),
      ]);
      final body = (await core(tester, (v) => v.typeMarkdownBody(board)))!;
      expect(body, isNot(contains(ghost)));
      expect(body, isNot(contains(pack)));
    },
  );

  appTest(
    'titles with Markdown metacharacters or a newline are escaped in the link label, and duplicate titles keep their own ids',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final meta = await core(
        tester,
        (v) => v.createPage(title: r'Plan *draft* [v2] a_b`c~<x>'),
      );
      final nl = await core(
        tester,
        (v) => v.createPage(title: 'Line one\nLine two'),
      );
      final d1 = await core(tester, (v) => v.createPage(title: 'Same name'));
      final d2 = await core(tester, (v) => v.createPage(title: 'Same name'));
      await core(tester, (v) => v.flush());

      final board = await newKanbanBoard(tester);
      await pickUntilStored(tester, board, 'To do', meta, 'draft');
      await pickUntilStored(tester, board, 'To do', nl, 'Line one');
      await pickUntilStored(tester, board, 'To do', d1, 'Same name');
      await pickUntilStored(tester, board, 'To do', d2, 'Same name');
      expect(storedCards(await storedKanban(tester, board), 'To do'), [
        meta,
        nl,
        d1,
        d2,
      ]);

      // Core escaping: each of \*_`[]~<> is backslash-escaped in the label. The
      // link target is the page id, so the two 'Same name' links differ.
      const metaLabel = r'Plan \*draft\* \[v2\] a\_b\`c\~\<x\>';
      final expected = [
        '## To do',
        '',
        '- [[$meta|$metaLabel]]',
        '- [[$nl|Line one Line two]]',
        '- [[$d1|Same name]]',
        '- [[$d2|Same name]]',
        '',
        '## Doing',
        '',
        '## Done',
        '',
      ].join('\n');

      for (var i = 0; i < 40; i++) {
        final body = await core(tester, (v) => v.typeMarkdownBody(board));
        if (body == expected) break;
        await settle(tester, min: Duration.zero);
      }
      expect(await core(tester, (v) => v.typeMarkdownBody(board)), expected);

      final exported = await core(tester, (v) => v.exportMarkdown(board));
      expect(bodyOf(exported), expected);
    },
  );
}
