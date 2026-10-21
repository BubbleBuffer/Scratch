// Moving cards and changing columns on a board, through the kit's drag and
// column controls. Each change is checked in the stored order and on
// screen. Deleting a column relocates its cards to the first remaining
// column, and the notes themselves are never removed.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'kanban_fixture.dart';

/// A point inside [column]'s add button that stays on screen: a phone shows
/// about one column, so a drop there lands near the column's left edge.
Offset dropPoint(WidgetTester tester, String column) {
  final box = tester.getRect(addButton(column));
  return Offset(box.left + 40, box.center.dy);
}

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    appTest(
      '${mobile ? 'mobile long press' : 'desktop drag'} moves a card into the next column at the end',
      (tester) async {
        final f = await pumpApp(tester, mobile: mobile);
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

        await dragCard(
          tester,
          'Garden plan',
          dropPoint(tester, 'Doing'),
          mobile: mobile,
        );

        final data = await storedKanban(tester, board);
        expect(storedCards(data, 'To do'), [recipes]);
        expect(storedCards(data, 'Doing'), [budget, garden]);
        expect(cardIn('Doing', 'Garden plan'), findsOneWidget);
        expect(cardIn('To do', 'Garden plan'), findsNothing);
        final note = await core(tester, (v) => v.loadPage(garden));
        expect(note.id, garden);
      },
    );
  }

  appTest(
    'dropping a card on another card places it before that card, and on the add button at the end',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final recipes = f.pages['Recipes']!;
      final budget = f.pages['Budget']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden, recipes, budget],
          'Doing': [],
          'Done': [],
        },
      );
      await openNote(tester, board, 'Launch board');

      await dragCard(
        tester,
        'Budget',
        tester.getCenter(find.byKey(ValueKey('drop-$garden'))),
      );
      expect(storedCards(await storedKanban(tester, board), 'To do'), [
        budget,
        garden,
        recipes,
      ]);
      expect(cardIn('To do', 'Budget'), findsOneWidget);

      await dragCard(tester, 'Budget', tester.getCenter(addButton('To do')));
      expect(storedCards(await storedKanban(tester, board), 'To do'), [
        garden,
        recipes,
        budget,
      ]);
      expect(cardIn('To do', 'Budget'), findsOneWidget);
    },
  );

  appTest(
    'adding, renaming and moving columns stores the new order and keeps the cards',
    (tester) async {
      final f = await pumpApp(tester, mobile: true);
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

      await revealAndTap(
        tester,
        find.byKey(const ValueKey('board-add-column')),
      );
      var data = await storedKanban(tester, board);
      expect(storedTitles(data), [...starterColumns, 'New column']);
      final added = columnIdOf(data, 'New column');

      await revealAndTap(tester, columnMenu('New column'));
      await revealAndTap(tester, find.byKey(const ValueKey('group-rename')));
      await tester.enterText(
        find.byKey(ValueKey('board-column-title-$added')),
        'Ideas',
      );
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await settle(tester);
      data = await storedKanban(tester, board);
      expect(storedTitles(data), ['To do', 'Doing', 'Done', 'Ideas']);

      // Ideas moves one place left, then To do one place right.
      await revealAndTap(
        tester,
        find.byKey(ValueKey('board-column-left-$added')),
      );
      data = await storedKanban(tester, board);
      expect(storedTitles(data), ['To do', 'Doing', 'Ideas', 'Done']);

      final todo = columnIdOf(data, 'To do');
      await revealAndTap(
        tester,
        find.byKey(ValueKey('board-column-right-$todo')),
      );
      data = await storedKanban(tester, board);
      expect(storedTitles(data), ['Doing', 'To do', 'Ideas', 'Done']);
      expect(columnsOnScreen(tester), ['Doing', 'To do', 'Ideas', 'Done']);
      expect(storedCards(data, 'To do'), [garden]);
      expect(cardIn('To do', 'Garden plan'), findsOneWidget);
    },
  );

  appTest(
    'deleting a column moves its cards to the first remaining column and keeps the notes',
    (tester) async {
      final f = await pumpApp(tester, mobile: true);
      final garden = f.pages['Garden plan']!;
      final recipes = f.pages['Recipes']!;
      final budget = f.pages['Budget']!;
      final reading = f.pages['Reading notes']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden],
          'Doing': [recipes, budget],
          'Done': [reading],
        },
      );
      await openNote(tester, board, 'Launch board');

      await revealAndTap(tester, columnMenu('Doing'));
      await revealAndTap(tester, find.byKey(const ValueKey('group-delete')));
      expect(find.text('Delete "Doing"?'), findsOneWidget);
      await revealAndTap(tester, find.byKey(const ValueKey('confirm-ok')));
      expect(columnNamed('Doing'), findsNothing);

      final data = await storedKanban(tester, board);
      expect(storedTitles(data), ['To do', 'Done']);
      expect(storedCards(data, 'To do'), [garden, recipes, budget]);
      expect(storedCards(data, 'Done'), [reading]);
      expect(cardIn('To do', 'Budget'), findsOneWidget);
      expect(cardIn('To do', 'Recipes'), findsOneWidget);
      for (final id in [recipes, budget]) {
        final note = await core(tester, (v) => v.loadPage(id));
        expect(note.id, id);
        expect(note.inTrash, isFalse);
        expect(note.parent, isNull);
      }
    },
  );

  appTest('the last remaining column cannot be deleted', (tester) async {
    await pumpApp(tester, mobile: true);
    final board = await seedKanban(
      tester,
      title: 'Launch board',
      columns: {'To do': []},
    );
    await openNote(tester, board, 'Launch board');

    await revealAndTap(tester, columnMenu('To do'));
    expect(find.byKey(const ValueKey('group-delete')), findsNothing);
    expect(storedTitles(await storedKanban(tester, board)), ['To do']);
  });
}
