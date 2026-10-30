// The card ellipsis. On desktop it appears while the pointer is over a card,
// and right-click opens the same menu. On a phone it is always shown, and its
// menu also offers Move to column…, which lists the other columns and appends
// the card to the one chosen. Open and Remove work on both; Remove keeps the
// note itself.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/kanban/api.dart';

import 'kanban_fixture.dart';

Future<String> _boardWithGarden(WidgetTester tester) async {
  final f = await pumpApp(
    tester,
    mobile: false,
    runners: RunnerAssignments(FakeRunnerBackend.demo()),
  );
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
  return garden;
}

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

  appTest(
    'desktop keyboard: the context-menu key opens the card menu, arrows move between items, Esc closes it and focus returns to the card',
    (tester) async {
      final garden = await _boardWithGarden(tester);

      cardFocus(tester).requestFocus();
      await settle(tester);
      expect(menuOpacity(tester, garden), 1, reason: 'shown while focused');

      await tester.sendKeyEvent(LogicalKeyboardKey.contextMenu);
      await settle(tester);
      expect(menuItem('Open'), findsOneWidget);
      expect(focusIsIn(menuItem('Open')), isTrue, reason: 'first item');

      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await settle(tester);
      expect(focusIsIn(menuItem('Assign to Agent')), isTrue);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await settle(tester);
      expect(focusIsIn(menuItem(removeFromBoardLabel)), isTrue);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowUp);
      await settle(tester);
      expect(focusIsIn(menuItem('Assign to Agent')), isTrue);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);
      expect(menuItem('Open'), findsNothing, reason: 'Esc closes the menu');
      expect(cardFocus(tester).hasPrimaryFocus, isTrue);
    },
  );

  appTest('desktop keyboard: Shift+F10 opens the menu and Enter runs the focused item', (
    tester,
  ) async {
    await _boardWithGarden(tester);

    cardFocus(tester).requestFocus();
    await settle(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.f10);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await settle(tester);
    expect(menuItem('Open'), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await settle(tester);
    expect(editableWith('Some text about garden plan.'), findsOneWidget);
  });

  appTest(
    'desktop keyboard: after the board scrolls to the focused card, the menu opens at the card',
    (tester) async {
      final f = await pumpApp(
        tester,
        mobile: false,
        runners: RunnerAssignments(FakeRunnerBackend.demo()),
      );
      tester.view.physicalSize = const Size(1000, 800);
      final garden = f.pages['Garden plan']!;
      final reading = f.pages['Reading notes']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden],
          'Doing': [],
          'Done': [reading],
        },
      );
      await openNote(tester, board, 'Launch board');

      cardFocus(tester, 1).requestFocus();
      await settle(tester);
      await tester.sendKeyEvent(LogicalKeyboardKey.contextMenu);
      await settle(tester);

      final card = tester.getRect(allCardSurfaces().at(1));
      final menu = tester.getRect(menuItem('Open'));
      expect(menu.left, lessThanOrEqualTo(card.right), reason: 'starts within the card');
      expect(menu.left, greaterThanOrEqualTo(card.left));
      expect(menu.top, lessThanOrEqualTo(card.bottom + 8));
    },
  );

  appTest(
    'desktop keyboard: the board is one Tab stop and the arrow keys move between cards',
    (tester) async {
      final f = await pumpApp(tester, mobile: false);
      final garden = f.pages['Garden plan']!;
      final recipes = f.pages['Recipes']!;
      final reading = f.pages['Reading notes']!;
      final board = await seedKanban(
        tester,
        title: 'Launch board',
        columns: {
          'To do': [garden, recipes],
          'Doing': [reading],
          'Done': [],
        },
      );
      await openNote(tester, board, 'Launch board');
      await settle(tester);

      expect(cardFocus(tester, 0).skipTraversal, isFalse, reason: 'the first card is the stop');
      expect(cardFocus(tester, 1).skipTraversal, isTrue);
      expect(cardFocus(tester, 2).skipTraversal, isTrue);

      cardFocus(tester, 0).requestFocus();
      await settle(tester);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await settle(tester);
      expect(cardFocus(tester, 1).hasPrimaryFocus, isTrue, reason: 'down: next card in the column');
      expect(cardFocus(tester, 1).skipTraversal, isFalse, reason: 'the stop follows focus');
      expect(cardFocus(tester, 0).skipTraversal, isTrue);

      await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await settle(tester);
      expect(cardFocus(tester, 2).hasPrimaryFocus, isTrue, reason: 'right: next column');

      await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
      await settle(tester);
      expect(cardFocus(tester, 0).hasPrimaryFocus, isTrue, reason: 'left: back, row kept within the shorter column');
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowUp);
      await settle(tester);
      expect(cardFocus(tester, 0).hasPrimaryFocus, isTrue, reason: 'up: previous card');
    },
  );
}
