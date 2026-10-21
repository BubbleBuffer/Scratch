// Pure reminder planning. Timed values are instants; date-only values are
// UTC calendar dates, as in Today. Scheduling backends convert instants to UTC.
import 'dart:convert';

import '../../core/models.dart';
import '../../ui/format.dart' show displayTitle;

enum ReminderKind { event, task, summary }

class ReminderSettings {
  const ReminderSettings({
    this.enabled = true,
    this.leadMinutes = 10,
    this.morning = 480,
    this.permissionAsked = false,
    this.sourceNames = const {},
  });

  final bool enabled;
  final int leadMinutes;
  final int morning;
  final bool permissionAsked;
  // Loaded page titles, rather than device preferences. AgendaItem carries
  // source ids only, so the coordinator supplies their names to the planner.
  final Map<String, String> sourceNames;

  ReminderSettings copyWith({bool? enabled, int? leadMinutes, int? morning,
      bool? permissionAsked, Map<String, String>? sourceNames}) => ReminderSettings(
    enabled: enabled ?? this.enabled,
    leadMinutes: leadMinutes ?? this.leadMinutes,
    morning: morning ?? this.morning,
    permissionAsked: permissionAsked ?? this.permissionAsked,
    sourceNames: sourceNames ?? this.sourceNames,
  );

  @override
  bool operator ==(Object other) => other is ReminderSettings &&
      enabled == other.enabled && leadMinutes == other.leadMinutes &&
      morning == other.morning && permissionAsked == other.permissionAsked &&
      sourceNames.length == other.sourceNames.length &&
      sourceNames.entries.every((e) => other.sourceNames[e.key] == e.value);

  @override
  int get hashCode => Object.hash(enabled, leadMinutes, morning,
      permissionAsked, Object.hashAllUnordered(sourceNames.entries.map((e) => Object.hash(e.key, e.value))));
}

class PlannedReminder {
  const PlannedReminder({required this.id, required this.kind,
    required this.instant, required this.title, required this.body,
    required this.payload});

  final int id;
  final ReminderKind kind;
  final DateTime instant;
  final String title;
  final String body;
  final String payload;

  @override
  bool operator ==(Object other) => other is PlannedReminder &&
      id == other.id && kind == other.kind && instant == other.instant &&
      title == other.title && body == other.body && payload == other.payload;

  @override
  int get hashCode => Object.hash(id, kind, instant, title, body, payload);
}

List<PlannedReminder> planReminders({required List<AgendaItem> items,
    required int overdueNow, required ReminderSettings settings,
    required DateTime now}) {
  if (!settings.enabled) return const [];
  final end = now.add(const Duration(days: 7));
  bool inWindow(DateTime instant) => instant.isAfter(now) && !instant.isAfter(end);
  final plan = <PlannedReminder>[];
  for (final item in items) {
    if (!item.hasTime || (item.kind == AgendaKind.task && item.done)) continue;
    final start = DateTime.fromMillisecondsSinceEpoch(item.startMs.toInt());
    final event = item.kind == AgendaKind.event;
    final instant = event ? start.subtract(Duration(minutes: settings.leadMinutes)) : start;
    if (!inWindow(instant)) continue;
    final kind = event ? ReminderKind.event : ReminderKind.task;
    final source = item.block == null ? (item.collection ?? item.page) : item.page;
    final eventEnd = item.endMs == null ? '' : '–${_time(DateTime.fromMillisecondsSinceEpoch(item.endMs!.toInt()))}';
    plan.add(PlannedReminder(
      id: _id(kind, item.page, item.block ?? '', instant),
      kind: kind,
      instant: instant,
      title: item.title,
      body: event
          ? '${_time(start)}$eventEnd · ${settings.leadMinutes == 0 ? 'now' : 'in ${settings.leadMinutes} min'}'
          : 'Due now · ${displayTitle(settings.sourceNames[source] ?? '')}',
      payload: jsonEncode({'page': item.page, 'block': item.block}),
    ));
  }

  final localNow = now.toLocal();
  final today = DateTime(localNow.year, localNow.month, localNow.day);
  // Calendar construction preserves local morning time across DST changes.
  for (var offset = 0; ; offset++) {
    final day = DateTime(today.year, today.month, today.day + offset);
    final instant = DateTime(day.year, day.month, day.day,
        settings.morning ~/ 60, settings.morning % 60);
    if (instant.isAfter(end)) break;
    if (!inWindow(instant)) continue;
    var due = 0;
    var overdue = overdueNow;
    var events = 0;
    for (final item in items) {
      if (item.kind == AgendaKind.event) {
        if (_overlaps(item, day)) events++;
      } else if (!item.done) {
        if (_overlaps(item, day)) due++;
        // The core counts items ending before today in overdueNow. Carry
        // forward only tasks that finish today or later, before this day.
        final last = (item.endMs ?? item.startMs).toInt();
        final todayStart = _dayMs(today, timed: item.hasTime);
        if (last >= todayStart && last < _dayMs(day, timed: item.hasTime)) overdue++;
      }
    }
    if (due == 0 && overdue == 0) continue;
    final tasks = <String>[
      if (due > 0) '$due ${due == 1 ? 'task' : 'tasks'} due',
      if (overdue > 0) '$overdue overdue',
    ].join(', ');
    plan.add(PlannedReminder(
      id: _id(ReminderKind.summary, '', '', instant),
      kind: ReminderKind.summary,
      instant: instant,
      title: 'Today',
      body: '$tasks${events > 0 ? ' · $events ${events == 1 ? 'event' : 'events'}' : ''}',
      payload: 'today',
    ));
  }
  plan.sort((a, b) {
    final order = a.instant.compareTo(b.instant);
    return order == 0 ? a.id.compareTo(b.id) : order;
  });
  return plan;
}

String _time(DateTime instant) => '${instant.hour.toString().padLeft(2, '0')}:${instant.minute.toString().padLeft(2, '0')}';

int _dayMs(DateTime day, {required bool timed}) =>
    (timed ? DateTime(day.year, day.month, day.day) : DateTime.utc(day.year, day.month, day.day)).millisecondsSinceEpoch;

bool _overlaps(AgendaItem item, DateTime day) =>
    item.startMs.toInt() < _dayMs(DateTime(day.year, day.month, day.day + 1), timed: item.hasTime) &&
    (item.endMs ?? item.startMs).toInt() >= _dayMs(day, timed: item.hasTime);

// FNV-1a is deterministic across Dart processes and platforms, unlike hashCode.
int _id(ReminderKind kind, String page, String block, DateTime instant) {
  var hash = 0x811c9dc5;
  for (final byte in utf8.encode('${kind.name}|$page|$block|${instant.millisecondsSinceEpoch}')) {
    hash = ((hash ^ byte) * 0x01000193) & 0xffffffff;
  }
  final positive = hash & 0x7fffffff;
  return positive == 0 ? 1 : positive;
}
