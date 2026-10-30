// The panel a board card opens: beside the board on desktop, a bottom sheet
// on a phone. It shows the card's status and runner, assigns to an agent
// through the Run on picker, and leads to the note. Enter on a focused card
// opens it and Esc closes it, returning focus to the card.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/kanban/api.dart';
import 'package:notes_app/ui/board/board_renderer.dart' show BoardRenderer;
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
        tester.widget<Text>(find.byKey(const ValueKey('kanban-panel-status'))).data,
        'To do',
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

      cardFocus(tester).requestFocus();
      await settle(tester);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await settle(tester);
      expect(find.byKey(_panel), findsOneWidget);
      expect(editableWith('Some text about garden plan.'), findsNothing);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);
      expect(find.byKey(_panel), findsNothing);
      expect(cardFocus(tester).hasPrimaryFocus, isTrue);
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

  appTest(
    'desktop at 1280 px: the panel is docked, the board keeps scrolling beside it and the last column stays reachable',
    (tester) async {
      await _openBoard(tester, mobile: false);
      tester.view.physicalSize = const Size(1280, 800);
      await settle(tester);
      await tapAndSettle(tester, cardIn('To do', 'Garden plan'));

      final panel = tester.getRect(find.byKey(_panel));
      expect(panel.width, 320);
      expect(panel.right, lessThanOrEqualTo(1280));
      final board = find.byType(BoardRenderer);
      expect(tester.getRect(board).right, lessThanOrEqualTo(panel.left));

      final horizontal = tester
          .stateList<ScrollableState>(
            find.descendant(of: board, matching: find.byType(Scrollable)),
          )
          .firstWhere((s) => s.axisDirection == AxisDirection.right);
      horizontal.position.jumpTo(horizontal.position.maxScrollExtent);
      await settle(tester);
      expect(
        tester.getRect(columnNamed('Done')).right,
        lessThanOrEqualTo(panel.left),
        reason: 'Done scrolls into view beside the panel, not under it',
      );
    },
  );

  appTest(
    'desktop: focus starts on the panel, not on Close, so no ring or tooltip shows until Tab',
    (tester) async {
      await _openBoard(tester, mobile: false);
      await tapAndSettle(tester, cardIn('To do', 'Garden plan'));

      expect(FocusManager.instance.primaryFocus?.debugLabel, 'kanban card panel title');
      expect(focusIsIn(find.byKey(const ValueKey('kanban-panel-close'))), isFalse);
      expect(find.text('Close'), findsNothing, reason: 'no tooltip');

      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await settle(tester);
      expect(
        focusIsIn(find.byKey(_assign)),
        isTrue,
        reason: 'Tab leaves the title for the first control after it',
      );

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);
      expect(find.byKey(_panel), findsNothing, reason: 'Esc closes from a control too');
    },
  );
}
