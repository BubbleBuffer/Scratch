// The card ellipsis. On desktop it appears while the pointer is over a card,
// and right-click opens the same menu. On a phone it is always shown, and its
// menu also offers Move to column…, which lists the other columns and appends
// the card to the one chosen. Open and Remove work on both; Remove keeps the
// note itself.

import 'package:flutter_test/flutter_test.dart';

import 'kanban_fixture.dart';

/// Waits until [column] stores exactly [expected] (the write is asynchronous
/// behind the widget), then lets the screen settle.
Future<void> settleStored(
  WidgetTester tester,
  String board,
  String column,
  List<String> expected,
) async {
  for (var i = 0; i < 40; i++) {
    final now = storedCards(await storedKanban(tester, board), column);
    if (now.join(',') == expected.join(',')) break;
    await settle(tester, min: Duration.zero);
  }
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  appTest(
    'desktop: the ellipsis appears when the pointer is over a card and opens the card menu',
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

      // The button is always built; hidden means Opacity 0 around it.
      expect(menuOpacity(tester, garden), 0, reason: 'hidden with no hover');
      await hoverCard(tester, 'Garden plan');
      expect(menuOpacity(tester, garden), 1, reason: 'shown while hovered');

      await tapAndSettle(tester, cardMenuTrigger(garden));
      expect(menuItem('Open'), findsOneWidget);
      expect(menuItem(removeFromBoardLabel), findsOneWidget);
    },
  );

  appTest('desktop: right-click opens the same card menu as the ellipsis', (
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

    await openCardMenu(tester, 'Garden plan');
    expect(menuItem('Open'), findsOneWidget);
    expect(menuItem(removeFromBoardLabel), findsOneWidget);
  });

  appTest('desktop: Open from the ellipsis opens the card note', (
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

    await hoverCard(tester, 'Garden plan');
    await tapAndSettle(tester, cardMenuTrigger(garden));
    await tapAndSettle(tester, menuItem('Open'));
    expect(editableWith('Some text about garden plan.'), findsOneWidget);
  });

  appTest(
    'desktop: Remove from the ellipsis takes the card off the board and keeps the note',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final recipes = f.pages['Recipes']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden, recipes],
          'Doing': [],
          'Done': [],
        },
      );
      await openNote(tester, board, 'Launch board');

      await hoverCard(tester, 'Garden plan');
      await tapAndSettle(tester, cardMenuTrigger(garden));
      await tapAndSettle(tester, menuItem(removeFromBoardLabel));
      await settleStored(tester, board, 'To do', [recipes]);

      expect(storedCards(await storedKanban(tester, board), 'To do'), [
        recipes,
      ]);
      expect(cardIn('To do', 'Garden plan'), findsNothing);
      final top = await core(tester, (v) => v.listChildren(null));
      expect(top.any((row) => row.id == garden), isTrue);
      final note = await core(tester, (v) => v.loadPage(garden));
      expect(note.title, 'Garden plan');
    },
  );

  appTest('phone: the ellipsis is always shown on a card, without any hover', (
    tester,
  ) async {
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

    expect(cardMenuTrigger(garden), findsOneWidget);
    expect(menuOpacity(tester, garden), 1);
  });

  appTest('phone: Open from the ellipsis opens the card note', (tester) async {
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

    await tapAndSettle(tester, cardMenuTrigger(garden));
    await tapAndSettle(tester, menuItem('Open'));
    expect(editableWith('Some text about garden plan.'), findsOneWidget);
  });

  appTest(
    'phone: Remove from the ellipsis takes the card off the board and keeps the note',
    (tester) async {
      final f = await pumpApp(tester, mobile: true);
      final garden = f.pages['Garden plan']!;
      final recipes = f.pages['Recipes']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden, recipes],
          'Doing': [],
          'Done': [],
        },
      );
      await openNote(tester, board, 'Launch board');

      await tapAndSettle(tester, cardMenuTrigger(garden));
      await tapAndSettle(tester, menuItem(removeFromBoardLabel));
      await settleStored(tester, board, 'To do', [recipes]);

      expect(storedCards(await storedKanban(tester, board), 'To do'), [
        recipes,
      ]);
      final note = await core(tester, (v) => v.loadPage(garden));
      expect(note.title, 'Garden plan');
    },
  );

  appTest(
    'phone: Move to column… lists the other columns and appends the card to the chosen one',
    (tester) async {
      final f = await pumpApp(tester, mobile: true);
      final garden = f.pages['Garden plan']!;
      final recipes = f.pages['Recipes']!;
      final reading = f.pages['Reading notes']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden, recipes],
          'Doing': [],
          'Done': [reading],
        },
      );
      await openNote(tester, board, 'Launch board');

      final columns = await storedKanban(tester, board);
      final todo = columnIdOf(columns, 'To do');
      final doing = columnIdOf(columns, 'Doing');
      final done = columnIdOf(columns, 'Done');

      await tapAndSettle(tester, cardMenuTrigger(garden));
      await tapAndSettle(tester, find.byKey(moveCardKey));

      // The card's own column is not offered; the other two are.
      expect(moveTarget(doing), findsOneWidget);
      expect(moveTarget(done), findsOneWidget);
      expect(moveTarget(todo), findsNothing);

      await tapAndSettle(tester, moveTarget(done));
      await settleStored(tester, board, 'Done', [reading, garden]);

      final data = await storedKanban(tester, board);
      expect(storedCards(data, 'To do'), [recipes]);
      expect(storedCards(data, 'Done'), [
        reading,
        garden,
      ], reason: 'appended at the end');
      final note = await core(tester, (v) => v.loadPage(garden));
      expect(note.title, 'Garden plan');
    },
  );
}
