import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import 'package:notes_app/ui/database/db_icons.dart';
import 'package:notes_app/ui/ui.dart';

import '../../addon_harness.dart';
import '../../calendar_source_fixture.dart' show seedCalendarToday;
import 'task_fixture.dart';

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    appTest(
      'short task card ${mobile ? 'phone' : 'desktop'} keeps height and completion independent from opening',
      (tester) async {
        final fixture = await pumpApp(tester, mobile: mobile, empty: true);
        final page = await core(tester, (v) async {
          final page = await v.createPage(
            title: 'A very long title that must stay on one line even on a narrow phone and never hide the metadata',
            pageType: taskType,
          );
          final calendars = await v.listCalendars();
          await saveTask(
            v,
            page,
            TaskData(
              date: DateTime(2026, 10, 8, 10, 30).toUtc().toIso8601String(),
              durationMin: 90,
              assignee: 'agent',
              calendar: calendars.calendars.first.id,
            ),
          );
          return page;
        });
        final item = await indexedTask(tester, page, DateTime(2026, 10, 8));
        final calendars = await core(tester, (v) => v.listCalendars());
        var toggles = 0, opens = 0;
        Widget card(AgendaItem item) => ShortItemCard(
          key: const ValueKey('test-card'),
          item: item,
          calendars: calendars,
          onToggle: () => toggles++,
          onOpen: () => opens++,
        );
        await pumpTaskHost(tester, fixture, card(item), mobile: mobile);
        final before = tester
            .getSize(find.byKey(const ValueKey('test-card')))
            .height;
        expect(before, lessThanOrEqualTo(mobile ? 68 : 48));
        expect(find.text('10:30'), findsOneWidget);
        expect(find.text('Agent'), findsOneWidget);
        expect(find.text('Me'), findsNothing);
        await tapAndSettle(tester, find.byType(StacksCheckbox));
        expect(toggles, 1);
        expect(opens, 0);
        await tapAndSettle(
          tester,
          find.byKey(const ValueKey('short-item-title')),
        );
        expect(opens, 1);
        await core(tester, (v) async {
          await v.run(
            (raw) => raw.setAgendaDone(page: page, done: true),
            writes: true,
          );
          await v.flush();
        });
        final done = await indexedTask(tester, page, DateTime(2026, 10, 8));
        await pumpTaskHost(tester, fixture, card(done), mobile: mobile);
        expect(
          tester.getSize(find.byKey(const ValueKey('test-card'))).height,
          before,
        );
        expect(
          tester
              .widget<Text>(find.byKey(const ValueKey('short-item-title')))
              .style!
              .decoration,
          TextDecoration.lineThrough,
        );
        expect(tester.takeException(), isNull);
      },
    );
  }

  appTest(
    'overdue cards replace time with original due date and expose their source',
    (tester) async {
      final fixture = await pumpApp(tester, mobile: false, empty: true);
      final page = await core(tester, (v) async {
        final page = await v.createPage(
          title: 'Send invoice',
          pageType: taskType,
        );
        final calendars = await v.listCalendars();
        await saveTask(
          v,
          page,
          TaskData(
            date: DateTime(2026, 10, 3, 10, 30).toUtc().toIso8601String(),
            calendar: calendars.calendars.first.id,
          ),
        );
        return page;
      });
      final item = await indexedTask(tester, page, DateTime(2026, 10, 3));
      final calendars = await core(tester, (v) => v.listCalendars());
      await pumpTaskHost(
        tester,
        fixture,
        ShortItemCard(
          item: item,
          calendars: calendars,
          sourceTitle: 'Source details',
          overdue: true,
          showDate: true,
        ),
      );
      expect(find.text('3 Oct'), findsOneWidget);
      expect(find.text('10:30'), findsNothing);
      await tapAndSettle(tester, find.text('Personal'));
      expect(find.text('Personal · Source details'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  appTest(
    'row, to-do and mention cards retain their source kind and checkbox visibility',
    (tester) async {
      final fixture = await pumpApp(tester, mobile: false, empty: true);
      await core(tester, seedCalendarToday);
      final items = await core(
        tester,
        (v) => v.calendarItems(
          timedFromMs: DateTime(2026, 10, 6).millisecondsSinceEpoch,
          timedToMs: DateTime(2026, 10, 7).millisecondsSinceEpoch,
          dayFromMs: DateTime.utc(2026, 10, 6).millisecondsSinceEpoch,
          dayToMs: DateTime.utc(2026, 10, 7).millisecondsSinceEpoch,
        ),
      );
      final calendars = await core(tester, (v) => v.listCalendars());
      final row = items.firstWhere((item) => item.collection != null);
      await pumpTaskHost(
        tester,
        fixture,
        ShortItemCard(item: row, calendars: calendars, sourceTitle: 'Projects'),
      );
      expect(find.text('Projects'), findsOneWidget);
      expect(
        find.byWidgetPredicate((w) => w is DbIcon && w.icon == DbIco.database),
        findsWidgets,
      );
      expect(find.byType(StacksCheckbox), findsOneWidget);
      final todo = items.firstWhere(
        (item) => item.block != null && item.kind == AgendaKind.task,
      );
      await pumpTaskHost(
        tester,
        fixture,
        ShortItemCard(item: todo, sourceTitle: 'Garden notes'),
      );
      expect(find.text('Garden notes'), findsOneWidget);
      expect(find.byType(StacksCheckbox), findsOneWidget);
      final mention = items.firstWhere(
        (item) => item.kind == AgendaKind.mention,
      );
      await pumpTaskHost(
        tester,
        fixture,
        ShortItemCard(item: mention, sourceTitle: 'Garden notes'),
      );
      expect(find.byType(StacksCheckbox), findsNothing);
      expect(find.byType(PageIcon), findsOneWidget);
      final timed = items.firstWhere(
        (item) => item.title == 'Dentist appointment',
      );
      await pumpTaskHost(
        tester,
        fixture,
        ShortItemCard(item: timed, calendars: calendars, checkbox: false),
      );
      expect(find.byType(StacksCheckbox), findsNothing);
      expect(
        find.byWidgetPredicate((w) => w is Icon20 && w.icon == Ico.clock),
        findsWidgets,
      );
      expect(tester.takeException(), isNull);
    },
  );

  appTest(
    'task chip and embed honor optional checkbox and read completion from status',
    (tester) async {
      final fixture = await pumpApp(tester, mobile: false, empty: true);
      final page = await core(tester, (v) async {
        final page = await v.createPage(
          title: 'Plain task',
          pageType: taskType,
        );
        await saveTask(
          v,
          page,
          const TaskData(status: 'done', checkbox: false),
        );
        return page;
      });
      for (final presentation in [
        TaskPresentation.chip,
        TaskPresentation.embed,
      ]) {
        await pumpTaskHost(
          tester,
          fixture,
          TaskNote(
            args: NoteArgs(pageId: page, title: 'Plain task'),
            presentation: presentation,
          ),
        );
        expect(find.byType(StacksCheckbox), findsNothing);
        expect(find.text('Plain task'), findsOneWidget);
        expect(
          tester.widget<Text>(find.text('Plain task')).style!.decoration,
          TextDecoration.lineThrough,
        );
        expect(find.text('1 Jan'), findsNothing);
      }
    },
  );

  appTest('a narrow desktop shared card stays on one line without overflow', (
    tester,
  ) async {
    final fixture = await pumpApp(tester, mobile: false, empty: true);
    final page = await core(tester, (v) async {
      final page = await v.createPage(
        title: 'A long task title in the day side list',
        pageType: taskType,
      );
      final calendars = await v.listCalendars();
      await saveTask(
        v,
        page,
        TaskData(
          date: DateTime(2026, 10, 8, 10, 30).toUtc().toIso8601String(),
          calendar: calendars.calendars.first.id,
          assignee: 'agent',
        ),
      );
      return page;
    });
    final item = await indexedTask(tester, page, DateTime(2026, 10, 8));
    final calendars = await core(tester, (v) => v.listCalendars());
    for (final showDate in [false, true]) {
      await pumpTaskHost(
        tester,
        fixture,
        ShortItemCard(
          key: const ValueKey('narrow-card'),
          item: item,
          calendars: calendars,
          showDate: showDate,
        ),
        width: 270,
      );
      expect(
        tester.getSize(find.byKey(const ValueKey('narrow-card'))).height,
        lessThanOrEqualTo(48),
      );
      expect(tester.takeException(), isNull);
    }
  });
}
