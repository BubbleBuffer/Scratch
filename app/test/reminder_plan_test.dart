import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/reminders/reminder_plan.dart';

// The same local clock as the app harness, without loading the Rust bridge.
final now = DateTime(2026, 10, 6, 11, 20);

AgendaItem item(String page, DateTime start, {AgendaKind kind = AgendaKind.task,
    bool done = false, bool timed = true, DateTime? end, String? block,
    String title = 'A reminder', String? collection}) => AgendaItem(
  page: page, block: block, title: title, kind: kind,
  startMs: start.millisecondsSinceEpoch, endMs: end?.millisecondsSinceEpoch,
  hasTime: timed, done: done, updatedMs: 0, collection: collection,
);

List<PlannedReminder> plan(List<AgendaItem> items, {int overdue = 0,
    ReminderSettings settings = const ReminderSettings(), DateTime? at}) =>
    planReminders(items: items, overdueNow: overdue, settings: settings, now: at ?? now);

List<PlannedReminder> summaries(List<PlannedReminder> reminders) =>
    reminders.where((r) => r.kind == ReminderKind.summary).toList();

void main() {
  test('settings have the agreed defaults and copy by value', () {
    const settings = ReminderSettings();
    expect(settings.enabled, isTrue);
    expect(settings.leadMinutes, 10);
    expect(settings.morning, 480);
    expect(settings.permissionAsked, isFalse);
    expect(settings.copyWith(), settings);
    expect(settings.copyWith(enabled: false).enabled, isFalse);
    expect(const ReminderSettings(sourceNames: {'a': 'A', 'b': 'B'}),
        const ReminderSettings(sourceNames: {'b': 'B', 'a': 'A'}));
  });

  for (final lead in [0, 5, 10, 30]) {
    test('timed event with a $lead minute lead', () {
      final start = DateTime(2026, 10, 6, 14);
      final event = plan([item('event', start, kind: AgendaKind.event,
        end: DateTime(2026, 10, 6, 14, 45), title: 'Appointment')],
        settings: ReminderSettings(leadMinutes: lead)).single;
      expect(event.kind, ReminderKind.event);
      expect(event.instant, start.subtract(Duration(minutes: lead)));
      expect(event.title, 'Appointment');
      expect(event.body, '14:00–14:45 · ${lead == 0 ? 'now' : 'in $lead min'}');
      expect(jsonDecode(event.payload), {'page': 'event', 'block': null});
    });
  }

  test('event without an end has just its start time', () {
    expect(plan([item('event', DateTime(2026, 10, 6, 14),
        kind: AgendaKind.event)], settings: const ReminderSettings(leadMinutes: 0)).single.body,
      '14:00 · now');
  });

  test('timed task uses the source name and carries its block payload', () {
    final reminders = plan([
      item('page', DateTime(2026, 10, 6, 12), block: 'block', title: 'Write notes'),
      item('row', DateTime(2026, 10, 6, 13), collection: 'database'),
    ], settings: const ReminderSettings(sourceNames: {'page': 'Project', 'database': 'Tasks'}));
    final tasks = reminders.where((r) => r.kind == ReminderKind.task).toList();
    expect(tasks[0].instant, DateTime(2026, 10, 6, 12));
    expect(tasks[0].title, 'Write notes');
    expect(tasks[0].body, 'Due now · Project');
    expect(jsonDecode(tasks[0].payload), {'page': 'page', 'block': 'block'});
    expect(tasks[1].body, 'Due now · Tasks');
  });

  test('past items and reminders exactly now are not individually planned', () {
    final reminders = plan([
      item('past-task', now.subtract(const Duration(minutes: 1))),
      item('now-task', now),
      item('past-event', now.subtract(const Duration(minutes: 1)), kind: AgendaKind.event),
      item('lead-now', now.add(const Duration(minutes: 10)), kind: AgendaKind.event),
      item('lead-past', now.add(const Duration(minutes: 5)), kind: AgendaKind.event),
    ]);
    expect(reminders.where((r) => r.kind != ReminderKind.summary), isEmpty);
    // Past times today still become overdue on tomorrow's summary.
    expect(summaries(reminders).first.body, '2 overdue');
  });

  test('done tasks get neither their own reminder nor summary counts', () {
    expect(plan([item('done', DateTime(2026, 10, 7, 14), done: true)]), isEmpty);
  });

  test('all-day items get only summaries and preserve UTC calendar dates', () {
    final reminders = plan([
      item('all-day-task', DateTime.utc(2026, 10, 7), timed: false),
      item('all-day-event', DateTime.utc(2026, 10, 7), timed: false, kind: AgendaKind.event),
    ]);
    expect(reminders.every((r) => r.kind == ReminderKind.summary), isTrue);
    expect(reminders.first.instant, DateTime(2026, 10, 7, 8));
    expect(reminders.first.title, 'Today');
    expect(reminders.first.body, '1 task due · 1 event');
    expect(reminders.first.payload, 'today');
    expect(reminders[1].body, '1 overdue');
  });

  test('summary carries overdue tasks across days without counting old ones twice', () {
    final reminders = summaries(plan([
      item('already-overdue', DateTime.utc(2026, 10, 5), timed: false),
      item('today-past', DateTime(2026, 10, 6, 9)),
      item('today-future', DateTime(2026, 10, 6, 14)),
      item('tomorrow', DateTime.utc(2026, 10, 7), timed: false),
      item('day-after', DateTime.utc(2026, 10, 8), timed: false),
      item('done', DateTime.utc(2026, 10, 7), timed: false, done: true),
      item('event-1', DateTime(2026, 10, 7, 14), kind: AgendaKind.event),
      item('event-2', DateTime.utc(2026, 10, 7), timed: false, kind: AgendaKind.event),
      item('event-3', DateTime(2026, 10, 7, 16), kind: AgendaKind.event),
    ], overdue: 2));
    expect(reminders[0].body, '1 task due, 4 overdue · 3 events');
    expect(reminders[1].body, '1 task due, 5 overdue');
    expect(reminders[2].body, '6 overdue');
  });

  test('summary includes tasks and events overlapping the day like Today', () {
    final reminders = summaries(plan([
      item('range-task', DateTime.utc(2026, 10, 6),
        end: DateTime.utc(2026, 10, 8), timed: false),
      item('range-event', DateTime(2026, 10, 6, 23),
        end: DateTime(2026, 10, 8, 1), kind: AgendaKind.event),
    ]));
    expect(reminders[0].body, '1 task due · 1 event');
    expect(reminders[1].body, '1 task due · 1 event');
    expect(reminders[2].body, '1 overdue');
  });

  test('summary skips empty days and event-only days', () {
    expect(plan([]), isEmpty);
    final reminders = plan([item('event', DateTime(2026, 10, 7, 14), kind: AgendaKind.event)]);
    expect(summaries(reminders), isEmpty);
    expect(reminders.single.kind, ReminderKind.event);
  });

  test('morning time is local and a summary exactly now is excluded', () {
    final at = DateTime(2026, 10, 6, 8);
    final reminders = summaries(plan([], overdue: 1, at: at));
    expect(reminders.first.instant, DateTime(2026, 10, 7, 8));
    expect(reminders.last.instant, DateTime(2026, 10, 13, 8));
    expect(summaries(plan([], overdue: 1,
      settings: const ReminderSettings(morning: 690))).first.instant,
      DateTime(2026, 10, 6, 11, 30));
  });

  test('ids are stable, positive 31-bit and independent of title or input order', () {
    final task = item('task', DateTime(2026, 10, 7, 14), block: 'block');
    final event = item('event', DateTime(2026, 10, 7, 15), kind: AgendaKind.event);
    final first = plan([task, event]);
    expect(plan([event, task]), first);
    expect(first.every((r) => r.id > 0 && r.id <= 0x7fffffff), isTrue);
    expect(first.map((r) => r.id).toSet().length, first.length);
    final edited = plan([item('task', DateTime(2026, 10, 7, 14),
      block: 'block', title: 'Changed title'), event]);
    expect(edited.map((r) => r.id), first.map((r) => r.id));
    expect(edited, isNot(first));
    final shifted = plan([task, event], settings: const ReminderSettings(leadMinutes: 30));
    expect(shifted.firstWhere((r) => r.kind == ReminderKind.event).id,
        isNot(first.firstWhere((r) => r.kind == ReminderKind.event).id));
  });

  test('the seven-day upper edge is inclusive for notification instants', () {
    final edge = now.add(const Duration(days: 7));
    final reminders = plan([
      item('edge', edge),
      item('beyond', edge.add(const Duration(milliseconds: 1))),
      item('event-edge', edge.add(const Duration(minutes: 10)), kind: AgendaKind.event),
      item('event-beyond', edge.add(const Duration(minutes: 10, milliseconds: 1)), kind: AgendaKind.event),
    ]).where((r) => r.kind != ReminderKind.summary).toList();
    expect(reminders.length, 2);
    expect(reminders.every((r) => r.instant == edge), isTrue);
    final mornings = summaries(plan([], overdue: 1,
        at: DateTime(2026, 10, 6, 8), settings: const ReminderSettings(morning: 480)));
    expect(mornings.length, 7);
    expect(mornings.last.instant, DateTime(2026, 10, 13, 8));
  });

  test('disabled reminders have no plan, including overdue summaries', () {
    expect(plan([item('task', DateTime(2026, 10, 7, 14))], overdue: 5,
      settings: const ReminderSettings(enabled: false)), isEmpty);
  });
}
