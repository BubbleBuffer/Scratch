// Date helpers of the calendar and timeline: the days an entry covers, and
// its value moved or resized by whole days.

import '../../../core/models.dart';
import '../../../ui/database/values.dart';

Day entryStart(RangeEntry e) => Day.ofMs(e.startMs, e.hasTime);

Day entryEnd(RangeEntry e) {
  final end = e.endMs;
  final s = entryStart(e);
  if (end == null) return s;
  final d = Day.ofMs(end, e.hasTime);
  return d.compareTo(s) < 0 ? s : d;
}

int _shiftMs(int ms, bool hasTime, int days) {
  if (days == 0) return ms;
  if (!hasTime) return Day.ofMs(ms, false).add(days).ms;
  final t = DateTime.fromMillisecondsSinceEpoch(ms);
  return DateTime(t.year, t.month, t.day + days, t.hour, t.minute, t.second).millisecondsSinceEpoch;
}

/// [e]'s value with its start moved by [startDays] and its end by
/// [endDays] (a single date gains an end when they differ).
PropValue shiftedValue(RangeEntry e, int startDays, int endDays) {
  final start = _shiftMs(e.startMs, e.hasTime, startDays);
  final endBase = e.endMs ?? e.startMs;
  final end = _shiftMs(endBase, e.hasTime, endDays);
  final hasEnd = e.endMs != null || startDays != endDays;
  if (!hasEnd || end <= start && !e.hasTime && Day.ofMs(end, false) == Day.ofMs(start, false)) {
    return Values.date(start, hasTime: e.hasTime);
  }
  return Values.date(start, end: end < start ? start : end, hasTime: e.hasTime);
}

/// The ms range [from, to) that covers the days [first, last] whatever the
/// time zone of timed values.
(int, int) queryRangeOf(Day first, Day last) => (first.add(-1).ms, last.add(2).ms);
