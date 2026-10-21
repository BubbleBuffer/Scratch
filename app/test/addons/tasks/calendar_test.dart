import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/addons/tasks/calendar_model.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/state/agenda_store.dart';
import 'package:notes_app/ui/database/values.dart';
import 'package:notes_app/ui/ui.dart' show Hover, StacksCheckbox;

import '../../addon_harness.dart';

BuildContext calendarContext(WidgetTester tester) =>
    tester.element(find.byType(Hover).first);

Finder monthItemTitle(String title) => find.descendant(
  of: find.byKey(const ValueKey('calendar-month-scroll')),
  matching: find.text(title),
);

Future<String> seedCalendarTask(
  WidgetTester tester,
  Vault vault,
  String title,
  DateTime date, {
  bool timed = false,
  int? duration,
}) async {
  return (await tester.runAsync(() async {
    final page = await vault.createPage(title: title, pageType: 'task');
    await saveTask(
      vault,
      page,
      TaskData(
        date: taskDateString(date, timed: timed),
        durationMin: duration,
        calendar: 'personal',
      ),
    );
    return page;
  }))!;
}

void main() {
  setUpAll(initCore);

  appTest('Calendar opens on Today selection and keeps selections on back', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    tv.app.agenda.selectDay(DateTime(2026, 11, 4));
    unawaited(openCalendar(calendarContext(tester)));
    await settle(tester);
    expect(find.byType(VaultCalendarScreen), findsOneWidget);
    expect(find.text('November 2026'), findsWidgets);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await settle(tester);
    expect(sameDay(tv.app.agenda.selectedDay, DateTime(2026, 11, 5)), isTrue);
    await tester.tap(find.bySemanticsLabel('Back to Pages'));
    await settle(tester);
    expect(find.byType(VaultCalendarScreen), findsNothing);
    expect(sameDay(tv.app.agenda.selectedDay, DateTime(2026, 11, 5)), isTrue);
  });

  appTest('jump validates typed date and period shortcuts pause while typing', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    unawaited(openCalendar(calendarContext(tester)));
    await settle(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyG);
    await settle(tester);
    final field = find.descendant(
      of: find.byKey(const ValueKey('calendar-jump-date')),
      matching: find.byType(EditableText),
    );
    await tester.enterText(field, '2026-02-30');
    await tester.tap(find.text('Go to date'));
    await settle(tester);
    expect(find.text('Use a valid date, YYYY-MM-DD.'), findsOneWidget);
    await tester.enterText(field, '2026-11-17');
    await tester.sendKeyEvent(LogicalKeyboardKey.keyW);
    expect(find.byKey(const ValueKey('calendar-time-scroll')), findsNothing);
    await tester.tap(find.text('Go to date'));
    await settle(tester);
    expect(
      sameDay(current.app.agenda.selectedDay, DateTime(2026, 11, 17)),
      isTrue,
    );
  });

  appTest('continuous months extend on scroll and day creation is prefilled', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    unawaited(openCalendar(calendarContext(tester)));
    await settle(tester);
    final scroll = find.byKey(const ValueKey('calendar-month-scroll'));
    await tester.drag(scroll, const Offset(0, -1800));
    await settle(tester);
    expect(find.text('December 2026'), findsWidgets);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyN);
    await settle(tester);
    expect(find.byType(TaskComposer), findsOneWidget);
    final composer = tester.widget<TaskComposer>(find.byType(TaskComposer));
    expect(
      sameDay(composer.initialDate, current.app.agenda.selectedDay),
      isTrue,
    );
  });

  appTest('task drag changes day and retains time and duration', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final page = await seedCalendarTask(
      tester,
      tv.vault,
      'Drag this task',
      DateTime(2026, 10, 6, 10, 30),
      timed: true,
      duration: 95,
    );
    unawaited(openCalendar(calendarContext(tester)));
    await settle(tester);
    final drag = find
        .ancestor(
          of: monthItemTitle('Drag this task').first,
          matching: find.byType(Draggable<AgendaItem>),
        )
        .first;
    final target = find.byKey(const ValueKey('calendar-day-2026-10-07-10'));
    await tester.dragFrom(
      tester.getCenter(drag),
      tester.getCenter(target) - tester.getCenter(drag),
    );
    await settle(tester);
    final task = (await tester.runAsync(() => loadTask(tv.vault, page)))!;
    expect(task.dateTime, DateTime(2026, 10, 7, 10, 30));
    expect(task.durationMin, 95);
  });

  appTest('deleting a category retains its dated tasks in Calendar', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final id = (await tester.runAsync(
      () => tv.vault.createCalendar(
        name: 'Temporary',
        color: 'moss',
        showInCalendar: true,
        showInToday: true,
      ),
    ))!;
    final page = await seedCalendarTask(
      tester,
      tv.vault,
      'Retained task',
      DateTime(2026, 10, 6),
    );
    await tester.runAsync(
      () => tv.vault.typeDataSet(page, 'tasks', ['calendar'], '"$id"'),
    );
    unawaited(openCalendar(calendarContext(tester)));
    await settle(tester);
    await tester.tap(find.bySemanticsLabel('Temporary calendar menu'));
    await settle(tester);
    await tester.tap(find.text('Delete calendar').first);
    await settle(tester);
    await tester.tap(find.text('Delete calendar').last);
    await settle(tester);
    expect(monthItemTitle('Retained task'), findsOneWidget);
    final task = (await tester.runAsync(() => loadTask(tv.vault, page)))!;
    expect(task.date, '2026-10-06');
    expect(task.calendar, id);
  });

  appTest('desktop week and day grids render with the current-time gutter', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    await seedCalendarTask(
      tester,
      tv.vault,
      'Night task',
      DateTime(2026, 10, 5, 23, 30),
      timed: true,
      duration: 120,
    );
    unawaited(openCalendar(calendarContext(tester)));
    await settle(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyW);
    await settle(tester);
    expect(find.byKey(const ValueKey('calendar-time-scroll')), findsOneWidget);
    for (final weekday in ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']) {
      expect(find.text(weekday), findsOneWidget);
    }
    expect(find.byKey(const ValueKey('calendar-now-label')), findsOneWidget);
    expect(find.byKey(const ValueKey('calendar-now-line')), findsOneWidget);
    expect(find.text('now 11:20'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyD);
    await settle(tester);
    expect(find.byKey(const ValueKey('calendar-now-label')), findsOneWidget);
    expect(find.text('now 11:20'), findsOneWidget);
    expect(find.textContaining('Continued'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('open day popover updates its shared card after completion', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    await seedCalendarTask(
      tester,
      tv.vault,
      'Popover plan',
      DateTime(2026, 10, 6),
    );
    unawaited(openCalendar(calendarContext(tester)));
    await settle(tester);
    await tester.tap(find.byKey(const ValueKey('calendar-day-2026-10-06-10')));
    await settle(tester);
    final box = find.descendant(
      of: find.byType(ShortItemCard),
      matching: find.byType(StacksCheckbox),
    );
    expect(tester.widget<StacksCheckbox>(box).checked, isFalse);
    await tester.tap(find.bySemanticsLabel('Complete Popover plan'));
    await settle(tester);
    expect(tester.widget<StacksCheckbox>(box).checked, isTrue);
  });

  appTest('phone week and day swipes move their periods and fit the viewport', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: true, empty: true);
    unawaited(openCalendar(calendarContext(tester)));
    await settle(tester);
    await tester.tap(find.text('Week'));
    await settle(tester);
    final before = tv.app.agenda.selectedDay;
    await tester.fling(
      find.text('Nothing planned'),
      const Offset(-240, 0),
      500,
    );
    await settle(tester);
    expect(
      sameDay(tv.app.agenda.selectedDay, calendarAddDays(before, 7)),
      isTrue,
    );
    expect(tester.takeException(), isNull);
    await tester.tap(find.text('Day'));
    await settle(tester);
    final dayBefore = tv.app.agenda.selectedDay;
    await tester.fling(
      find.text('Nothing planned'),
      const Offset(-240, 0),
      500,
    );
    await settle(tester);
    expect(
      sameDay(tv.app.agenda.selectedDay, calendarAddDays(dayBefore, 1)),
      isTrue,
    );
    expect(tester.takeException(), isNull);
    await tester.fling(find.text('Nothing planned'), const Offset(240, 0), 500);
    await settle(tester);
    expect(sameDay(tv.app.agenda.selectedDay, dayBefore), isTrue);
    expect(tester.takeException(), isNull);
  });

  appTest(
    'calendar and Today switches independently control their projections',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      final page = await seedCalendarTask(
        tester,
        tv.vault,
        'Personal plan',
        DateTime(2026, 10, 6),
      );
      unawaited(openCalendar(calendarContext(tester)));
      await settle(tester);
      Future<List<AgendaItem>> today() => tv.vault.run(
        (v) => v.agenda(
          timedFromMs: DateTime(2026, 10, 6).millisecondsSinceEpoch,
          timedToMs: DateTime(2026, 10, 7).millisecondsSinceEpoch,
          dayFromMs: dayStamp(DateTime(2026, 10, 6)),
          dayToMs: dayStamp(DateTime(2026, 10, 7)),
        ),
      );
      await tester.tap(find.bySemanticsLabel('Show Personal in calendar'));
      await settle(tester);
      expect(monthItemTitle('Personal plan'), findsNothing);
      expect(
        (await tester.runAsync(today))!.any((i) => i.page == page),
        isTrue,
      );
      await tester.tap(find.bySemanticsLabel('Show Personal in calendar'));
      await settle(tester);
      await tester.tap(find.bySemanticsLabel('Show Personal in Today'));
      await settle(tester);
      expect(monthItemTitle('Personal plan'), findsOneWidget);
      expect(
        (await tester.runAsync(today))!.any((i) => i.page == page),
        isFalse,
      );
    },
  );

  appTest(
    'calendar creation rename and colour edits persist through their UI',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      unawaited(openCalendar(calendarContext(tester)));
      await settle(tester);
      await tester.tap(find.text('New calendar'));
      await settle(tester);
      await tester.enterText(find.byType(EditableText).last, 'Focus');
      await tester.pump();
      await tester.tap(find.text('Save calendar'));
      await settle(tester);
      await tester.tap(find.bySemanticsLabel('Focus calendar menu'));
      await settle(tester);
      await tester.tap(find.text('Rename or change colour'));
      await settle(tester);
      await tester.enterText(find.byType(EditableText).last, 'Deep work');
      await tester.pump();
      await tester.tap(find.bySemanticsLabel('moss'));
      await tester.tap(find.text('Save calendar'));
      await settle(tester);
      final settings = (await tester.runAsync(tv.vault.listCalendars))!;
      final calendar = settings.calendars.singleWhere(
        (c) => c.name == 'Deep work',
      );
      expect(calendar.color, 'moss');
      expect(calendar.showInCalendar, isTrue);
      expect(calendar.showInToday, isTrue);
      expect(find.text('Focus'), findsNothing);
      expect(find.text('Deep work'), findsOneWidget);
    },
  );

  appTest('database visibility hides only its overlay and retains row dates', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final seeded = (await tester.runAsync(() async {
      final database = await tv.vault.createPage(title: 'Projects');
      await tv.vault.run((v) => v.makeCollection(page: database), writes: true);
      final schema = await tv.vault.run(
        (v) => v.setSchema(
          collection: database,
          schema: const [
            PropDef(
              key: 'due',
              name: 'Due',
              kind: PropKind.date,
              options: [],
              config: '',
            ),
          ],
        ),
        writes: true,
      );
      final row = await tv.vault.createPage(
        parent: database,
        title: 'Database plan',
      );
      await tv.vault.run(
        (v) => v.setProperty(
          page: row,
          key: schema.single.key,
          value: Values.date(dayStamp(DateTime(2026, 10, 6))),
        ),
        writes: true,
      );
      await tv.vault.flush();
      return (database, row, schema.single.key);
    }))!;
    unawaited(openCalendar(calendarContext(tester)));
    await settle(tester);
    expect(find.text('Database plan'), findsOneWidget);
    await tester.tap(find.bySemanticsLabel('Show Projects in calendar'));
    await settle(tester);
    expect(find.text('Database plan'), findsNothing);
    final row = (await tester.runAsync(() => tv.vault.loadPage(seeded.$2)))!;
    expect(
      row.properties
          .singleWhere((p) => p.key == seeded.$3)
          .value
          .startMs
          .toInt(),
      dayStamp(DateTime(2026, 10, 6)),
    );
    await tester.tap(find.bySemanticsLabel('Show Projects in calendar'));
    await settle(tester);
    expect(find.text('Database plan'), findsOneWidget);
  });
}
