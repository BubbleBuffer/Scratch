// The schedule view's state: the stored definition, the runner's status and
// live state, and every edit. Each edit writes only the top-level keys it
// changed, then asks the engine for the preview and the runner's state again.
// Hashes, titles and next runs are the engine's; this file never computes
// them.

import 'dart:convert';

import 'package:flutter/foundation.dart';

import 'backend.dart';
import 'format.dart';
import 'storage.dart';

/// Schedules are not offered as runners after this long without a sign of
/// life, unless a schedule still uses them.
const staleNodeAfter = Duration(days: 30);

class ScheduleController extends ChangeNotifier {
  ScheduleController({
    required this.pageId,
    required this.storage,
    required this.backend,
    required this.deviceTimeZone,
    this.localDeviceId = '',
    DateTime Function()? now,
  }) : now = now ?? DateTime.now;

  final String pageId;
  final ScheduleStorage storage;
  final ScheduleBackend backend;

  /// The IANA zone a new schedule uses (the device's).
  final String deviceTimeZone;

  /// The device this client runs on, to tell its own runner from another
  /// desktop's.
  final String localDeviceId;
  final DateTime Function() now;

  /// The definition as the view shows it. Never null after [load].
  ScheduleDef? _def;
  ScheduleDef get def => _def ?? ScheduleDef.blank(tz: deviceTimeZone, at: '');

  /// The definition as last stored (the base of every write).
  Map<String, Object?> _stored = const {};

  /// The runner's synced status (`schedule_status`), shown when the runner
  /// cannot be asked.
  ScheduleStatus status = ScheduleStatus.none;

  /// The engine's check of the definition as the view holds it.
  SchedulePreview? preview;

  /// The runner's answer, when it gave one.
  ScheduleLiveState? live;

  /// Why the runner did not answer, when it did not.
  String? hostError;

  /// Page titles of the step's template and parent (null: no such page).
  final Map<String, String?> _titles = {};

  List<ScheduleNode> nodes = const [];

  bool loaded = false;

  /// Set when the last action failed; shown next to the action.
  String? actionError;

  /// Set when the typed time or day of month is invalid; nothing is written.
  String? timeError, monthDayError;

  bool busy = false;

  Future<void> load() async {
    final raw = decodeObject(await storage.area(pageId, scheduleAddonId));
    _stored = raw ?? const {};
    _def = ScheduleDef.fromJson(raw ?? const {}, tz: deviceTimeZone);
    if (raw == null) {
      // A page with no schedule yet gets a blank draft; nothing is stored
      // until the user edits it.
      _def = ScheduleDef.blank(tz: deviceTimeZone, at: localAtString(now()));
    }
    status = ScheduleStatus.fromJson(
      decodeObject(await storage.area(pageId, scheduleStatusKey)),
    );
    nodes = await _safeNodes();
    await _loadTitles();
    loaded = true;
    notifyListeners();
    await refreshEngine();
  }

  /// Asks the engine to check the definition and the runner for its state.
  /// Errors become the reason next to the form, so the form stays usable.
  /// A runner that is another desktop is never asked.
  Future<void> refreshEngine() async {
    final d = _def;
    if (d == null) return;
    try {
      preview = await backend.preview(jsonEncode(d.toJson()));
    } catch (e) {
      preview = SchedulePreview(ok: false, reason: _message(e));
    }
    live = null;
    hostError = null;
    final runner = d.runner;
    final node = runner == null ? null : nodeOf(runner);
    if (runner != null && node != null && !_isOtherDesktop(node)) {
      try {
        live = await backend.state(pageId, runner);
      } catch (e) {
        hostError = _message(e);
      }
    }
    notifyListeners();
  }

  Future<List<ScheduleNode>> _safeNodes() async {
    try {
      return await backend.listNodes();
    } catch (_) {
      return const [];
    }
  }

  Future<void> _loadTitles() async {
    final first = def.firstStep;
    for (final link in [first?.template, first?.parent]) {
      final id = pageIdOf(link);
      if (id != null) _titles[id] = await storage.titleOf(id);
    }
  }

  /// The title of the page a stored `[[id]]` link points to: null when the
  /// link is empty or the page is gone (see [hasPageTitle]).
  String? titleOf(String? link) {
    final id = pageIdOf(link);
    if (id == null) return null;
    return _titles[id];
  }

  bool hasPageTitle(String? link) {
    final id = pageIdOf(link);
    return id != null && _titles.containsKey(id);
  }

  bool _isOtherDesktop(ScheduleNode node) =>
      node.kind != 'hub' && node.deviceId != localDeviceId;

  /// Runners the picker offers: available nodes seen recently, plus the
  /// current runner whatever its state.
  List<ScheduleNode> get pickerNodes {
    final cutoff = now().subtract(staleNodeAfter);
    return [
      for (final node in nodes)
        if (node.id == def.runner ||
            (node.available &&
                (node.lastSeen == null || node.lastSeen!.isAfter(cutoff))))
          node,
    ];
  }

  ScheduleNode? nodeOf(String? id) {
    if (id == null) return null;
    for (final node in nodes) {
      if (node.id == id) return node;
    }
    return null;
  }

  /// True when the current runner is another desktop: this client shows its
  /// synced status and sends it nothing.
  bool get runnerIsOtherDesktop {
    final node = nodeOf(def.runner);
    return node != null && _isOtherDesktop(node);
  }

  /// The refusal this client gives without asking the runner: a newer format,
  /// more than one step, or a step kind it cannot run.
  String? get clientRefusal {
    final d = def;
    if (d.version > scheduleFormatVersion) {
      return 'This schedule needs a newer version of the app '
          '(format ${d.version}). It is not run here.';
    }
    if (d.steps.length > 1) {
      return 'This version runs one step per schedule. The other steps are '
          'shown read-only and will not run.';
    }
    final first = d.firstStep;
    if (first != null && !first.isNoteFromTemplate) {
      return 'This version cannot run the step "${first.kind}".';
    }
    return null;
  }

  /// The refusal to show: this client's, the runner's, or its synced status.
  String? get refusal =>
      clientRefusal ??
      (live?.runnerState == 'refused' ? live?.reason : null) ??
      (status.runnerState == ScheduleHealth.refused ? status.reason : null);

  /// Health of the schedule on its runner, for the chip and the card.
  ScheduleHealth get health {
    final d = def;
    if (d.runner == null) return ScheduleHealth.noRunner;
    if (clientRefusal != null) return ScheduleHealth.refused;
    final node = nodeOf(d.runner);
    if (node == null) return ScheduleHealth.noRunner;
    if (runnerIsOtherDesktop) return ScheduleHealth.elsewhere;
    final l = live;
    if (l == null) return ScheduleHealth.unreachable;
    return switch (l.runnerState) {
      'refused' => ScheduleHealth.refused,
      'runner_off' => ScheduleHealth.runnerOff,
      'paused' => ScheduleHealth.paused,
      'needs_acceptance' => ScheduleHealth.needsAcceptance,
      _ =>
        !l.accepted || !l.hashMatches
            ? ScheduleHealth.needsAcceptance
            : ScheduleHealth.ok,
    };
  }

  /// The hash to accept with: the engine's check of the definition on
  /// screen, else the runner's own hash. Null when the engine gave neither.
  String? get acceptHash => preview?.currentHash ?? live?.currentHash;

  /// Next runs: the engine's preview, else the runner's, else the synced one.
  List<DateTime> get nextRuns {
    final runs = preview?.nextRuns ?? const <DateTime>[];
    if (runs.isNotEmpty) return runs.take(3).toList();
    final stored = live?.nextRun ?? status.nextRun;
    return stored == null ? const [] : [stored];
  }

  /// The newest run that was not skipped: the runner's answer, else the
  /// synced status.
  ScheduleLastRun? get lastRun {
    final runs = live?.lastRuns ?? const <ScheduleRunRecord>[];
    for (final run in runs.reversed) {
      if (run.result == 'skipped') continue;
      return ScheduleLastRun(
        occurrence: run.occurrence,
        at: run.at,
        result: run.result,
        page: run.page,
        error: run.error,
      );
    }
    return status.lastRun;
  }

  // ---- edits -----------------------------------------------------------

  /// Sets the frequency. [ScheduleFreq.once] removes the rule.
  Future<void> setFreq(ScheduleFreq freq) async {
    final trigger = def.trigger;
    final rule = trigger.rule ?? ScheduleRule.parse('FREQ=DAILY');
    final next = freq == ScheduleFreq.once
        ? trigger.copyWith(rrule: null)
        : trigger.copyWith(
            rrule: rule!
                .withFreq(
                  freq,
                  defaultDay: _dayCodeOf(trigger),
                  defaultMonthDay: _dayOfMonthOf(trigger),
                )
                .toText(),
          );
    await _commit(def.copyWith(trigger: next));
  }

  /// Sets the weekdays of a weekly rule. The last weekday cannot be cleared.
  Future<void> setWeekdays(List<String> days) async {
    final rule = def.trigger.rule;
    if (rule == null || rule.freq != ScheduleFreq.weekly || days.isEmpty) {
      return;
    }
    await _commit(
      def.copyWith(
        trigger: def.trigger.copyWith(rrule: rule.withByDay(days).toText()),
      ),
    );
  }

  /// Sets the day of month of a monthly rule (1 to 31).
  Future<void> setMonthDay(String text) async {
    final day = int.tryParse(text.trim());
    if (day == null || day < 1 || day > 31) {
      monthDayError = 'Use a day from 1 to 31.';
      notifyListeners();
      return;
    }
    monthDayError = null;
    final rule = def.trigger.rule;
    if (rule == null || rule.freq != ScheduleFreq.monthly) return;
    await _commit(
      def.copyWith(
        trigger: def.trigger.copyWith(rrule: rule.withMonthDay(day).toText()),
      ),
    );
  }

  /// Sets the start time (`HH:MM`). An invalid time is shown and not stored.
  Future<void> setTime(String hhmm) async {
    final problem = validateTime(hhmm);
    timeError = problem;
    if (problem != null) {
      notifyListeners();
      return;
    }
    await _commit(
      def.copyWith(
        trigger: def.trigger.copyWith(
          at: withTime(def.trigger.at, hhmm.trim()),
        ),
      ),
    );
  }

  Future<void> setTimeZone(String zone) async {
    final value = zone.trim();
    if (value.isEmpty || value == def.trigger.tz) return;
    await _commit(def.copyWith(trigger: _trigger(tz: value)));
  }

  Future<void> setMissed(String missed) async {
    if (missed == def.trigger.missed) return;
    await _commit(def.copyWith(trigger: _trigger(missed: missed)));
  }

  Future<void> setRunner(String runnerId) async {
    if (runnerId == def.runner) return;
    await _commit(def.copyWith(runner: runnerId));
  }

  /// Sets the title template of the first step.
  Future<void> setStepTitle(String title) =>
      _editFirst((s) => s.copyWith(title: title));

  /// Sets the template page of the first step (null clears it).
  Future<void> setStepTemplate(String? pageId) async {
    await _editFirst(
      (s) => s.copyWith(template: pageId == null ? null : pageLink(pageId)),
    );
    await _loadTitles();
    notifyListeners();
  }

  /// Sets the parent page of the first step (null clears it).
  Future<void> setStepParent(String? pageId) async {
    await _editFirst(
      (s) => s.copyWith(parent: pageId == null ? null : pageLink(pageId)),
    );
    await _loadTitles();
    notifyListeners();
  }

  ScheduleTrigger _trigger({String? tz, String? missed}) =>
      def.trigger.copyWith(tz: tz, missed: missed);

  Future<void> _editFirst(ScheduleStep Function(ScheduleStep) edit) async {
    final current = def;
    final first =
        current.firstStep ??
        ScheduleStep(id: 'note', kind: scheduleNoteStepKind);
    final steps = [edit(first), ...current.steps.skip(1)];
    await _commit(current.copyWith(steps: steps));
  }

  Future<void> _commit(ScheduleDef next) async {
    final changes = changedKeys(_stored, next.toJson());
    final previous = _def;
    _def = next;
    actionError = null;
    notifyListeners();
    try {
      for (final entry in changes.entries) {
        if (entry.value == null) {
          await storage.removeKey(pageId, scheduleAddonId, entry.key);
        } else {
          await storage.setKey(
            pageId,
            scheduleAddonId,
            entry.key,
            jsonEncode(entry.value),
          );
        }
      }
      _stored = next.toJson();
    } catch (e) {
      _def = previous;
      actionError =
          'Could not save this change. The schedule is unchanged: ${_message(e)}';
      notifyListeners();
      return;
    }
    await refreshEngine();
  }

  // ---- runner actions ----------------------------------------------------

  /// Accepts the definition the view shows, on its runner. The engine's hash
  /// makes the runner accept exactly what the user reviewed.
  Future<void> accept() {
    final hash = acceptHash;
    if (hash == null) {
      actionError = 'The runner has not described this schedule yet. Try again '
          'when it answers.';
      notifyListeners();
      return Future.value();
    }
    return _runnerAction(() async {
      live = await backend.accept(pageId, def.runner!, hash);
    });
  }

  Future<void> setPaused(bool paused) => _runnerAction(() async {
    live = await backend.pause(pageId, def.runner!, paused);
  });

  Future<void> runNow() => _runnerAction(() async {
    await backend.runNow(pageId, def.runner!);
  });

  Future<void> _runnerAction(Future<void> Function() action) async {
    if (def.runner == null || busy) return;
    busy = true;
    actionError = null;
    notifyListeners();
    try {
      await action();
    } catch (e) {
      actionError = _message(e);
    } finally {
      busy = false;
    }
    await refreshEngine();
  }

  String _dayCodeOf(ScheduleTrigger trigger) {
    final date = DateTime.tryParse(trigger.date);
    if (date == null) return 'MO';
    return weekdayCodes[date.weekday - 1];
  }

  int _dayOfMonthOf(ScheduleTrigger trigger) =>
      DateTime.tryParse(trigger.date)?.day ?? 1;

  static String _message(Object e) {
    if (e is ScheduleUnavailable) return e.reason;
    return e.toString().replaceFirst('Exception: ', '');
  }
}
