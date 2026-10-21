import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/calendar_model.dart';
import 'package:notes_app/core/models.dart';

AgendaItem timed(String title, DateTime start, DateTime end) => AgendaItem(
  page: title,
  title: title,
  kind: AgendaKind.task,
  startMs: start.millisecondsSinceEpoch,
  endMs: end.millisecondsSinceEpoch,
  hasTime: true,
  done: false,
  updatedMs: 0,
);

void main() {
  test(
    'month row count uses calendar days across daylight-saving transitions',
    () {
      expect(calendarMonthWeeks(DateTime(2025, 3)), 6);
      expect(calendarMonthWeeks(DateTime(2026, 10)), 5);
    },
  );
  test('cross-midnight intervals clip once per day and exclude the ending midnight', () {
    final item = timed(
      'Night task',
      DateTime(2026, 10, 12, 23, 30),
      DateTime(2026, 10, 13, 1, 15),
    );
    final first = calendarSegments([item], DateTime(2026, 10, 12)).single;
    final second = calendarSegments([item], DateTime(2026, 10, 13)).single;
    expect(
      (first.startMinute, first.endMinute, first.continued),
      (1410, 1440, false),
    );
    expect(
      (second.startMinute, second.endMinute, second.continued),
      (0, 75, true),
    );
    final midnight = timed(
      'Ends midnight',
      DateTime(2026, 10, 12, 23),
      DateTime(2026, 10, 13),
    );
    expect(calendarSegments([midnight], DateTime(2026, 10, 13)), isEmpty);
  });

  test(
    'overlapping blocks share columns while a later group gets full width',
    () {
      final day = DateTime(2026, 10, 12);
      final items = [
        timed('A', DateTime(2026, 10, 12, 9), DateTime(2026, 10, 12, 11)),
        timed('B', DateTime(2026, 10, 12, 10), DateTime(2026, 10, 12, 12)),
        timed('C', DateTime(2026, 10, 12, 14), DateTime(2026, 10, 12, 15)),
      ];
      final segments = calendarSegments(items, day);
      expect(segments.map((s) => (s.lane, s.lanes)).toList(), [
        (0, 2),
        (1, 2),
        (0, 1),
      ]);
    },
  );

  test('calendar visibility is independent from Today and deleted categories stay visible', () {
    const hidden = CalendarCategory(
      id: 'hidden',
      name: 'Hidden',
      color: 'moss',
      showInCalendar: false,
      showInToday: true,
    );
    const settings = CalendarSettings(
      calendars: [hidden],
      databases: [CalendarDatabase(id: 'db', showInCalendar: false)],
    );
    AgendaItem item({String? calendar, String? collection}) => AgendaItem(
      page: 'task',
      title: 'Task',
      kind: AgendaKind.task,
      startMs: 0,
      hasTime: false,
      done: false,
      updatedMs: 0,
      calendar: calendar,
      collection: collection,
    );
    expect(calendarItemVisible(item(calendar: 'hidden'), settings), isFalse);
    expect(calendarItemVisible(item(calendar: 'deleted'), settings), isTrue);
    expect(calendarItemVisible(item(collection: 'db'), settings), isFalse);
    expect(calendarItemVisible(item(), settings), isTrue);
  });

  test('date-only intervals use UTC calendar dates', () {
    final item = AgendaItem(
      page: 'all-day',
      title: 'All day',
      kind: AgendaKind.task,
      startMs: DateTime.utc(2026, 10, 12).millisecondsSinceEpoch,
      endMs: DateTime.utc(2026, 10, 14).millisecondsSinceEpoch,
      hasTime: false,
      done: false,
      updatedMs: 0,
    );
    expect(calendarItemOnDay(item, DateTime(2026, 10, 11)), isFalse);
    expect(calendarItemOnDay(item, DateTime(2026, 10, 12)), isTrue);
    expect(calendarItemOnDay(item, DateTime(2026, 10, 14)), isTrue);
    expect(calendarItemOnDay(item, DateTime(2026, 10, 15)), isFalse);
  });
}
