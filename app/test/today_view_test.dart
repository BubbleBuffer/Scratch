import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/today_view.dart';
import 'package:notes_app/state/agenda_store.dart';
import 'package:notes_app/ui/ui.dart' show StacksCheckbox;

import 'calendar_source_fixture.dart';
import 'harness.dart';

Finder key(String value) => find.byKey(ValueKey(value));
Finder agendaTitle(String title) =>
    find.descendant(of: find.byType(TodayView), matching: find.text(title));
Finder card(AgendaItem item) => key('today-item-${AgendaStore.itemKey(item)}');
Future<void> showAll(WidgetTester tester) async {
  while (key('today-more').evaluate().isNotEmpty) {
    await tester.ensureVisible(key('today-more'));
    await tapAndSettle(tester, key('today-more'));
  }
}

Future<void> jump(WidgetTester tester, String date) async {
  await tapAndSettle(tester, key('today-jump'));
  final field = find.descendant(
    of: key('calendar-jump-date'),
    matching: find.byType(EditableText),
  );
  await tester.enterText(field, date);
  await tapAndSettle(tester, find.text('Go to date'));
}

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    final form = mobile ? 'mobile' : 'desktop';
    appTest(
      'date strip browsing preserves selection and Today recenters on $form',
      (tester) async {
        final f = await pumpApp(tester, mobile: mobile, empty: true);
        final origin = f.app.agenda.selectedDay;
        await tapAndSettle(tester, key('today-strip-next'));
        expect(f.app.agenda.selectedDay, origin);
        await tapAndSettle(tester, key('today-strip-previous'));
        expect(f.app.agenda.selectedDay, origin);
        await tapAndSettle(tester, key('today-date-2026-10-07'));
        expect(f.app.agenda.selectedDay, DateTime(2026, 10, 7));
        expect(
          tester
              .widget<Semantics>(key('today-date-2026-10-07'))
              .properties
              .selected,
          isTrue,
        );
        expect(key('today-empty'), findsOneWidget);
        expect(
          await core(
            tester,
            (v) => v.run(
              (r) => r.dailyNote(
                dayMs: dayStamp(DateTime(2026, 10, 7)),
                title: '',
                create: false,
              ),
            ),
          ),
          isNull,
        );
        await tapAndSettle(tester, key('today-back'));
        expect(f.app.agenda.selectedDay, origin);
        expect(
          tester
              .widget<Semantics>(key('today-date-2026-10-06'))
              .properties
              .selected,
          isTrue,
        );
      },
    );

    appTest(
      'jump validates dates and selects across a year boundary on $form',
      (tester) async {
        final f = await pumpApp(tester, mobile: mobile, empty: true);
        await tapAndSettle(tester, key('today-jump'));
        final field = find.descendant(
          of: key('calendar-jump-date'),
          matching: find.byType(EditableText),
        );
        await tester.enterText(field, '2026-02-30');
        await tapAndSettle(tester, find.text('Go to date'));
        expect(find.text('Use a valid date, YYYY-MM-DD.'), findsOneWidget);
        expect(f.app.agenda.selectedDay, DateTime(2026, 10, 6));
        await tester.enterText(field, '2027-01-01');
        await tapAndSettle(tester, find.text('Go to date'));
        expect(f.app.agenda.selectedDay, DateTime(2027, 1, 1));
        expect(key('today-date-2027-01-01'), findsOneWidget);
        expect(
          tester.widget<Text>(key('today-selected-heading')).data,
          'Fri 1 Jan',
        );
        await tapAndSettle(tester, key('today-back'));
        expect(f.app.agenda.isToday, isTrue);
      },
    );

    appTest(
      'journal note marks follow dayNotesInRange without agenda items on $form',
      (tester) async {
        final f = await pumpApp(tester, mobile: mobile, empty: true);
        expect(key('today-note-mark-2026-10-07'), findsNothing);
        final day = DateTime(2026, 10, 7);
        await core(
          tester,
          (v) => v.run(
            (r) => r.dailyNote(
              dayMs: dayStamp(day),
              title: 'Wednesday 7 October 2026',
              create: true,
            ),
            writes: true,
          ),
        );
        final notes = await core(
          tester,
          (v) => v.dayNotesInRange(
            fromMs: dayStamp(DateTime(2026, 10, 6)),
            toMs: dayStamp(DateTime(2026, 10, 9)),
          ),
        );
        expect(notes, [dayStamp(day)]);
        await settleUntil(
          tester,
          () => key('today-note-mark-2026-10-07').evaluate().isNotEmpty,
        );
        expect(f.app.agenda.onDay(day), isEmpty);
        expect(
          tester
              .widget<Semantics>(key('today-date-2026-10-07'))
              .properties
              .label,
          contains('has a note'),
        );
        expect(key('today-note-mark-2026-10-08'), findsNothing);
      },
    );
  }

  appTest('date strip keyboard selects days, returns Today and opens jump', (
    tester,
  ) async {
    final f = await pumpApp(tester, mobile: false, empty: true);
    await tapAndSettle(tester, key('today-date-2026-10-06'));
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    await settle(tester);
    expect(f.app.agenda.selectedDay, DateTime(2026, 10, 7));
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
    await settle(tester);
    expect(f.app.agenda.isToday, isTrue);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyT);
    await settle(tester);
    expect(f.app.agenda.isToday, isTrue);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyG);
    await settle(tester);
    expect(find.byType(CalendarJumpPicker), findsOneWidget);
    final field = find.descendant(
      of: key('calendar-jump-date'),
      matching: find.byType(EditableText),
    );
    await tester.enterText(field, '2026-10-12');
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    expect(f.app.agenda.isToday, isTrue);
    await tapAndSettle(tester, find.text('Go to date'));
    expect(f.app.agenda.selectedDay, DateTime(2026, 10, 12));
  });

  appTest(
    'selected agenda respects Today visibility, excludes mentions and uses event start day',
    (tester) async {
      final f = await pumpApp(tester, mobile: false, empty: true);
      final pages = await core(tester, seedCalendarToday);
      await finishAsync(tester, f.app.agenda.reload());
      await settle(tester);
      await showAll(tester);
      final agenda = f.app.agenda;
      AgendaItem item(String title) =>
          agenda.items.firstWhere((i) => i.title == title);
      for (final title in [
        'Water the plants',
        'Dentist appointment',
        'Buy compost',
        'Review project quote',
      ]) {
        expect(card(item(title)), findsOneWidget);
      }
      await core(
        tester,
        (v) => v.run(
          (r) => r.setAgendaSettings(
            collection: pages['Projects']!,
            settings: const AgendaSettings(role: AgendaRole.off),
          ),
          writes: true,
        ),
      );
      await finishAsync(tester, agenda.reload());
      await settle(tester);
      expect(agendaTitle('Review project quote'), findsNothing);
      expect(
        find.descendant(
          of: find.byType(TodayView),
          matching: find.textContaining('Reference only'),
        ),
        findsNothing,
      );
      expect(agendaTitle('Hidden agent plan'), findsNothing);
      final fullCalendar = await core(
        tester,
        (v) => v.calendarItems(
          timedFromMs: DateTime(2026, 10, 6).millisecondsSinceEpoch,
          timedToMs: DateTime(2026, 10, 7).millisecondsSinceEpoch,
          dayFromMs: dayStamp(fixedNow),
          dayToMs: dayStamp(DateTime(2026, 10, 7)),
        ),
      );
      expect(
        fullCalendar.any(
          (i) =>
              i.kind == AgendaKind.mention &&
              i.title.contains('Reference only'),
        ),
        isTrue,
      );
      final settings = await core(tester, (v) => v.listCalendars());
      final work = settings.calendars.firstWhere((c) => c.name == 'Work');
      await core(
        tester,
        (v) => v.updateCalendar(
          CalendarCategory(
            id: work.id,
            name: work.name,
            color: work.color,
            showInCalendar: true,
            showInToday: false,
          ),
        ),
      );
      await finishAsync(tester, agenda.reload());
      await settle(tester);
      expect(agendaTitle('Dentist appointment'), findsNothing);
      expect(agendaTitle('Late night handover'), findsNothing);
      expect(agendaTitle('Water the plants'), findsOneWidget);
      // Calendar visibility is independent: these rows still appear in Calendar.
      final calendarAfter = await core(
        tester,
        (v) => v.calendarItems(
          timedFromMs: DateTime(2026, 10, 6).millisecondsSinceEpoch,
          timedToMs: DateTime(2026, 10, 7).millisecondsSinceEpoch,
          dayFromMs: dayStamp(fixedNow),
          dayToMs: dayStamp(DateTime(2026, 10, 7)),
        ),
      );
      expect(
        calendarAfter.any((i) => i.title == 'Dentist appointment'),
        isTrue,
      );
      await core(
        tester,
        (v) => v.updateCalendar(
          CalendarCategory(
            id: work.id,
            name: work.name,
            color: work.color,
            showInCalendar: false,
            showInToday: true,
          ),
        ),
      );
      await finishAsync(tester, agenda.reload());
      await settle(tester);
      await jump(tester, '2026-10-07');
      expect(agendaTitle('Late night handover'), findsNothing);
      expect(agendaTitle('Plan the weekend'), findsNothing);
      await jump(tester, '2026-10-10');
      expect(agendaTitle('Plan the weekend'), findsOneWidget);
      expect(agendaTitle('Water the plants'), findsNothing);
    },
  );

  appTest('strip item markers exclude plain mentions and hidden calendars', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    await core(tester, (v) async {
      final hidden = await v.createCalendar(
        name: 'Hidden',
        color: 'moss',
        showInCalendar: true,
        showInToday: false,
      );
      final task = await v.createPage(
        title: 'Hidden dated task',
        pageType: 'task',
      );
      await saveTask(v, task, TaskData(date: '2026-10-07', calendar: hidden));
      final note = await v.createPage(title: 'Strip sources');
      const mention = 'Mention only @October 8, 2026';
      const todo = 'Dated to-do @October 9, 2026';
      final mentionBlock = newId();
      await v.apply(note, [
        BOp(
          op: OpKind.insert,
          id: mentionBlock,
          kind: Kinds.paragraph,
          text: mention,
          checked: false,
          marks: [
            BMark(
              start: mention.indexOf('@'),
              end: mention.length,
              kind: BMarkKind.date,
              url: '2026-10-08',
            ),
          ],
        ),
        BOp(
          op: OpKind.insert,
          id: newId(),
          after: mentionBlock,
          kind: Kinds.todo,
          text: todo,
          checked: false,
          marks: [
            BMark(
              start: todo.indexOf('@'),
              end: todo.length,
              kind: BMarkKind.date,
              url: '2026-10-09',
            ),
          ],
        ),
      ]);
      await v.flush();
    });
    await settle(tester);
    for (final date in ['2026-10-07', '2026-10-08']) {
      await jump(tester, date);
      expect(
        tester.widget<Semantics>(key('today-date-$date')).properties.label,
        isNot(contains(' items')),
      );
      // Saving even a hidden dated task creates its Journal link. Note marks
      // are independent of item visibility; plain mentions create no note.
      expect(
        key('today-note-mark-$date'),
        date == '2026-10-07' ? findsOneWidget : findsNothing,
      );
    }
    await jump(tester, '2026-10-09');
    await settleUntil(
      tester,
      () => tester
          .widget<Semantics>(key('today-date-2026-10-09'))
          .properties
          .label!
          .contains('1 items'),
    );
    expect(key('today-note-mark-2026-10-09'), findsNothing);
    expect(find.text('Dated to-do'), findsOneWidget);
  });

  appTest('Agent task keeps its category and badge while completion settles', (
    tester,
  ) async {
    final f = await pumpApp(tester, mobile: false, empty: true);
    final category = await core(
      tester,
      (v) => v.createCalendar(
        name: 'Studio',
        color: 'moss',
        showInCalendar: true,
        showInToday: true,
      ),
    );
    final page = await core(tester, (v) async {
      final page = await v.createPage(
        title: 'Agent studio plan',
        pageType: 'task',
      );
      await saveTask(
        v,
        page,
        TaskData(date: '2026-10-06', calendar: category, assignee: 'agent'),
      );
      return page;
    });
    await finishAsync(tester, f.app.agenda.reload());
    await settle(tester);
    final row = key('today-item-$page/');
    expect(
      find.descendant(of: row, matching: find.text('Studio')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: row, matching: key('short-item-agent')),
      findsOneWidget,
    );
    final checkbox = find.descendant(
      of: row,
      matching: find.byType(StacksCheckbox),
    );
    await tapAndSettle(tester, checkbox);
    await settleUntil(
      tester,
      () => f.app.agenda.selectedDoneItems.any((i) => i.page == page),
    );
    expect(tester.widget<StacksCheckbox>(checkbox).checked, isTrue);
    expect(
      find.descendant(of: row, matching: find.text('Studio')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: row, matching: key('short-item-agent')),
      findsOneWidget,
    );
  });
}
