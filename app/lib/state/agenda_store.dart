// One vault's organizer projection. Timed dates are local instants; date-only
// values are UTC midnights representing calendar days.
import 'dart:async';

import 'package:flutter/foundation.dart';

import '../core/change_feed.dart';
import '../core/clock.dart';
import '../core/debounce.dart';
import '../core/models.dart';
import '../core/today_timers.dart';
import '../core/vault.dart';
import '../ui/format.dart' show displayTitle;
import 'pages_store.dart' show firstLineOfTexts;

DateTime calendarDay(DateTime d) => DateTime(d.year, d.month, d.day);
int dayStamp(DateTime d) => DateTime.utc(d.year, d.month, d.day).millisecondsSinceEpoch;
bool sameDay(DateTime a, DateTime b) => a.year == b.year && a.month == b.month && a.day == b.day;

const weekdays = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
const months = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
String dayTitle(DateTime d, {bool year = false}) =>
    '${weekdays[d.weekday - 1]} ${d.day} ${months[d.month - 1]}${year ? ' ${d.year}' : ''}';

class AgendaStore extends ChangeNotifier {
  AgendaStore(this.vault, this.feed) : selectedDay = calendarDay(clockNow()) {
    feed.addListener(_changed);
    vault.addCloseListener(_onVaultClosed);
    _scheduleMidnight();
  }

  final Vault vault;
  final ChangeFeed feed;
  DateTime selectedDay;
  List<AgendaItem> items = const [];
  List<AgendaItem> _todayItems = const [];
  List<AgendaItem> overdueItems = const [];
  List<AgendaItem> completedOverdueItems = const [];
  int overdueLimit = 50;
  int overdueCount = 0;
  List<Progress> inMotion = const [];
  Map<String, String> sourceTitles = {};
  String? error;
  bool loading = true;
  final _reload = Debounce();
  Timer? _midnight;
  bool _disposed = false;
  bool get _inactive => _disposed || vault.isClosed;
  int _generation = 0;
  bool _reloadOwed = false;
  Map<int, int> _dayCounts = {};
  Map<int, bool> _dayOverdue = {};
  final Map<String, bool> _optimistic = {};

  DateTime get today => calendarDay(clockNow());
  DateTime get weekStart => DateTime(selectedDay.year, selectedDay.month, selectedDay.day - selectedDay.weekday + 1);
  List<DateTime> get days => [for (var i = 0; i < 7; i++) DateTime(weekStart.year, weekStart.month, weekStart.day + i)];
  bool get isToday => sameDay(selectedDay, today);
  bool overdueOn(DateTime day) => _dayOverdue[dayStamp(day)] ?? false;

  static String itemKey(AgendaItem item) => '${item.page}/${item.block ?? ''}';
  static String sourceId(AgendaItem item) => item.block == null ? (item.collection ?? item.page) : item.page;
  static int _oldestFirst(AgendaItem a, AgendaItem b) {
    var order = a.startMs.compareTo(b.startMs);
    if (order == 0) order = a.title.compareTo(b.title);
    if (order == 0) order = itemKey(a).compareTo(itemKey(b));
    return order;
  }
  bool busy(AgendaItem item) => _optimistic.containsKey(itemKey(item));

  DateTime itemDay(AgendaItem item) {
    final d = DateTime.fromMillisecondsSinceEpoch(item.startMs.toInt(), isUtc: !item.hasTime);
    return calendarDay(d);
  }

  List<AgendaItem> onDay(DateTime day) => [for (final i in items) if (_overlaps(i, day)) i];
  bool _overlaps(AgendaItem i, DateTime day) {
    final from = i.hasTime ? calendarDay(day).millisecondsSinceEpoch : dayStamp(day);
    final next = DateTime(day.year, day.month, day.day + 1);
    final to = i.hasTime ? next.millisecondsSinceEpoch : dayStamp(next);
    return i.startMs.toInt() < to && (i.endMs ?? i.startMs).toInt() >= from;
  }
  int countOn(DateTime day) => _dayCounts[dayStamp(day)] ?? 0;
  List<AgendaItem> get selectedItems => onDay(selectedDay);
  List<AgendaItem> get selectedDoneItems {
    final done = <String, AgendaItem>{
      for (final i in selectedItems) if (i.kind == AgendaKind.task && i.done) itemKey(i): i,
      if (isToday) for (final i in completedOverdueItems) if (i.done) itemKey(i): i,
    };
    return done.values.toList();
  }

  void _cacheDayMarks() {
    _dayCounts = {for (final day in days) dayStamp(day): onDay(day).length};
    _dayOverdue = {for (final day in days) dayStamp(day): sameDay(day, today) && overdueCount > 0};
  }

  Future<void> showMoreOverdue() async {
    overdueLimit += 50;
    await reload();
  }
  int get eventsToday => _todayItems.where((i) => _overlaps(i, today) && i.kind == AgendaKind.event).length;
  int get tasksDueToday => _todayItems.where((i) => _overlaps(i, today) && i.kind == AgendaKind.task && !i.done).length;

  String get summary {
    final parts = <String>[];
    if (eventsToday > 0) parts.add('$eventsToday ${eventsToday == 1 ? 'event' : 'events'}');
    if (tasksDueToday > 0) parts.add('$tasksDueToday ${tasksDueToday == 1 ? 'task' : 'tasks'} due');
    if (overdueCount > 0) parts.add('$overdueCount overdue');
    return '${dayTitle(today)} · ${parts.isEmpty ? 'Nothing planned' : parts.join(' · ')}';
  }

  void selectDay(DateTime day) {
    final oldWeek = weekStart;
    if (!sameDay(selectedDay, day)) overdueLimit = 50;
    selectedDay = calendarDay(day);
    final changedWeek = !sameDay(oldWeek, weekStart);
    if (changedWeek) loading = true;
    notifyListeners();
    if (changedWeek) unawaited(reload());
  }

  void moveWeek(int delta) => selectDay(DateTime(selectedDay.year, selectedDay.month, selectedDay.day + delta * 7));
  void backToToday() => selectDay(today);

  void _changed(ChangeBatch _) {
    if (_inactive) return;
    _reload.run(const Duration(milliseconds: 150), () => unawaited(reload()));
  }

  void _scheduleMidnight() {
    if (_inactive) return;
    _midnight?.cancel();
    final now = clockNow();
    final oldToday = calendarDay(now);
    _midnight = Timer(midnightRefreshDelay(now), () {
      if (_inactive) return;
      if (sameDay(selectedDay, oldToday)) {
        selectedDay = today;
        overdueLimit = 50;
      }
      unawaited(reload());
      _scheduleMidnight();
    });
  }

  Future<void> reload() async {
    // A feed reload must not replace an optimistic projection halfway through
    // its write. The last pending toggle always reloads the persisted result.
    if (_inactive) return;
    if (_optimistic.isNotEmpty) { _reloadOwed = true; return; }
    _reloadOwed = false;
    final generation = ++_generation;
    final from = weekStart;
    final to = DateTime(from.year, from.month, from.day + 7);
    final nowDay = today;
    final todayOutsideWeek = nowDay.isBefore(from) || !nowDay.isBefore(to);
    try {
      // The feed reports document edits before the delayed writer refreshes
      // block-derived agenda/progress. Wait for its index acknowledgement.
      await vault.flush();
      if (_inactive || generation != _generation) return;
      final results = await Future.wait<Object>([
        vault.run((v) => v.agenda(timedFromMs: from.millisecondsSinceEpoch, timedToMs: to.millisecondsSinceEpoch,
            dayFromMs: dayStamp(from), dayToMs: dayStamp(to))),
        vault.run((v) => v.overdue(timedBeforeMs: nowDay.millisecondsSinceEpoch, dayBeforeMs: dayStamp(nowDay), limit: overdueLimit)),
        vault.run((v) => v.overdueCount(timedBeforeMs: nowDay.millisecondsSinceEpoch, dayBeforeMs: dayStamp(nowDay))),
        vault.run((v) => v.inMotion(limit: 5)),
        vault.run((v) => v.doneSince(timedBeforeMs: nowDay.millisecondsSinceEpoch, dayBeforeMs: dayStamp(nowDay),
            editedSinceMs: nowDay.millisecondsSinceEpoch, limit: 50)),
        if (todayOutsideWeek) vault.run((v) => v.agenda(timedFromMs: nowDay.millisecondsSinceEpoch,
          timedToMs: DateTime(nowDay.year, nowDay.month, nowDay.day + 1).millisecondsSinceEpoch,
          dayFromMs: dayStamp(nowDay), dayToMs: dayStamp(DateTime(nowDay.year, nowDay.month, nowDay.day + 1)))),
      ]);
      if (_inactive || generation != _generation) return;
      final week = results[0] as List<AgendaItem>;
      final lateItems = results[1] as List<AgendaItem>;
      final completed = results[4] as List<AgendaItem>;
      final sources = {for (final i in [...week, ...lateItems, ...completed]) sourceId(i)};
      final titles = <String, String>{};
      await Future.wait([for (final id in sources) () async {
        try {
          final p = await vault.loadPage(id);
          titles[id] = displayTitle(p.title, firstLine: firstLineOfTexts(p.blocks.map((b) => b.text)));
        } catch (_) {
          titles[id] = displayTitle('');
        }
      }()]);
      if (_inactive || generation != _generation) return;
      items = [for (final i in week) _overlay(i)];
      _todayItems = [for (final i in todayOutsideWeek ? results[5] as List<AgendaItem> : week) _overlay(i)];
      overdueItems = [for (final i in lateItems) if (!(_optimistic[itemKey(i)] ?? i.done)) _overlay(i)];
      completedOverdueItems = [for (final i in completed) _overlay(i)];
      overdueCount = results[2] as int;
      // Counts include items outside the limited list; only subtract known
      // overdue items while their optimistic completion is in flight.
      overdueCount -= lateItems.where((i) => !i.done && _optimistic[itemKey(i)] == true).length;
      inMotion = results[3] as List<Progress>;
      sourceTitles = titles;
      _cacheDayMarks();
      loading = false;
      error = null;
      notifyListeners();
    } catch (e) {
      if (_inactive || generation != _generation) return;
      loading = false;
      error = 'Could not load Today. Try again.';
      debugPrint('agenda reload failed: $e');
      notifyListeners();
    }
  }

  AgendaItem _overlay(AgendaItem i) => _withDone(i, _optimistic[itemKey(i)] ?? i.done);
  AgendaItem _withDone(AgendaItem i, bool done) => AgendaItem(page: i.page, block: i.block, title: i.title,
      kind: i.kind, startMs: i.startMs, endMs: i.endMs, hasTime: i.hasTime, done: done,
      status: i.status, statusGroup: i.statusGroup, statusColor: i.statusColor,
      collection: i.collection, space: i.space, spaceColor: i.spaceColor, icon: i.icon, updatedMs: i.updatedMs);

  /// Returns false after reverting a failed write. The caller displays [error]
  /// as an error toast; only one write per item may be in flight.
  Future<bool> toggleDone(AgendaItem item) async {
    final key = itemKey(item);
    if (_inactive || _optimistic.containsKey(key)) return false;
    final done = !item.done;
    _generation++; // discard snapshots started before the optimistic write
    _optimistic[key] = done;
    items = [for (final i in items) itemKey(i) == key ? _withDone(i, done) : i];
    _todayItems = [for (final i in _todayItems) itemKey(i) == key ? _withDone(i, done) : i];
    overdueItems = [for (final i in overdueItems) itemKey(i) == key ? _withDone(i, done) : i];
    final datedBeforeToday = item.kind == AgendaKind.task &&
        (item.endMs ?? item.startMs).toInt() < (item.hasTime ? today.millisecondsSinceEpoch : dayStamp(today));
    if (datedBeforeToday) {
      overdueCount += done ? -1 : 1;
      completedOverdueItems = [for (final i in completedOverdueItems) if (itemKey(i) != key) i,
        if (done) _withDone(item, true)];
      overdueItems = [for (final i in overdueItems) if (itemKey(i) != key) i,
        if (!done) _withDone(item, false)]..sort(_oldestFirst);
      _dayOverdue = {for (final day in days) dayStamp(day): sameDay(day, today) && overdueCount > 0};
    }
    error = null;
    notifyListeners();
    var success = false;
    String? failure;
    try {
      await vault.run((v) => v.setAgendaDone(page: item.page, block: item.block, done: done), writes: true);
      // Block writes are indexed by the core's delayed writer. Keep the
      // optimistic value until that projection has caught up.
      await vault.flush();
      success = true;
      _reloadOwed = true;
    } catch (e) {
      if (_inactive) return false;
      items = [for (final i in items) itemKey(i) == key ? _withDone(i, item.done) : i];
      _todayItems = [for (final i in _todayItems) itemKey(i) == key ? _withDone(i, item.done) : i];
      overdueItems = [for (final i in overdueItems) itemKey(i) == key ? _withDone(i, item.done) : i];
      if (datedBeforeToday) {
        overdueCount += done ? 1 : -1;
        completedOverdueItems = [for (final i in completedOverdueItems) if (itemKey(i) != key) i,
          if (item.done) _withDone(item, true)];
        overdueItems = [for (final i in overdueItems) if (itemKey(i) != key) i,
          if (!item.done) _withDone(item, false)]..sort(_oldestFirst);
        _dayOverdue = {for (final day in days) dayStamp(day): sameDay(day, today) && overdueCount > 0};
      }
      failure = 'Could not update the task. Try again.';
      error = failure;
      debugPrint('agenda toggle failed: $e');
      notifyListeners();
    } finally {
      _optimistic.remove(key);
      if (!_inactive && _optimistic.isEmpty && _reloadOwed) {
        await reload();
        if (!_inactive && failure != null) { error = failure; notifyListeners(); }
      }
    }
    return success;
  }

  void _onVaultClosed() {
    _reload.cancel();
    _generation++;
    _midnight?.cancel();
    _reloadOwed = false;
  }

  @override
  void dispose() {
    vault.removeCloseListener(_onVaultClosed);
    _disposed = true;
    _generation++;
    feed.removeListener(_changed);
    _reload.cancel();
    _midnight?.cancel();
    super.dispose();
  }
}

