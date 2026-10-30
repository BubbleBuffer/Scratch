// The panel a board card opens: beside the board on desktop, a bottom sheet
// on a phone. It shows the card's status and runner, assigns to an agent
// through the Run on picker, and leads to the note. Enter on a focused card
// opens it and Esc closes it, returning focus to the card.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/kanban/api.dart';
import 'package:notes_app/ui/sheet.dart' show SheetSurface;

import 'kanban_fixture.dart';

const _panel = ValueKey('kanban-card-panel');
const _assign = ValueKey('kanban-panel-assign');
const _openNote = ValueKey('kanban-panel-open-note');

Future<String> _openBoard(WidgetTester tester, {required bool mobile}) async {
  final f = await pumpApp(
    tester,
    mobile: mobile,
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

/// The board's keyboard stop for the first card.
FocusNode _cardFocus(WidgetTester tester) => tester
    .widget<FocusableActionDetector>(
      find
          .descendant(
            of: allCardSurfaces().first,
            matching: find.byType(FocusableActionDetector),
          )
          .first,
    )
    .focusNode!;

void main() {
  setUpAll(initCore);

  appTest(
    'desktop: clicking a card opens a panel beside the board with its status, runner and Assign to Agent',
    (tester) async {
      await _openBoard(tester, mobile: false);
      expect(find.byKey(_panel), findsNothing);

      await tapAndSettle(tester, cardIn('To do', 'Garden plan'));
      expect(find.byKey(_panel), findsOneWidget);
      expect(
        find.descendant(of: find.byKey(_panel), matching: find.text('Garden plan')),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('kanban-panel-status')), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('kanban-panel-status')),
          matching: find.text('To do'),
        ),
        findsOneWidget,
      );
      expect(find.text('Not assigned'), findsOneWidget);
      expect(find.byKey(_assign), findsOneWidget);
      expect(cardIn('To do', 'Garden plan'), findsOneWidget, reason: 'the board stays');
      expect(editableWith('Some text about garden plan.'), findsNothing);

      await tapAndSettle(tester, find.byKey(const ValueKey('kanban-panel-close')));
      expect(find.byKey(_panel), findsNothing);
    },
  );

  appTest(
    'desktop: Assign to Agent in the panel opens the Run on picker and the chosen runner shows in the panel',
    (tester) async {
      final garden = await _openBoard(tester, mobile: false);
      await tapAndSettle(tester, cardIn('To do', 'Garden plan'));
      await tapAndSettle(tester, find.byKey(_assign));

      expect(find.text('Run on'), findsOneWidget);
      await tapAndSettle(tester, find.byKey(const ValueKey('runner-row-office')));
      await tapAndSettle(tester, find.byKey(const ValueKey('runner-assign')));

      expect(find.text('Run on'), findsNothing);
      expect(
        find.descendant(
          of: find.byKey(_panel),
          matching: find.byKey(ValueKey('runner-chip-$garden')),
        ),
        findsOneWidget,
      );
      expect(find.byKey(_assign), findsNothing, reason: 'already assigned');
      expect(find.text('Office-PC · queued'), findsNWidgets(2), reason: 'panel and card');
    },
  );

  appTest('desktop: Open note in the panel opens the note and closes the panel', (
    tester,
  ) async {
    await _openBoard(tester, mobile: false);
    await tapAndSettle(tester, cardIn('To do', 'Garden plan'));
    await tapAndSettle(tester, find.byKey(_openNote));
    expect(editableWith('Some text about garden plan.'), findsOneWidget);
  });

  appTest(
    'desktop keyboard: Enter on a focused card opens the panel, Esc closes it and focus returns to the card',
    (tester) async {
      await _openBoard(tester, mobile: false);

      _cardFocus(tester).requestFocus();
      await settle(tester);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await settle(tester);
      expect(find.byKey(_panel), findsOneWidget);
      expect(editableWith('Some text about garden plan.'), findsNothing);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);
      expect(find.byKey(_panel), findsNothing);
      expect(_cardFocus(tester).hasPrimaryFocus, isTrue);
    },
  );

  appTest('phone: tapping a card opens its panel as a bottom sheet', (
    tester,
  ) async {
    await _openBoard(tester, mobile: true);
    await tapAndSettle(tester, cardIn('To do', 'Garden plan'));

    expect(find.byType(SheetSurface), findsOneWidget);
    expect(find.byKey(_assign), findsOneWidget);
    expect(find.byKey(_panel), findsNothing, reason: 'no side panel on a phone');

    await tapAndSettle(tester, find.byKey(_openNote));
    expect(editableWith('Some text about garden plan.'), findsOneWidget);
  });
}
