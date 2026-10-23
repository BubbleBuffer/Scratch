// Per-vault reminder projection; settings and permission history stay local.
import 'dart:async';

import 'package:flutter/foundation.dart';

import '../core/change_feed.dart';
import '../core/clock.dart';
import '../core/debounce.dart';
import '../core/models.dart';
import '../core/paths.dart';
import '../core/today_timers.dart';
import '../core/vault.dart';
import '../features/reminders/backends.dart';
import '../features/reminders/reminder_plan.dart';
import '../features/settings/settings_store.dart';
import '../ui/format.dart' show displayTitle;
import 'agenda_store.dart';
import 'pages_store.dart' show firstLineOfTexts;

class ReminderStore extends ChangeNotifier {
  ReminderStore(this.vault, this.feed, this.settings, {ReminderBackend? backend}) {
    vault.addCloseListener(_onVaultClosed);
    this.backend = backend ?? createReminderBackend(settings, toast: (title, body) => toast?.call(title, body));
  }

  final Vault vault;
  final ChangeFeed feed;
  final AppSettings settings;
  late final ReminderBackend backend;
  bool? permissionGranted;
  String? error;
  List<PlannedReminder> plan = const [];
  final _debounce = Debounce();
  Timer? _midnight;
  StreamSubscription<String>? _taps;
  Future<void> _work = Future.value();
  bool _disposed = false;
  bool get _inactive => _disposed || vault.isClosed;
  bool _started = false;
  bool _dirty = false;
  bool _cancelled = false;
  bool _hasApplied = false;
  ReminderSettings? _lastSettings;
  Future<void> Function(String)? onTap;
  void Function(String, String)? toast;
  final List<String> _pendingTaps = [];
  bool get hasPendingTap => _pendingTaps.isNotEmpty;

  Future<void> start() async {
    if (_started || _inactive) return;
    _started = true;
    _taps = backend.taps.listen(_tap);
    await backend.init();
    BootTiming.mark('backend.init');
    if (_inactive) return;
    feed.addListener(_changed);
    settings.addListener(_settingsChanged);
    _lastSettings = settings.reminders;
    _scheduleMidnight();
    await replan();
  }

  void bind({required Future<void> Function(String) taps, required void Function(String, String) showToast}) {
    onTap = taps;
    toast = showToast;
    for (final payload in _pendingTaps) { _tap(payload); }
    _pendingTaps.clear();
  }

  void unbind() { onTap = null; toast = null; }

  void _tap(String payload) {
    if (_inactive) return;
    final route = onTap;
    if (route == null) { _pendingTaps.add(payload); return; }
    unawaited(route(payload).catchError((Object e) { debugPrint('reminder tap failed: $e'); }));
  }

  void _changed(ChangeBatch _) {
    if (_inactive) return;
    _debounce.run(const Duration(seconds: 2), () => unawaited(replan()));
  }

  void _settingsChanged() {
    final s = settings.reminders;
    final old = _lastSettings;
    _lastSettings = s;
    // Persisting permissionAsked must not recursively request another plan.
    if (old != null && old.enabled == s.enabled && old.leadMinutes == s.leadMinutes && old.morning == s.morning) return;
    _debounce.cancel();
    unawaited(replan());
  }

  void _scheduleMidnight() {
    if (_inactive) return;
    _midnight?.cancel();
    _midnight = Timer(midnightRefreshDelay(clockNow()), () {
      if (_inactive) return;
      unawaited(replan());
      _scheduleMidnight();
    });
  }

  // Serialize backend mutations, retaining reloads that arrive during a query.
  Future<void> replan() {
    if (_inactive) return Future.value();
    _dirty = true;
    _work = _work.then((_) async {
      while (_dirty && !_inactive) {
        _dirty = false;
        try { await _replan(); }
        catch (e) {
          if (_inactive) return;
          error = 'Could not update reminders. Try again.';
          debugPrint('reminder replan failed: $e');
          notifyListeners();
        }
      }
    });
    return _work;
  }

  Future<void> _replan() async {
    final s = settings.reminders;
    if (!s.enabled) {
      if (!_cancelled) await backend.cancelAll();
      if (_inactive) return;
      _cancelled = true;
      plan = const [];
      error = null;
      notifyListeners();
      return;
    }
    final now = clockNow();
    final from = calendarDay(now);
    final end = now.add(const Duration(days: 7));
    final to = DateTime(end.year, end.month, end.day + 1);
    final leadEnd = end.add(Duration(minutes: s.leadMinutes, milliseconds: 1));
    final timedTo = leadEnd.isAfter(to) ? leadEnd : to;
    await vault.flush();
    if (_inactive) return;
    final results = await Future.wait<Object>([
      vault.run((v) => v.agenda(timedFromMs: from.millisecondsSinceEpoch, timedToMs: timedTo.millisecondsSinceEpoch,
          dayFromMs: dayStamp(from), dayToMs: dayStamp(to))),
      vault.run((v) => v.overdueCount(timedBeforeMs: from.millisecondsSinceEpoch, dayBeforeMs: dayStamp(from))),
    ]);
    if (_inactive) return;
    final items = results[0] as List<AgendaItem>;
    final names = <String, String>{};
    await Future.wait([for (final id in items.where((i) => i.kind == AgendaKind.task && !i.done && i.hasTime)
        .map(AgendaStore.sourceId).toSet()) () async {
      try {
        final p = await vault.loadPage(id);
        names[id] = displayTitle(p.title, firstLine: firstLineOfTexts(p.blocks.map((b) => b.text)));
      } catch (_) {
        names[id] = displayTitle('');
      }
    }()]);
    if (_inactive) return;
    // A setting change during loading is handled by the queued current plan.
    if (!settings.reminders.enabled || s.leadMinutes != settings.reminders.leadMinutes || s.morning != settings.reminders.morning) {
      _dirty = true;
      return;
    }
    final next = planReminders(items: items, overdueNow: results[1] as int,
        settings: s.copyWith(sourceNames: names), now: now);
    if (_hasApplied && !_cancelled && listEquals(plan, next)) return;
    if (next.isNotEmpty) {
      permissionGranted = await backend.ensurePermission();
      if (_inactive) return;
      if (!settings.reminders.enabled) { _dirty = true; return; }
    }
    await backend.apply(next);
    if (_inactive) return;
    plan = next;
    _hasApplied = true;
    _cancelled = false;
    error = null;
    notifyListeners();
  }

  void _onVaultClosed() {
    _debounce.cancel();
    _midnight?.cancel();
    _dirty = false;
  }

  Future<void> close() async {
    if (_disposed) return;
    vault.removeCloseListener(_onVaultClosed);
    _disposed = true;
    _debounce.cancel();
    _midnight?.cancel();
    feed.removeListener(_changed);
    settings.removeListener(_settingsChanged);
    await _taps?.cancel();
    await _work;
    // Android schedules survive closing the app; only stop runtime resources.
    await backend.dispose();
    super.dispose();
  }
}
