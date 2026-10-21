// In-memory stand-ins for the engine and the vault, used by the widget tests
// and goldens. They follow the engine's API shape (hashes, next runs, runner
// states) without a running engine. Titles, hashes and next runs here are
// stand-ins for the layout checks; the engine owns the real rules.

import 'dart:convert';

import 'backend.dart';
import 'format.dart';
import 'storage.dart';

/// Keeps schedule areas and page titles in memory, as the vault does.
class InMemoryScheduleStorage implements ScheduleStorage {
  InMemoryScheduleStorage({
    Map<String, Map<String, Map<String, Object?>>>? seed,
    Map<String, String> titles = const {},
  }) : _titles = Map.of(titles),
       _areas = {
         if (seed != null)
           for (final page in seed.entries)
             page.key: {
               for (final addon in page.value.entries)
                 addon.key: {
                   for (final key in addon.value.entries)
                     key.key: jsonEncode(key.value),
                 },
             },
       };

  // page id -> add-on id -> key -> JSON text.
  final Map<String, Map<String, Map<String, String>>> _areas;

  // page id -> title, for template and parent pages.
  final Map<String, String> _titles;

  /// Every key written, as `page/addon/key`, in order (`-key` for removals).
  final List<String> writes = [];

  /// The stored area, decoded (empty when absent).
  Map<String, Object?> object(String pageId, String addon) {
    final keys = _areas[pageId]?[addon] ?? const <String, String>{};
    return {for (final e in keys.entries) e.key: jsonDecode(e.value)};
  }

  @override
  Future<String> area(String pageId, String addon) async =>
      jsonEncode(object(pageId, addon));

  @override
  Future<void> setKey(
    String pageId,
    String addon,
    String key,
    String jsonValue,
  ) async {
    writes.add('$pageId/$addon/$key');
    final page = _areas.putIfAbsent(pageId, () => {});
    page.putIfAbsent(addon, () => {})[key] = jsonValue;
  }

  @override
  Future<void> removeKey(String pageId, String addon, String key) async {
    writes.add('$pageId/$addon/-$key');
    _areas[pageId]?[addon]?.remove(key);
  }

  @override
  Future<String?> titleOf(String pageId) async => _titles[pageId];
}

/// A hash over the parts the engine hashes (runner, tools, steps). The real
/// hash is the engine's; this one only has to change when those parts change.
String fakeHash(ScheduleDef def) => 'fake:${jsonEncode({
  'runner': def.runner,
  'tools': def.toJson()['tools'],
  'steps': [for (final s in def.steps) s.toJson()],
})}';

/// A runner and engine that run nothing. It checks a definition, renders its
/// first title (with stand-in tokens), and keeps the acceptance and pause
/// state per page, as the engine would.
class FakeScheduleBackend extends ScheduleBackend {
  FakeScheduleBackend({
    this.nodes = const [],
    Map<String, ScheduleLiveState>? states,
    DateTime? now,
    this.error,
    this.stateError,
  }) : states = states ?? {},
       now = now ?? DateTime(2026, 10, 6, 11, 20);

  List<ScheduleNode> nodes;

  /// The live state per page id. Accept and pause update it.
  final Map<String, ScheduleLiveState> states;

  /// The clock previews start from.
  final DateTime now;

  /// When set, every call fails with this error.
  Object? error;

  /// When set, only [state] fails with this error (a runner that is not
  /// reachable).
  Object? stateError;

  /// Calls in order, as `name:args`, for assertions.
  final List<String> calls = [];

  @override
  Future<SchedulePreview> preview(String scheduleJson) async {
    _check(null);
    calls.add('preview');
    final json = decodeObject(scheduleJson) ?? const <String, Object?>{};
    final def = ScheduleDef.fromJson(json, tz: 'UTC');
    final first = def.firstStep;
    if (first == null) {
      return const SchedulePreview(
        ok: false,
        reason: 'Add a step, such as a note from a template.',
      );
    }
    final runs = nextRunsOf(def.trigger, now);
    final anchor = runs.isEmpty ? now : runs.first;
    return SchedulePreview(
      ok: true,
      currentHash: fakeHash(def),
      nextRuns: runs,
      renderedTitle: _stubTitle(first.title ?? '', anchor),
    );
  }

  @override
  Future<ScheduleLiveState> state(String pageId, String runnerId) async {
    _check(stateError);
    calls.add('state:$pageId:$runnerId');
    return states[pageId] ??
        const ScheduleLiveState(
          accepted: false,
          hashMatches: false,
          paused: false,
          runnerState: 'needs_acceptance',
        );
  }

  @override
  Future<ScheduleLiveState> accept(
    String pageId,
    String runnerId,
    String hash,
  ) async {
    _check(null);
    calls.add('accept:$pageId:$runnerId:$hash');
    final old = states[pageId];
    final accepted = ScheduleLiveState(
      accepted: true,
      hashMatches: true,
      paused: old?.paused ?? false,
      runnerState: (old?.paused ?? false) ? 'paused' : 'ok',
      currentHash: hash,
      lastRuns: old?.lastRuns ?? const [],
      nextRun: old?.nextRun,
    );
    states[pageId] = accepted;
    return accepted;
  }

  @override
  Future<ScheduleLiveState> pause(
    String pageId,
    String runnerId,
    bool paused,
  ) async {
    _check(null);
    calls.add('pause:$pageId:$runnerId:$paused');
    final old = states[pageId];
    final next = ScheduleLiveState(
      accepted: old?.accepted ?? false,
      hashMatches: old?.hashMatches ?? false,
      paused: paused,
      runnerState: paused ? 'paused' : 'ok',
      currentHash: old?.currentHash,
      lastRuns: old?.lastRuns ?? const [],
      nextRun: old?.nextRun,
    );
    states[pageId] = next;
    return next;
  }

  @override
  Future<ScheduleRunRecord> runNow(String pageId, String runnerId) async {
    _check(null);
    calls.add('runNow:$pageId:$runnerId');
    return ScheduleRunRecord(
      occurrence: now,
      at: now,
      result: 'created',
      page: 'created-now',
    );
  }

  @override
  Future<List<ScheduleNode>> listNodes() async {
    _check(null);
    calls.add('listNodes');
    return nodes;
  }

  void _check(Object? extra) {
    final e = extra ?? error;
    if (e != null) throw e;
  }
}

/// A stand-in title render: `{{year}}`, `{{month}}`, `{{day}}` and `{{week}}`
/// take the anchor date's values. The week is a rough count, for layout only.
String _stubTitle(String title, DateTime anchor) {
  final dayOfYear = anchor.difference(DateTime(anchor.year)).inDays + 1;
  return title
      .replaceAll('{{year}}', '${anchor.year}')
      .replaceAll('{{month}}', anchor.month.toString().padLeft(2, '0'))
      .replaceAll('{{day}}', anchor.day.toString().padLeft(2, '0'))
      .replaceAll('{{week}}', '${(dayOfYear / 7).ceil()}');
}

/// The next [count] occurrences of [trigger] at or after [from]. Interval
/// and end dates are not modelled: the fake is for layout and checks, not
/// for the engine's calendar rules.
List<DateTime> nextRunsOf(
  ScheduleTrigger trigger,
  DateTime from, {
  int count = 3,
}) {
  final start = DateTime.tryParse(trigger.at);
  final hm = trigger.time.isEmpty ? '00:00' : trigger.time;
  final hour = int.parse(hm.substring(0, 2));
  final minute = int.parse(hm.substring(3, 5));
  final rule = trigger.rule;
  if (rule == null || !rule.supported) {
    if (start == null) return const [];
    final once = DateTime(start.year, start.month, start.day, hour, minute);
    return once.isAfter(from) ? [once] : const [];
  }
  final out = <DateTime>[];
  final firstDay = DateTime(from.year, from.month, from.day);
  for (var d = 0; d < 800 && out.length < count; d++) {
    final day = firstDay.add(Duration(days: d));
    if (start != null &&
        day.isBefore(DateTime(start.year, start.month, start.day))) {
      continue;
    }
    final matches = switch (rule.freq) {
      ScheduleFreq.daily => true,
      ScheduleFreq.weekly => rule.byDay.contains(
        weekdayCodes[day.weekday - 1],
      ),
      ScheduleFreq.monthly => day.day == (rule.monthDay ?? -1),
      ScheduleFreq.once => false,
    };
    if (!matches) continue;
    final candidate = DateTime(day.year, day.month, day.day, hour, minute);
    if (candidate.isAfter(from)) out.add(candidate);
  }
  return out;
}
