import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/editor/mention_menu.dart' show isoDate, longDate;
import 'package:notes_app/state/agenda_store.dart';
import 'package:notes_app/ui/ui.dart' show StacksCheckbox;

import 'harness.dart';

Finder key(String name) => find.byKey(ValueKey(name));
Finder itemRow(AgendaItem item) =>
    key('today-item-${AgendaStore.itemKey(item)}');
Finder itemCheck(AgendaItem item) =>
    find.descendant(of: itemRow(item), matching: find.byType(StacksCheckbox));
Finder rowsInPage(String page) => find.byWidgetPredicate(
  (widget) =>
      widget.key is ValueKey<String> &&
      (widget.key! as ValueKey<String>).value.startsWith('today-item-$page/'),
);
Future<void> showItem(WidgetTester tester, AgendaItem item) async {
  while (itemRow(item).evaluate().isEmpty &&
      key('today-more').evaluate().isNotEmpty) {
    await reveal(tester, key('today-more'));
    await tapAndSettle(tester, key('today-more'));
  }
  await reveal(tester, itemRow(item));
}

Finder get capture => find.descendant(
  of: key('capture-field'),
  matching: find.byType(EditableText),
);
Future<void> reveal(WidgetTester tester, Finder target) async {
  await tester.ensureVisible(target);
  await tester.pump();
}

List<BOp> datedTasks(
  int count,
  DateTime day, {
  bool checked = false,
  String prefix = 'Due',
  String? after,
}) {
  final ops = <BOp>[];
  for (var n = 0; n < count; n++) {
    final block = newId();
    final title = '$prefix task ${n.toString().padLeft(3, '0')}';
    final text = '$title @${longDate(day)}';
    ops.add(
      BOp(
        op: OpKind.insert,
        id: block,
        after: after,
        kind: Kinds.todo,
        text: text,
        checked: checked,
        marks: [
          BMark(
            start: title.length + 1,
            end: text.length,
            kind: BMarkKind.date,
            url: isoDate(day),
          ),
        ],
      ),
    );
    after = block;
  }
  return ops;
}

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    final form = mobile ? 'mobile' : 'desktop';

    appTest('Today ticks a database task and dated to-do on $form', (
      tester,
    ) async {
      final f = await pumpApp(tester, mobile: mobile, demo: true);
      final a = f.app.agenda;
      for (final item in [
        a.items.firstWhere((i) => i.title == 'Draft Q4 goals'),
        a.items.firstWhere((i) => i.title == 'Plant garlic' && i.block != null),
      ]) {
        await showItem(tester, item);
        await tapAndSettle(tester, itemCheck(item));
        await settleUntil(
          tester,
          () => a.items.any(
            (i) =>
                AgendaStore.itemKey(i) == AgendaStore.itemKey(item) && i.done,
          ),
        );
      }
      expect(a.tasksDueToday, 3);
      final garden = await pageOf(tester, f.pages['Garden plan']!);
      expect(
        garden.blocks
            .singleWhere((b) => b.text.startsWith('Plant garlic'))
            .checked,
        isTrue,
      );
      await reveal(tester, key('today-done'));
      await tapAndSettle(tester, key('today-done'));
      final item = a.items.firstWhere((i) => i.title == 'Draft Q4 goals');
      await showItem(tester, item);
      expect(tester.widget<StacksCheckbox>(itemCheck(item)).checked, isTrue);
    });

    appTest('select a day and return to today on $form', (tester) async {
      final f = await pumpApp(tester, mobile: mobile, demo: true);
      await tapAndSettle(tester, key('today-date-2026-10-08'));
      expect(f.app.agenda.selectedDay, DateTime(2026, 10, 8));
      expect(
        tester.widget<Text>(key('today-selected-heading')).data,
        'Thu 8 Oct',
      );
      expect(find.text('Turn the compost'), findsOneWidget);
      expect(key('today-overdue'), findsNothing);
      await tapAndSettle(tester, key('today-back'));
      expect(f.app.agenda.isToday, isTrue);
      expect(key('today-overdue'), findsOneWidget);
    });

    appTest(
      'capture appends dated task then paragraph to one daily note on $form',
      (tester) async {
        final f = await pumpApp(tester, mobile: mobile, empty: true);
        // The feedback and its Open link last four seconds of fake time.
        // settle advances fake time while it waits for native writes and
        // agenda reloads, which under CI load can outlast them, so wait for
        // the capture in real time with zero-length pumps instead.
        final feedback = find.text("Added to today's note · ");
        Future<void> captured() async {
          final saved = Stopwatch()..start();
          while ((tester.widget<EditableText>(capture).controller.text
                      .isNotEmpty ||
                  feedback.evaluate().isEmpty) &&
              saved.elapsed < const Duration(seconds: 10)) {
            await tester.runAsync(
              () => Future<void>.delayed(const Duration(milliseconds: 1)),
            );
            await tester.pump();
          }
          expect(tester.widget<EditableText>(capture).controller.text, isEmpty);
          expect(feedback, findsOneWidget);
        }

        await tester.enterText(capture, '🌱 Plant a tree');
        await tester.testTextInput.receiveAction(TextInputAction.done);
        await captured();
        await settleUntil(
          tester,
          () => f.app.agenda.items.any((i) => i.title == '🌱 Plant a tree'),
        );
        final id = await core(
          tester,
          (v) => v.run(
            (r) => r.dailyNote(
              dayMs: dayStamp(fixedNow),
              title: '',
              create: false,
            ),
          ),
        );
        expect(id, isNotNull);
        var page = await pageOf(tester, id!);
        expect(page.title, 'Tuesday 6 October 2026');
        final task = page.blocks.singleWhere(
          (b) => b.kind == Kinds.todo && b.text.contains('Plant a tree'),
        );
        final logIndex = page.blocks.indexWhere(
          (b) => b.kind == Kinds.toggle && b.text == 'Log',
        );
        expect(page.blocks.indexOf(task), lessThan(logIndex));
        expect(task.kind, Kinds.todo);
        expect(task.text, '🌱 Plant a tree @October 6, 2026');
        expect(task.marks.single.kind, BMarkKind.date);
        expect(task.marks.single.url, '2026-10-06');
        expect(task.marks.single.start, '🌱 Plant a tree '.length);
        await tapAndSettle(tester, key('capture-note'));
        await tester.enterText(capture, 'An idea for later');
        await tester.testTextInput.receiveAction(TextInputAction.done);
        await captured();
        // Open while the feedback still shows; the checks below settle.
        await tapAndSettle(tester, key('capture-open'));
        expect(
          tester.widget<EditableText>(key('page-title')).controller.text,
          'Tuesday 6 October 2026',
        );
        page = await pageOf(tester, id);
        final idea = page.blocks.singleWhere(
          (b) => b.text == 'An idea for later',
        );
        expect(idea.kind, Kinds.paragraph);
        expect(idea.marks, isEmpty);
        final ideaIndex = page.blocks.indexOf(idea);
        expect(page.blocks[ideaIndex - 1].id, task.id);
        expect(
          ideaIndex,
          lessThan(
            page.blocks.indexWhere(
              (b) => b.kind == Kinds.toggle && b.text == 'Log',
            ),
          ),
        );
        final journal = await core(
          tester,
          (v) => v.run((r) => r.findByRole(role: 'journal')),
        );
        expect((await core(tester, (v) => v.listChildren(journal))).length, 1);
      },
    );

    appTest('Today’s note opens and reuses the daily note on $form', (
      tester,
    ) async {
      final f = await pumpApp(tester, mobile: mobile, empty: true);
      await reveal(tester, key('todays-note'));
      await tapAndSettle(tester, key('todays-note'));
      expect(
        tester.widget<EditableText>(key('page-title')).controller.text,
        'Tuesday 6 October 2026',
      );
      final id = f.app.ui.lastPage;
      expect(id, isNotNull);
      await tapAndSettle(tester, key('back'));
      await reveal(tester, key('todays-note'));
      await tapAndSettle(tester, key('todays-note'));
      expect(f.app.ui.lastPage, id);
    });
  }

  appTest('dated to-do opens its page at the block with a brief highlight', (
    tester,
  ) async {
    final f = await pumpApp(tester, mobile: false, demo: true);
    final item = f.app.agenda.items.firstWhere(
      (i) => i.title == 'Plant garlic' && i.block != null,
    );
    final row = itemRow(item);
    await showItem(tester, item);
    await tapAndSettle(tester, row);
    final editor = tester.state<PageEditorState>(find.byType(PageScreen));
    expect(editor.widget.initialBlock, item.block);
    expect(editor.highlightedBlock, item.block);
    final rect = tester.getRect(editableStarting('Plant garlic'));
    expect(rect.top, greaterThanOrEqualTo(45));
    expect(rect.bottom, lessThan(900));
    await tester.pump(const Duration(seconds: 3));
    expect(editor.highlightedBlock, isNull);
  });

  appTest(
    'completing an overdue task moves it into done and unticking returns it to overdue',
    (tester) async {
      final f = await pumpApp(tester, mobile: false, demo: true);
      final store = f.app.agenda;
      final item = store.overdueItems.firstWhere(
        (i) => i.title == 'Return library books',
      );
      final itemKey = AgendaStore.itemKey(item);
      expect(itemRow(item), findsOneWidget);
      await showItem(tester, item);
      await tapAndSettle(tester, itemCheck(item));
      await settleUntil(
        tester,
        () => store.selectedDoneItems.any(
          (i) => AgendaStore.itemKey(i) == itemKey,
        ),
      );
      expect(store.overdueCount, 2);
      final done = store.selectedDoneItems.firstWhere(
        (i) => AgendaStore.itemKey(i) == itemKey,
      );
      await showItem(tester, done);
      expect(tester.widget<StacksCheckbox>(itemCheck(done)).checked, isTrue);
      await tapAndSettle(tester, itemCheck(done));
      await settleUntil(
        tester,
        () => store.overdueItems.any(
          (i) => AgendaStore.itemKey(i) == itemKey && !i.done,
        ),
      );
      expect(store.overdueCount, 3);
      expect(
        store.selectedDoneItems.any((i) => AgendaStore.itemKey(i) == itemKey),
        isFalse,
      );
      await showItem(tester, item);
      expect(tester.widget<StacksCheckbox>(itemCheck(item)).checked, isFalse);
    },
  );

  appTest(
    'capture mode and All pages remember their state and Escape clears capture',
    (tester) async {
      var f = await pumpApp(tester, mobile: false, empty: true, persist: true);
      expect(f.app.ui.allPagesExpanded, isFalse);
      expect(key('search-field'), findsNothing);
      await tapAndSettle(tester, key('capture-note'));
      await tester.enterText(capture, 'discard this');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      expect(tester.widget<EditableText>(capture).controller.text, isEmpty);
      await reveal(tester, key('all-pages-toggle'));
      await tapAndSettle(tester, key('all-pages-toggle'));
      expect(key('search-field'), findsOneWidget);
      f = await restartApp(tester, restoreLastPage: false);
      expect(f.app.ui.captureTask, isFalse);
      expect(f.app.ui.allPagesExpanded, isTrue);
    },
  );

  appTest(
    'mobile capture stays above the dock and the dock clears the keyboard',
    (tester) async {
      await pumpApp(tester, mobile: true, empty: true);
      await reveal(tester, key('capture-row'));
      expect(
        tester.getRect(key('capture-row')).bottom,
        lessThan(tester.getRect(key('mobile-dock')).top),
      );
      await showKeyboard(tester);
      await reveal(tester, key('capture-row'));
      expect(
        tester.getRect(key('capture-row')).bottom,
        lessThan(tester.getRect(key('mobile-dock')).top),
      );
      expect(
        tester.getRect(key('mobile-dock')).bottom,
        lessThanOrEqualTo(812 - keyboardHeight),
      );
    },
  );

  appTest('narrow desktop stacks capture actions without overflow', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    tester.view.physicalSize = const Size(800, 900);
    await settle(tester);
    expect(tester.takeException(), isNull);
    expect(key('sidebar-new-page-row'), findsOneWidget);
    expect(key('capture-add'), findsOneWidget);
    await tester.enterText(capture, 'A narrow-window task');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await settle(tester);
    expect(key('capture-open'), findsOneWidget);
  });

  appTest(
    'selected-day cards show five at a time and reset after changing days',
    (tester) async {
      final f = await pumpApp(tester, mobile: false, empty: true);
      final id = await core(tester, (v) async {
        final page = await v.createPage(title: 'A full day');
        await v.apply(page, datedTasks(12, fixedNow));
        return page;
      });
      await settleUntil(tester, () => f.app.agenda.selectedItems.length == 12);
      final rows = find.byWidgetPredicate(
        (widget) =>
            widget.key is ValueKey<String> &&
            (widget.key! as ValueKey<String>).value.startsWith(
              'today-item-$id/',
            ),
      );
      expect(rows, findsNWidgets(5));
      for (final count in [10, 12]) {
        await reveal(tester, key('today-more'));
        await tapAndSettle(tester, key('today-more'));
        expect(rows, findsNWidgets(count));
      }
      expect(key('today-more'), findsNothing);
      await reveal(tester, key('today-date-2026-10-07'));
      await tapAndSettle(tester, key('today-date-2026-10-07'));
      expect(rows, findsNothing);
      await tapAndSettle(tester, key('today-back'));
      expect(rows, findsNWidgets(5));
      expect(key('today-more'), findsOneWidget);
    },
  );

  appTest(
    'overdue cards are disclosed and load beyond the bounded store projection',
    (tester) async {
      final f = await pumpApp(tester, mobile: false, empty: true);
      final id = await core(tester, (v) async {
        final page = await v.createPage(title: 'Task groups');
        await v.apply(
          page,
          datedTasks(55, DateTime(2026, 10, 5), prefix: 'Overdue'),
        );
        return page;
      });
      await settleUntil(tester, () => f.app.agenda.overdueCount == 55);
      final rows = find.byWidgetPredicate(
        (widget) =>
            widget.key is ValueKey<String> &&
            (widget.key! as ValueKey<String>).value.startsWith(
              'today-item-$id/',
            ),
      );
      expect(f.app.agenda.overdueItems.length, 50);
      expect(rows, findsNWidgets(5));
      for (var count = 10; count <= 55; count += 5) {
        await reveal(tester, key('today-overdue-more'));
        await tapAndSettle(tester, key('today-overdue-more'));
        expect(rows, findsNWidgets(count));
      }
      expect(f.app.agenda.overdueItems.length, 55);
      expect(key('today-overdue-more'), findsNothing);
      await reveal(tester, key('today-date-2026-10-07'));
      await tapAndSettle(tester, key('today-date-2026-10-07'));
      expect(key('today-overdue'), findsNothing);
      await tapAndSettle(tester, key('today-back'));
      // Overdue re-expands on the day change, with its budget of five again.
      expect(rows, findsNWidgets(5));
      expect(key('today-overdue-more'), findsOneWidget);
      expect(f.app.agenda.overdueLimit, 50);
    },
  );

  for (final overdueCount in [3, 7]) {
    appTest(
      'overdue rows have their own budget beside Today rows with $overdueCount overdue',
      (tester) async {
        final f = await pumpApp(tester, mobile: false, empty: true);
        final (late, today) = await core(tester, (v) async {
          final late = await v.createPage(title: 'Late groups');
          await v.apply(
            late,
            datedTasks(overdueCount, DateTime(2026, 10, 5), prefix: 'Late'),
          );
          final today = await v.createPage(title: 'Today groups');
          await v.apply(today, datedTasks(5, fixedNow, prefix: 'Now'));
          return (late, today);
        });
        final shownLate = overdueCount > 5 ? 5 : overdueCount;
        await settleUntil(
          tester,
          () =>
              rowsInPage(late).evaluate().length == shownLate &&
              rowsInPage(today).evaluate().length == 5,
        );
        expect(f.app.agenda.overdueCount, overdueCount);
        expect(key('today-overdue'), findsOneWidget);
        expect(rowsInPage(today), findsNWidgets(5));
        expect(rowsInPage(late), findsNWidgets(shownLate));
        expect(key('today-more'), findsNothing);
        if (overdueCount > 5) {
          expect(key('today-overdue-more'), findsOneWidget);
          expect(find.text('Show ${overdueCount - 5} more'), findsOneWidget);
          await reveal(tester, key('today-overdue-more'));
          await tapAndSettle(tester, key('today-overdue-more'));
          expect(rowsInPage(late), findsNWidgets(overdueCount));
          expect(rowsInPage(today), findsNWidgets(5));
          expect(key('today-overdue-more'), findsNothing);
        } else {
          expect(key('today-overdue-more'), findsNothing);
        }
      },
    );
  }

  appTest(
    'midnight advances today, reloads and keeps an independently selected day',
    (tester) async {
      final f = await pumpApp(tester, mobile: false, empty: true);
      Clock.fixedAt = DateTime(2026, 10, 6, 23, 59, 59);
      final a = AgendaStore(f.vault, f.app.feed);
      final b = AgendaStore(f.vault, f.app.feed)
        ..selectDay(DateTime(2026, 10, 9));
      try {
        await finishAsync(tester, a.reload());
        Clock.fixedAt = DateTime(2026, 10, 7);
        await tester.pump(const Duration(minutes: 1));
        await settle(tester);
        expect(a.selectedDay, DateTime(2026, 10, 7));
        expect(b.selectedDay, DateTime(2026, 10, 9));
        expect(a.summary, 'Wednesday 7 October · Nothing planned');
      } finally {
        a.dispose();
        b.dispose();
      }
    },
  );
}
