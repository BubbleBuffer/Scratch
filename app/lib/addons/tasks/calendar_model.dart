import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import '../../core/change_feed.dart';
import '../../core/debounce.dart';
import '../../core/models.dart';
import '../../core/vault.dart';
import '../../state/agenda_store.dart';
import '../../ui/format.dart';
import 'task_data.dart';

DateTime calendarMonday(DateTime day) =>
    DateTime(day.year, day.month, day.day - day.weekday + 1);
DateTime calendarAddDays(DateTime day, int amount) =>
    DateTime(day.year, day.month, day.day + amount);
DateTime calendarMonth(DateTime day, [int offset = 0]) =>
    DateTime(day.year, day.month + offset);
String calendarDateKey(DateTime day) =>
    '${day.year}-${day.month.toString().padLeft(2, '0')}-${day.day.toString().padLeft(2, '0')}';
String calendarMonthTitle(DateTime day) =>
    '${months[day.month - 1]} ${day.year}';
int calendarMonthWeeks(DateTime month) =>
    ((dayStamp(calendarMonth(month, 1)) - dayStamp(calendarMonday(month))) /
            86400000 /
            7)
        .ceil();
String calendarShortDate(DateTime day) =>
    '${weekdays[day.weekday - 1].substring(0, 3)} ${day.day} ${months[day.month - 1].substring(0, 3)}';
String calendarTime(int minute) =>
    '${(minute ~/ 60).toString().padLeft(2, '0')}:${(minute % 60).toString().padLeft(2, '0')}';

bool calendarItemVisible(AgendaItem item, CalendarSettings settings) {
  if (item.collection != null) {
    return !settings.databases.any(
      (d) => d.id == item.collection && !d.showInCalendar,
    );
  }
  if (item.calendar == null) return true;
  for (final c in settings.calendars) {
    if (c.id == item.calendar) return c.showInCalendar;
  }
  // Deleted categories keep their dated items visible, matching core.
  return true;
}

DateTime calendarItemDay(AgendaItem item) {
  final date = DateTime.fromMillisecondsSinceEpoch(
    item.startMs.toInt(),
    isUtc: !item.hasTime,
  );
  return calendarDay(date);
}

bool calendarItemOnDay(AgendaItem item, DateTime day) {
  final start = item.startMs.toInt();
  final from = item.hasTime
      ? calendarDay(day).millisecondsSinceEpoch
      : dayStamp(day);
  final next = calendarAddDays(day, 1);
  final to = item.hasTime ? next.millisecondsSinceEpoch : dayStamp(next);
  final end = item.endMs?.toInt();
  // Timed end bounds are exclusive, so midnight does not make an empty block.
  return start < to &&
      (end == null || end <= start
          ? start >= from
          : item.hasTime
          ? end > from
          : end >= from);
}

class CalendarTimeSegment {
  CalendarTimeSegment(
    this.item,
    this.startMinute,
    this.endMinute,
    this.continued,
  );
  final AgendaItem item;
  final int startMinute, endMinute;
  final bool continued;
  int lane = 0;
  int lanes = 1;
}

/// Clip each timed interval against local midnights, including DST boundaries.
List<CalendarTimeSegment> calendarSegments(
  List<AgendaItem> items,
  DateTime day,
) {
  final first = calendarDay(day);
  final next = calendarAddDays(first, 1);
  final result = <CalendarTimeSegment>[];
  for (final item in items) {
    if (!item.hasTime || !calendarItemOnDay(item, day)) continue;
    final start = DateTime.fromMillisecondsSinceEpoch(item.startMs.toInt());
    final end = DateTime.fromMillisecondsSinceEpoch(
      item.endMs?.toInt() ?? item.startMs.toInt() + 3600000,
    );
    final clippedStart = start.isBefore(first) ? first : start;
    final clippedEnd = end.isAfter(next) ? next : end;
    final from = clippedStart.hour * 60 + clippedStart.minute;
    final to = !clippedEnd.isBefore(next)
        ? 1440
        : clippedEnd.hour * 60 + clippedEnd.minute;
    result.add(
      CalendarTimeSegment(
        item,
        from,
        math.max(from + 1, to),
        start.isBefore(first),
      ),
    );
  }
  result.sort((a, b) {
    final order = a.startMinute.compareTo(b.startMinute);
    return order == 0 ? b.endMinute.compareTo(a.endMinute) : order;
  });
  var active = <CalendarTimeSegment>[];
  var group = <CalendarTimeSegment>[];
  void finishGroup() {
    if (group.isEmpty) return;
    final width = group.map((s) => s.lane + 1).reduce(math.max);
    for (final segment in group) {
      segment.lanes = width;
    }
    group = [];
  }

  for (final segment in result) {
    active.removeWhere((s) => s.endMinute <= segment.startMinute);
    if (active.isEmpty) finishGroup();
    var lane = 0;
    while (active.any((s) => s.lane == lane)) {
      lane++;
    }
    segment.lane = lane;
    active.add(segment);
    group.add(segment);
  }
  finishGroup();
  return result;
}

class VaultCalendarStore extends ChangeNotifier {
  VaultCalendarStore(this.vault, this.feed) {
    feed.addListener(_changed);
    vault.addCloseListener(_closed);
  }
  final Vault vault;
  final ChangeFeed feed;
  final _debounce = Debounce();
  DateTime? from, to;
  List<AgendaItem> items = const [];
  CalendarSettings settings = const CalendarSettings(
    calendars: [],
    databases: [],
  );
  Map<String, String> sourceTitles = {};
  Map<String, bool> checkboxes = {};
  bool loading = true;
  String? error;
  int _generation = 0;
  bool _disposed = false;

  List<AgendaItem> onDay(DateTime day) => [
    for (final item in items)
      if (calendarItemVisible(item, settings) && calendarItemOnDay(item, day))
        item,
  ];

  void _changed(ChangeBatch _) => _debounce.run(
    const Duration(milliseconds: 150),
    () => unawaited(reload()),
  );
  Future<void> loadRange(DateTime first, DateTime last) async {
    if (from != null && sameDay(from!, first) && sameDay(to!, last)) return;
    from = first;
    to = last;
    loading = true;
    notifyListeners();
    await reload();
  }

  Future<void> reload() async {
    if (_disposed || vault.isClosed || from == null || to == null) return;
    final generation = ++_generation;
    try {
      await vault.flush();
      final results = await Future.wait<Object>([
        vault.calendarItems(
          timedFromMs: from!.millisecondsSinceEpoch,
          timedToMs: to!.millisecondsSinceEpoch,
          dayFromMs: dayStamp(from!),
          dayToMs: dayStamp(to!),
        ),
        vault.listCalendars(),
      ]);
      if (_disposed || vault.isClosed || generation != _generation) return;
      final loaded = results[0] as List<AgendaItem>;
      final titles = <String, String>{};
      final loadedCheckboxes = <String, bool>{};
      final sources = {for (final item in loaded) AgendaStore.sourceId(item)};
      await Future.wait([
        for (final id in sources)
          () async {
            try {
              titles[id] = displayTitle((await vault.loadPage(id)).title);
            } catch (_) {
              titles[id] = 'Untitled';
            }
          }(),
      ]);
      await Future.wait([
        for (final item in loaded.where(
          (i) =>
              i.kind == AgendaKind.task &&
              i.block == null &&
              i.collection == null,
        ))
          () async {
            try {
              loadedCheckboxes[item.page] = (await loadTask(
                vault,
                item.page,
              )).checkbox;
            } catch (_) {
              loadedCheckboxes[item.page] = true;
            }
          }(),
      ]);
      if (_disposed || vault.isClosed || generation != _generation) return;
      items = loaded;
      settings = results[1] as CalendarSettings;
      sourceTitles = titles;
      checkboxes = loadedCheckboxes;
      loading = false;
      error = null;
      notifyListeners();
    } catch (e) {
      if (_disposed || vault.isClosed || generation != _generation) return;
      loading = false;
      error = 'Could not load Calendar. Try again.';
      debugPrint('calendar reload failed: $e');
      notifyListeners();
    }
  }

  void _closed() {
    _generation++;
    _debounce.cancel();
  }

  @override
  void dispose() {
    _disposed = true;
    _closed();
    feed.removeListener(_changed);
    vault.removeCloseListener(_closed);
    super.dispose();
  }
}
