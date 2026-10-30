// The card's "Assign to Agent" flow against the fake runner backend: the
// ellipsis menu, the Run on picker (a popover at 700 px and wider, a bottom
// sheet below), assigning, and the runner chip for each state.

import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter/widgets.dart' show Size, ValueKey;
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/kanban/api.dart';
import 'package:notes_app/ui/menu.dart' show MenuSurface;
import 'package:notes_app/ui/sheet.dart' show SheetSurface;

import 'kanban_fixture.dart';

const _assignLabel = 'Assign to Agent';

/// Opens the card menu of [garden] and chooses Assign to Agent.
Future<void> _openPicker(WidgetTester tester, String garden) async {
  await tapAndSettle(tester, cardMenuTrigger(garden));
  await tapAndSettle(tester, menuItem(_assignLabel));
}

/// A board with one card, "Garden plan", opened on screen.
Future<String> _openBoard(WidgetTester tester, String garden) async {
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
  return board;
}

void main() {
  setUpAll(initCore);

  appTest(
    'desktop: the ellipsis offers Assign to Agent, a popover lists the runners and the chosen one is assigned',
    (tester) async {
      final backend = FakeRunnerBackend.demo();
      final f = await pumpApp(
        tester,
        mobile: false,
        runners: RunnerAssignments(backend),
      );
      final garden = f.pages['Garden plan']!;
      await _openBoard(tester, garden);
      await hoverCard(tester, 'Garden plan');
      await _openPicker(tester, garden);

      expect(find.text('Run on'), findsOneWidget);
      expect(find.byType(SheetSurface), findsNothing, reason: 'a popover at 1440 px');
      for (final name in ['Hub', 'Studio-PC', 'Office-PC', 'Laptop']) {
        expect(find.text(name), findsOneWidget);
      }
      expect(find.text('Always on · vault tools only'), findsOneWidget);
      expect(find.text('This device · online'), findsOneWidget);
      expect(find.text('Online'), findsOneWidget);
      expect(find.text('Offline · last seen 2 h ago'), findsOneWidget);
      expect(find.text('default'), findsOneWidget);
      expect(find.text('Change default in Settings'), findsOneWidget);

      // Studio-PC is the default, so it starts selected.
      final assign = find.byKey(const ValueKey('runner-assign'));
      await tapAndSettle(tester, find.byKey(const ValueKey('runner-row-office')));
      await tapAndSettle(tester, assign);

      expect(backend.calls, contains('assign:$garden:office'));
      expect(find.text('Run on'), findsNothing, reason: 'the picker closes');
      expect(find.text('Office-PC · queued'), findsOneWidget);
    },
  );

  appTest(
    'phone: under 700 px the picker is a bottom sheet, and an offline runner leaves the task waiting with its reason',
    (tester) async {
      final backend = FakeRunnerBackend.demo();
      final f = await pumpApp(
        tester,
        mobile: true,
        runners: RunnerAssignments(backend),
      );
      final garden = f.pages['Garden plan']!;
      await _openBoard(tester, garden);
      await _openPicker(tester, garden);

      expect(find.text('Run on'), findsOneWidget);
      expect(find.byType(SheetSurface), findsOneWidget, reason: 'a sheet at 375 px');

      await tapAndSettle(tester, find.byKey(const ValueKey('runner-row-laptop')));
      await tapAndSettle(tester, find.byKey(const ValueKey('runner-assign')));

      expect(backend.calls, contains('assign:$garden:laptop'));
      expect(find.text('Laptop · waiting'), findsOneWidget);
      expect(
        find.text('Laptop is offline (last seen 2 h ago)'),
        findsOneWidget,
        reason: 'the reason sits under the chip',
      );
    },
  );

  appTest('each assignment state shows its chip text and reason on the card', (
    tester,
  ) async {
    final runners = RunnerAssignments(FakeRunnerBackend.demo());
    final f = await pumpApp(tester, mobile: false, runners: runners);
    final garden = f.pages['Garden plan']!;
    await _openBoard(tester, garden);

    void record(AssignState state, {String? reason}) => runners.record(
      RunnerAssignment(
        taskId: garden,
        runnerId: 'studio',
        runnerName: 'Studio-PC',
        state: state,
        reason: reason,
      ),
    );

    record(AssignState.queued);
    await settle(tester);
    expect(find.text('Studio-PC · queued'), findsOneWidget);

    record(AssignState.running);
    await settle(tester);
    expect(find.text('Studio-PC · running'), findsOneWidget);

    record(AssignState.cancelling);
    await settle(tester);
    expect(find.text('Studio-PC · cancellation requested'), findsOneWidget);

    record(AssignState.done);
    await settle(tester);
    expect(find.text('Studio-PC · done'), findsOneWidget);

    record(AssignState.failed, reason: 'The agent stopped with an error.');
    await settle(tester);
    expect(find.text('Studio-PC · failed'), findsOneWidget);
    expect(find.text('The agent stopped with an error.'), findsOneWidget);
  });

  appTest('without a runner scope the card menu uses the agent runners', (
    tester,
  ) async {
    // No scope given and no SEED_DEMO: the board falls back to AiStore's runners.
    final f = await pumpApp(tester, mobile: false);
    final garden = f.pages['Garden plan']!;
    await _openBoard(tester, garden);
    await hoverCard(tester, 'Garden plan');
    await tapAndSettle(tester, cardMenuTrigger(garden));

    expect(menuItem('Open'), findsOneWidget);
    expect(menuItem(_assignLabel), findsOneWidget);
  });

  appTest('the picker shows the reason when the runners cannot be listed', (
    tester,
  ) async {
    final backend = FakeRunnerBackend.demo()..error = 'The hub is not reachable.';
    final f = await pumpApp(
      tester,
      mobile: false,
      runners: RunnerAssignments(backend),
    );
    final garden = f.pages['Garden plan']!;
    await _openBoard(tester, garden);
    await hoverCard(tester, 'Garden plan');
    await _openPicker(tester, garden);

    expect(find.text('The hub is not reachable.'), findsOneWidget);
    expect(find.byKey(const ValueKey('runner-assign')), findsOneWidget);
    expect(find.text('Office-PC'), findsNothing);
  });

  appTest('a PC with projects shows a Project row, and the task is assigned to the chosen one', (
    tester,
  ) async {
    final backend = FakeRunnerBackend.demo();
    final f = await pumpApp(
      tester,
      mobile: false,
      runners: RunnerAssignments(backend),
    );
    final garden = f.pages['Garden plan']!;
    await _openBoard(tester, garden);
    await hoverCard(tester, 'Garden plan');
    await _openPicker(tester, garden);

    // Studio-PC is the default, so its projects show under it, with None first.
    expect(find.text('Project'), findsOneWidget);
    expect(find.byKey(const ValueKey('project-row-none')), findsOneWidget);
    expect(find.byKey(const ValueKey('project-row-notes-app')), findsOneWidget);
    expect(find.byKey(const ValueKey('project-row-docs-site')), findsOneWidget);

    await tapAndSettle(tester, find.byKey(const ValueKey('project-row-docs-site')));
    await tapAndSettle(tester, find.byKey(const ValueKey('runner-assign')));

    expect(backend.calls, contains('assign:$garden:studio:docs-site'));
    expect(find.text('Run on'), findsNothing, reason: 'the picker closes');
    expect(find.text('Studio-PC · docs-site · queued'), findsOneWidget);
  });

  appTest('the hub and a PC without projects show no Project row', (
    tester,
  ) async {
    final backend = FakeRunnerBackend.demo();
    final f = await pumpApp(
      tester,
      mobile: false,
      runners: RunnerAssignments(backend),
    );
    final garden = f.pages['Garden plan']!;
    await _openBoard(tester, garden);
    await hoverCard(tester, 'Garden plan');
    await _openPicker(tester, garden);

    await tapAndSettle(tester, find.byKey(const ValueKey('runner-row-hub')));
    expect(find.text('Project'), findsNothing);
    expect(find.byKey(const ValueKey('project-row-none')), findsNothing);

    await tapAndSettle(tester, find.byKey(const ValueKey('runner-row-office')));
    expect(find.text('Project'), findsNothing);
    expect(find.byKey(const ValueKey('project-row-none')), findsNothing);
    expect(find.byKey(const ValueKey('project-row-notes-app')), findsNothing);
  });

  appTest('switching runners resets the project to None', (tester) async {
    final backend = FakeRunnerBackend.demo();
    final f = await pumpApp(
      tester,
      mobile: false,
      runners: RunnerAssignments(backend),
    );
    final garden = f.pages['Garden plan']!;
    await _openBoard(tester, garden);
    await hoverCard(tester, 'Garden plan');
    await _openPicker(tester, garden);

    await tapAndSettle(tester, find.byKey(const ValueKey('project-row-notes-app')));
    await tapAndSettle(tester, find.byKey(const ValueKey('runner-row-office')));
    await tapAndSettle(tester, find.byKey(const ValueKey('runner-row-studio')));
    await tapAndSettle(tester, find.byKey(const ValueKey('runner-assign')));

    expect(
      backend.calls,
      contains('assign:$garden:studio'),
      reason: 'back on Studio-PC the project is None again',
    );
    expect(find.text('Studio-PC · queued'), findsOneWidget);
  });

  appTest(
    'desktop at 1280 px: the picker opens below the card menu button, inside the window, and leaves the column headers uncovered',
    (tester) async {
      final backend = FakeRunnerBackend.demo();
      final f = await pumpApp(
        tester,
        mobile: false,
        runners: RunnerAssignments(backend),
      );
      tester.view.physicalSize = const Size(1280, 800);
      final garden = f.pages['Garden plan']!;
      await _openBoard(tester, garden);
      await hoverCard(tester, 'Garden plan');
      final button = tester.getRect(cardMenuTrigger(garden));
      final header = tester.getRect(find.text('To do').first);
      await _openPicker(tester, garden);

      final picker = tester.getRect(
        find.ancestor(of: find.text('Run on'), matching: find.byType(MenuSurface)),
      );
      expect(picker.top, greaterThanOrEqualTo(button.bottom), reason: 'below the button');
      expect(picker.top, greaterThan(header.bottom), reason: 'the column header stays visible');
      expect(picker.right, lessThanOrEqualTo(1280));
      expect(picker.bottom, lessThanOrEqualTo(800));
    },
  );

  appTest(
    'desktop keyboard: arrows move through the runners and projects, Space selects, Enter on Assign submits',
    (tester) async {
      final backend = FakeRunnerBackend.demo();
      final f = await pumpApp(
        tester,
        mobile: false,
        runners: RunnerAssignments(backend),
      );
      final garden = f.pages['Garden plan']!;
      await _openBoard(tester, garden);
      await hoverCard(tester, 'Garden plan');
      await _openPicker(tester, garden);

      expect(
        focusIsIn(find.byKey(const ValueKey('runner-row-studio'))),
        isTrue,
        reason: 'starts on the chosen runner',
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await settle(tester);
      expect(focusIsIn(find.byKey(const ValueKey('project-row-none'))), isTrue);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await settle(tester);
      expect(focusIsIn(find.byKey(const ValueKey('project-row-notes-app'))), isTrue);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      await settle(tester);

      final assign = find.byKey(const ValueKey('runner-assign'));
      for (var i = 0; i < 12 && !focusIsIn(assign); i++) {
        await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
        await settle(tester);
      }
      expect(focusIsIn(assign), isTrue);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await settle(tester);
      expect(backend.calls, contains('assign:$garden:studio:notes-app'));
      expect(find.text('Run on'), findsNothing);
    },
  );

  appTest('desktop keyboard: Esc closes the picker without assigning', (
    tester,
  ) async {
    final backend = FakeRunnerBackend.demo();
    final f = await pumpApp(
      tester,
      mobile: false,
      runners: RunnerAssignments(backend),
    );
    final garden = f.pages['Garden plan']!;
    await _openBoard(tester, garden);
    await hoverCard(tester, 'Garden plan');
    await _openPicker(tester, garden);
    expect(find.text('Run on'), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);
    expect(find.text('Run on'), findsNothing);
    expect(backend.calls.where((c) => c.startsWith('assign')), isEmpty);
  });
}
