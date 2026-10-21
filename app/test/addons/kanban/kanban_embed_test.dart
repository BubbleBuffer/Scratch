// A board inside another note, the board's data after a restart, and changes
// that reach an open board from the change feed: a renamed note updates its
// card, a card stored by another writer appears, and a trashed note is hidden
// until it is restored (its id stays in the stored board).

import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/embed_block.dart';
import 'package:notes_app/ui/board/board_renderer.dart';

import 'kanban_fixture.dart';

void main() {
  setUpAll(initCore);

  appTest(
    'a board embedded in another note draws its columns and cards in that note',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden],
          'Doing': [],
          'Done': [],
        },
      );
      final host = await core(
        tester,
        (v) => v.createPage(title: 'Weekly plan'),
      );
      const embedId = 'launch-embed';
      await core(
        tester,
        (v) => v.apply(host, [
          const BOp(
            op: OpKind.insert,
            id: embedId,
            kind: embedKind,
            text: '',
            marks: [],
            checked: false,
          ),
          BOp(
            op: OpKind.setTarget,
            id: embedId,
            kind: embedKind,
            text: '',
            marks: const [],
            checked: false,
            target: board,
          ),
        ]),
      );

      await openNote(tester, host, 'Weekly plan');
      final embed = find.byKey(const ValueKey('embed-$embedId'));
      expect(embed, findsOneWidget);

      // The embed draws the kit board inline (not a card or a summary).
      final inline = inlineBoardIn(embed);
      expect(inline, findsOneWidget);
      expect(tester.widget<BoardRenderer>(inline).inline, isTrue);
      for (final title in starterColumns) {
        expect(
          find.descendant(of: inline, matching: columnNamed(title)),
          findsOneWidget,
        );
      }
      expect(
        find.descendant(of: inline, matching: cardNamed('Garden plan')),
        findsOneWidget,
      );
      expect(storedTitles(await storedKanban(tester, board)), starterColumns);
    },
  );

  appTest(
    'the board after a restart keeps the cards and columns moved on the full page',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final recipes = f.pages['Recipes']!;
      final budget = f.pages['Budget']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden, recipes],
          'Doing': [budget],
          'Done': [],
        },
      );
      await openNote(tester, board, 'Launch board');
      await dragCard(tester, 'Recipes', tester.getCenter(addButton('Done')));
      expect(cardIn('Done', 'Recipes'), findsOneWidget);

      await restartApp(tester, restoreLastPage: false);
      await openNote(tester, board, 'Launch board');

      expect(columnsOnScreen(tester), starterColumns);
      expect(cardIn('To do', 'Garden plan'), findsOneWidget);
      expect(cardIn('Done', 'Recipes'), findsOneWidget);
      expect(cardIn('Doing', 'Budget'), findsOneWidget);
      final data = await storedKanban(tester, board);
      expect(storedCards(data, 'To do'), [garden]);
      expect(storedCards(data, 'Done'), [recipes]);
    },
  );

  appTest('a note renamed elsewhere updates its card on an open board', (
    tester,
  ) async {
    final f = await pumpApp(tester, mobile: false);
    final garden = f.pages['Garden plan']!;
    final board = await seedKanban(
      tester,
      title: 'Launch board',
      columns: {
        'To do': [garden],
        'Doing': [],
        'Done': [],
      },
    );
    await openNote(tester, board, 'Launch board');
    expect(cardIn('To do', 'Garden plan'), findsOneWidget);

    await core(tester, (v) => v.renamePage(garden, 'Garden plan v2'));
    await settleUntil(
      tester,
      () => cardIn('To do', 'Garden plan v2').evaluate().isNotEmpty,
    );
    expect(cardIn('To do', 'Garden plan'), findsNothing);
  });

  appTest('a card stored by another writer appears on an open board', (
    tester,
  ) async {
    final f = await pumpApp(tester, mobile: false);
    final garden = f.pages['Garden plan']!;
    final recipes = f.pages['Recipes']!;
    final board = await seedKanban(
      tester,
      title: 'Launch board',
      columns: {
        'To do': [garden],
        'Doing': [],
        'Done': [],
      },
    );
    await openNote(tester, board, 'Launch board');
    expect(cardNamed('Recipes'), findsNothing);

    final todo = columnIdOf(await storedKanban(tester, board), 'To do');
    await core(
      tester,
      (v) => v.typeDataListInsert(board, kanbanAddonId, [
        'cards',
        todo,
      ], jsonEncode(recipes)),
    );
    await settleUntil(
      tester,
      () => cardIn('To do', 'Recipes').evaluate().isNotEmpty,
    );
    expect(storedCards(await storedKanban(tester, board), 'To do'), [
      garden,
      recipes,
    ]);
  });

  appTest(
    'a card whose note is trashed is hidden, its id is kept, and it returns when restored',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden],
          'Doing': [],
          'Done': [],
        },
      );
      await openNote(tester, board, 'Launch board');
      expect(cardIn('To do', 'Garden plan'), findsOneWidget);

      await core(tester, (v) => v.deletePage(garden));
      await settleUntil(
        tester,
        () => cardIn('To do', 'Garden plan').evaluate().isEmpty,
      );
      expect(storedCards(await storedKanban(tester, board), 'To do'), [garden]);

      await core(tester, (v) => v.restorePage(garden));
      await settleUntil(
        tester,
        () => cardIn('To do', 'Garden plan').evaluate().isNotEmpty,
      );
    },
  );
}
