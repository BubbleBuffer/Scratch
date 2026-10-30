// First-run Home copy and layout, the mobile tree panel, the `/` menu order
// and the to-do spacing (UX review, 2026-10-07).

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/blocks/todo.dart';
import 'package:notes_app/features/getting_started.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/shell/addons.dart';
import 'package:notes_app/shell/today.dart';
import 'package:notes_app/state/agenda_store.dart';
import 'package:notes_app/ui/ui.dart' show plural, StacksCheckbox;

import 'harness.dart';

Finder key(String name) => find.byKey(ValueKey(name));

/// The id of the top-level page titled [title].
String topLevel(TestVault f, String title) =>
    f.app.tree.childrenOf(null)!.firstWhere((p) => p.title == title).id;

void main() {
  setUpAll(initCore);

  group('copy', () {
    test('plurals', () {
      expect(plural(1, 'task'), '1 task');
      expect(plural(2, 'task'), '2 tasks');
      expect(plural(0, 'item'), '0 items');
      expect(plural(1, 'item'), '1 item');
      expect(plural(1200, 'event'), '1,200 events');
    });

    test('the header summary is one sentence', () {
      expect(
        todaySummary(events: 1, tasks: 1, overdue: 0),
        '1 event and 1 task today.',
      );
      expect(
        todaySummary(events: 0, tasks: 0, overdue: 0),
        'Nothing planned today.',
      );
      expect(
        todaySummary(events: 0, tasks: 3, overdue: 2),
        '3 tasks today, 2 overdue.',
      );
      expect(todaySummary(events: 2, tasks: 0, overdue: 0), '2 events today.');
      expect(
        todaySummary(events: 0, tasks: 0, overdue: 1),
        'Nothing due today, 1 overdue.',
      );
    });

    test('another day reads date then counts', () {
      final wed = DateTime(2026, 10, 7);
      expect(
        dayPeek(wed, events: 1, tasks: 2),
        'Wed 7 October · 1 event, 2 tasks',
      );
      expect(dayPeek(wed, events: 0, tasks: 1), 'Wed 7 October · 1 task');
      expect(
        dayPeek(wed, events: 0, tasks: 0),
        'Wed 7 October · Nothing planned',
      );
    });
  });

  group('/ menu', () {
    test('leads with the everyday blocks, View and Callout early', () {
      registerBuiltins();
      final labels = [for (final t in blockRegistry.filter('')) t.menuLabel];
      expect(labels.take(17).toList(), [
        'Text',
        'Page',
        'To-do list',
        'Heading 1',
        'Heading 2',
        'Heading 3',
        'Bulleted list',
        'Numbered list',
        'View',
        'Callout',
        'Quote',
        'Divider',
        'Toggle list',
        'Toggle heading 1',
        'Toggle heading 2',
        'Toggle heading 3',
        'Link to page',
      ]);
      expect(blockRegistry.filter('').first.group, 'Basic blocks');
    });

    test('filtered entries sit under one Results header', () {
      registerBuiltins();
      final hits = blockRegistry.filter('view');
      expect(hits.map((t) => t.kind), contains('view'));
      expect(hits.every((t) => t.group == BlockRegistry.resultsGroup), isTrue);
      expect(BlockRegistry.resultsGroup, 'Results');
      // The registered types keep their own sections.
      expect(blockRegistry.of('view').group, 'Basic blocks');
    });

    appTest('the menu shows Results while filtering', (tester) async {
      await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final b = editableStarting('Plain paragraph');
      await focusAt(
        tester,
        b,
        editState(tester, b).widget.controller.text.length,
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await settle(tester);
      await typeText(tester, '/');
      expect(
        find.descendant(
          of: key('slash-menu'),
          matching: find.text('Basic blocks'),
        ),
        findsOneWidget,
      );
      await typeText(tester, 'view');
      expect(
        find.descendant(of: key('slash-menu'), matching: find.text('Results')),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: key('slash-menu'),
          matching: find.text('Basic blocks'),
        ),
        findsNothing,
      );
      expect(key('slash-view'), findsOneWidget);
    });
  });

  for (final mobile in [false, true]) {
    final form = mobile ? 'mobile' : 'desktop';

    appTest(
      'first-run Today shows the selected agenda and capture once on $form',
      (tester) async {
        await pumpApp(tester, mobile: mobile, fresh: true);
        expect(key('today-date-strip'), findsOneWidget);
        expect(
          tester.widget<Text>(key('today-selected-heading')).data,
          'Tue 6 Oct',
        );
        expect(key('capture-field'), findsOneWidget);
        expect(key('today-done'), findsNothing);
        expect(find.text('Recent'), findsNothing);
        expect(key('home-summary'), findsNothing);
        expect(key('week-peek'), findsNothing);
        expect(find.text('Plan today'), findsWidgets);
        if (mobile) {
          final bar = tester.getRect(key('capture-row'));
          final helper = tester.getRect(key('capture-helper'));
          expect(helper.top - bar.bottom, 8);
        }
      },
    );

    appTest('another day shows its own heading and task on $form', (
      tester,
    ) async {
      final f = await pumpApp(tester, mobile: mobile, fresh: true);
      await tapAndSettle(tester, key('today-date-2026-10-07'));
      expect(f.app.agenda.isToday, isFalse);
      expect(
        tester.widget<Text>(key('today-selected-heading')).data,
        'Wed 7 Oct',
      );
      expect(key('today-overdue'), findsNothing);
      expect(f.app.agenda.selectedItems.where((i) => !i.done).length, 1);
    });

    appTest('the done pile appears once a task is done on $form', (
      tester,
    ) async {
      final f = await pumpApp(tester, mobile: mobile, fresh: true);
      expect(key('today-done'), findsNothing);
      final item = f.app.agenda.selectedItems.firstWhere(
        (i) => i.title == 'Plan today',
      );
      final check = find.descendant(
        of: key('today-item-${AgendaStore.itemKey(item)}'),
        matching: find.byType(StacksCheckbox),
      );
      await tester.ensureVisible(check);
      await tapAndSettle(tester, check);
      await settleUntil(
        tester,
        () => f.app.agenda.selectedDoneItems.isNotEmpty,
      );
      await settle(tester, min: const Duration(seconds: 1));
      expect(key('today-done'), findsOneWidget);
      expect(tester.widget<StacksCheckbox>(check).checked, isTrue);
    });
  }

  appTest('mobile tree panel: open, expand, Trash and Settings', (
    tester,
  ) async {
    final f = await pumpApp(tester, mobile: true, fresh: true);
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
    expect(key('nav-home'), findsNothing);
    expect(key('nav-trash'), findsNothing);
    await openMobileTree(tester);
    expect(key('pages-list'), findsOneWidget);
    final getting = topLevel(f, gettingStartedTitle);
    final tasks = topLevel(f, 'Tasks');
    for (final id in [getting, tasks, topLevel(f, 'Calendar')]) {
      final row = tester.getRect(key('pages-$id'));
      expect(row.height, greaterThanOrEqualTo(48));
    }
    // Getting started has no pages inside: no chevron.
    expect(key('pages-toggle-$getting'), findsNothing);

    await tapAndSettle(tester, key('pages-toggle-$tasks'));
    expect(f.app.tree.isExpanded(tasks), isTrue);
    expect(
      find.descendant(of: key('pages-list'), matching: find.text('Plan today')),
      findsOneWidget,
    );
    await tapAndSettle(tester, key('pages-toggle-$tasks'));
    expect(
      find.descendant(of: key('pages-list'), matching: find.text('Plan today')),
      findsNothing,
    );

    await tapAndSettle(tester, key('pages-$getting'));
    expect(
      tester.widget<EditableText>(key('page-title')).controller.text,
      gettingStartedTitle,
    );
    await tapAndSettle(tester, key('back'));
    // A page opened from the tree returns to Home, with the tree closed.
    expect(key('page-title'), findsNothing);
    expect(key('pages-list'), findsNothing);
    expect(key('capture-field'), findsOneWidget);

    await openMobileTree(tester);
    await tester.ensureVisible(key('pages-trash'));
    await tapAndSettle(tester, key('pages-trash'));
    expect(key('trash-panel'), findsOneWidget);
    await tapAndSettle(tester, key('screen-back'));
    expect(key('pages-list'), findsOneWidget);
    await closeMobileTreeWithBack(tester);
    expect(key('pages-list'), findsNothing);

    await openMobileTree(tester);
    await tester.ensureVisible(key('pages-settings'));
    await tapAndSettle(tester, key('pages-settings'));
    expect(key('settings-nav-reminders'), findsOneWidget);
    await tapAndSettle(tester, key('back'));

    // Settings returns to the tree; back closes it before leaving Home.
    expect(key('pages-list'), findsOneWidget);
    await closeMobileTreeWithBack(tester);
    expect(key('pages-list'), findsNothing);
    expect(key('capture-field'), findsOneWidget);
    expect(key('mobile-dock'), findsOneWidget);
  });

  appTest('a to-do box sits 8 px before its text, centred on the first line', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, fresh: true);
    await tapAndSettle(
      tester,
      find.descendant(
        of: key('sidebar'),
        matching: find.text(gettingStartedTitle),
      ),
    );
    final text = editableStarting('Click anywhere');
    final state = editState(tester, text);
    final check = find.byWidgetPredicate(
      (w) =>
          w.key is ValueKey<String> &&
          (w.key! as ValueKey<String>).value.startsWith('check-'),
    );
    final box = tester.getRect(check.first);
    expect(box.width, todoBoxSize);
    final editable = state.renderEditable;
    final first = editable
        .getBoxesForSelection(
          const TextSelection(baseOffset: 0, extentOffset: 1),
        )
        .first;
    final origin = editable.localToGlobal(Offset.zero);
    final line = first.toRect().shift(origin);
    expect(line.left - box.right, closeTo(todoTextGap, 0.5));
    expect(box.center.dy, closeTo(line.center.dy, 1));
  });
}
