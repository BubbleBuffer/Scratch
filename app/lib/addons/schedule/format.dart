// The schedule note's stored shape (format v1) and its pure rules. Nothing
// here touches the vault or the engine, so the parse and edit rules can be
// tested on their own. Unknown keys are kept at every level and written back
// unchanged, so a newer client's fields survive an edit here. Hashes, titles
// and next runs come from the engine, never from this file.

import 'dart:convert';

/// The page type of a schedule note.
const scheduleType = 'schedule';

/// The add-on id: the key of the definition area in the page's type data.
const scheduleAddonId = 'schedule';

/// The type-data key of the runner's status (written by the engine).
const scheduleStatusKey = 'schedule_status';

/// The highest format version this client understands.
const scheduleFormatVersion = 1;

/// The only step kind phase 1 runs.
const scheduleNoteStepKind = 'note.from_template';

/// Weekday codes in week order (RRULE BYDAY values).
const weekdayCodes = ['MO', 'TU', 'WE', 'TH', 'FR', 'SA', 'SU'];

/// Weekday names in week order, as shown to the user.
const weekdayNames = [
  'Monday',
  'Tuesday',
  'Wednesday',
  'Thursday',
  'Friday',
  'Saturday',
  'Sunday',
];

/// Frequencies the editor offers. Anything else in a stored rule is kept but
/// reported as unsupported.
enum ScheduleFreq { once, daily, weekly, monthly }

/// The `missed` policies of a trigger.
const missedLatest = 'latest';
const missedSkip = 'skip';

/// Decodes a JSON object, or null when [json] is empty or not an object.
Map<String, Object?>? decodeObject(String? json) {
  if (json == null || json.trim().isEmpty) return null;
  try {
    final value = jsonDecode(json);
    return value is Map<String, Object?> ? value : null;
  } on FormatException {
    return null;
  }
}

/// An RRULE in the phase 1 subset. Parts keep their order and any part the
/// editor does not change, such as INTERVAL or UNTIL.
class ScheduleRule {
  ScheduleRule._(this._parts);

  /// Parses [raw]; null for an empty rule (a one-off).
  static ScheduleRule? parse(String? raw) {
    if (raw == null || raw.trim().isEmpty) return null;
    final parts = <String, String>{};
    for (final piece in raw.split(';')) {
      final eq = piece.indexOf('=');
      if (eq <= 0) continue;
      parts[piece.substring(0, eq).trim().toUpperCase()] = piece
          .substring(eq + 1)
          .trim();
    }
    return ScheduleRule._(parts);
  }

  final Map<String, String> _parts;

  String? get _freqCode => _parts['FREQ'];

  /// True when the frequency is one the editor can show and change.
  bool get supported =>
      const {'DAILY', 'WEEKLY', 'MONTHLY'}.contains(_freqCode);

  ScheduleFreq get freq => switch (_freqCode) {
    'DAILY' => ScheduleFreq.daily,
    'WEEKLY' => ScheduleFreq.weekly,
    'MONTHLY' => ScheduleFreq.monthly,
    _ => ScheduleFreq.once,
  };

  /// The weekdays of a weekly rule, in week order.
  List<String> get byDay {
    final days = (_parts['BYDAY'] ?? '')
        .split(',')
        .map((d) => d.trim().toUpperCase())
        .where(weekdayCodes.contains)
        .toSet();
    return [
      for (final code in weekdayCodes)
        if (days.contains(code)) code,
    ];
  }

  /// The day of month of a monthly rule.
  int? get monthDay => int.tryParse(_parts['BYMONTHDAY'] ?? '');

  /// The interval between occurrences (1 when absent).
  int get interval => int.tryParse(_parts['INTERVAL'] ?? '') ?? 1;

  /// The rule as stored. Parts are joined in their stored order.
  String toText() => [for (final e in _parts.entries) '${e.key}=${e.value}']
      .join(';');

  /// The rule for [freq]. A switch keeps INTERVAL and UNTIL and drops the
  /// parts of the old frequency. [defaultDay] and [defaultMonthDay] fill in
  /// what the new frequency needs and the rule does not have yet.
  ScheduleRule withFreq(
    ScheduleFreq freq, {
    required String defaultDay,
    required int defaultMonthDay,
  }) {
    final parts = <String, String>{'FREQ': _code(freq)};
    switch (freq) {
      case ScheduleFreq.weekly:
        final days = byDay.isEmpty ? [defaultDay] : byDay;
        parts['BYDAY'] = days.join(',');
      case ScheduleFreq.monthly:
        parts['BYMONTHDAY'] = '${monthDay ?? defaultMonthDay}';
      case ScheduleFreq.daily || ScheduleFreq.once:
        break;
    }
    for (final key in const ['INTERVAL', 'UNTIL']) {
      final value = _parts[key];
      if (value != null) parts[key] = value;
    }
    return ScheduleRule._(parts);
  }

  /// The rule with its weekdays set to [days] (kept in week order).
  ScheduleRule withByDay(List<String> days) {
    final parts = Map<String, String>.of(_parts);
    final ordered = [
      for (final code in weekdayCodes)
        if (days.contains(code)) code,
    ];
    parts['BYDAY'] = ordered.join(',');
    return ScheduleRule._(parts);
  }

  /// The rule with its day of month set to [day] (1 to 31).
  ScheduleRule withMonthDay(int day) {
    final parts = Map<String, String>.of(_parts);
    parts['BYMONTHDAY'] = '$day';
    return ScheduleRule._(parts);
  }

  static String _code(ScheduleFreq freq) => switch (freq) {
    ScheduleFreq.daily => 'DAILY',
    ScheduleFreq.weekly => 'WEEKLY',
    ScheduleFreq.monthly => 'MONTHLY',
    ScheduleFreq.once => '',
  };
}

/// A trigger: when a schedule runs. [rrule] is null for a one-off.
class ScheduleTrigger {
  ScheduleTrigger({
    this.rrule,
    required this.at,
    required this.tz,
    this.missed = missedLatest,
    Map<String, Object?> extra = const {},
  }) : _extra = Map.of(extra);

  /// Reads a trigger object. A missing or damaged field gets its default, so
  /// the view can still show and repair the rest.
  factory ScheduleTrigger.fromJson(Object? json, {required String tz}) {
    final map = json is Map<String, Object?> ? json : <String, Object?>{};
    final rrule = map['rrule'];
    final at = map['at'];
    final missed = map['missed'];
    final zone = map['tz'];
    final extra = Map<String, Object?>.of(map)
      ..remove('rrule')
      ..remove('at')
      ..remove('tz')
      ..remove('missed');
    return ScheduleTrigger(
      rrule: rrule is String && rrule.trim().isNotEmpty ? rrule : null,
      at: at is String ? at : '',
      tz: zone is String && zone.isNotEmpty ? zone : tz,
      missed: missed == missedSkip ? missedSkip : missedLatest,
      extra: extra,
    );
  }

  final String? rrule;

  /// Local start, `YYYY-MM-DDTHH:MM`.
  final String at;
  final String tz;
  final String missed;
  final Map<String, Object?> _extra;

  ScheduleRule? get rule => ScheduleRule.parse(rrule);

  ScheduleTrigger copyWith({
    Object? rrule = _keep,
    String? at,
    String? tz,
    String? missed,
  }) => ScheduleTrigger(
    rrule: identical(rrule, _keep) ? this.rrule : rrule as String?,
    at: at ?? this.at,
    tz: tz ?? this.tz,
    missed: missed ?? this.missed,
    extra: _extra,
  );

  /// The date part of [at] (`YYYY-MM-DD`), or empty.
  String get date => at.length >= 10 ? at.substring(0, 10) : '';

  /// The time part of [at] (`HH:MM`), or empty.
  String get time {
    final t = at.indexOf('T');
    if (t < 0 || at.length < t + 6) return '';
    return at.substring(t + 1, t + 6);
  }

  Map<String, Object?> toJson() => {
    ..._extra,
    if (rrule != null) 'rrule': rrule,
    'at': at,
    'tz': tz,
    'missed': missed,
  };
}

const _keep = Object();

/// One step. Phase 1 runs `note.from_template` only; other kinds are kept and
/// shown read-only.
class ScheduleStep {
  ScheduleStep({
    required this.id,
    required this.kind,
    this.template,
    this.parent,
    this.title,
    Map<String, Object?> extra = const {},
  }) : _extra = Map.of(extra);

  factory ScheduleStep.fromJson(Map<String, Object?> json, int index) {
    final id = json['id'];
    final kind = json['do'];
    final template = json['template'];
    final parent = json['parent'];
    final title = json['title'];
    final extra = Map<String, Object?>.of(json)
      ..remove('id')
      ..remove('do')
      ..remove('template')
      ..remove('parent')
      ..remove('title');
    return ScheduleStep(
      id: id is String && id.isNotEmpty ? id : 'step-${index + 1}',
      kind: kind is String ? kind : '',
      template: template is String ? template : null,
      parent: parent is String ? parent : null,
      title: title is String ? title : null,
      extra: extra,
    );
  }

  final String id;
  final String kind;
  final String? template;
  final String? parent;
  final String? title;
  final Map<String, Object?> _extra;

  /// True for a step this client can run and edit.
  bool get isNoteFromTemplate => kind == scheduleNoteStepKind;

  ScheduleStep copyWith({
    Object? template = _keep,
    Object? parent = _keep,
    Object? title = _keep,
  }) => ScheduleStep(
    id: id,
    kind: kind,
    template: identical(template, _keep) ? this.template : template as String?,
    parent: identical(parent, _keep) ? this.parent : parent as String?,
    title: identical(title, _keep) ? this.title : title as String?,
    extra: _extra,
  );

  Map<String, Object?> toJson() => {
    ..._extra,
    'id': id,
    'do': kind,
    if (template != null) 'template': template,
    if (parent != null) 'parent': parent,
    if (title != null) 'title': title,
  };
}

/// The schedule definition: the `schedule` type-data area of a note.
class ScheduleDef {
  ScheduleDef({
    this.version = scheduleFormatVersion,
    required this.trigger,
    this.runner,
    this.tools = const [],
    this.toolSet,
    this.steps = const [],
    Map<String, Object?> extra = const {},
  }) : _extra = Map.of(extra);

  /// Reads an area. Anything malformed reads as its default.
  factory ScheduleDef.fromJson(
    Map<String, Object?> json, {
    required String tz,
  }) {
    final version = json['version'];
    final runner = json['runner'];
    final tools = json['tools'];
    final steps = json['steps'];
    final trigger = json['trigger'];
    final extra = Map<String, Object?>.of(json)
      ..remove('version')
      ..remove('trigger')
      ..remove('runner')
      ..remove('tools')
      ..remove('steps');
    return ScheduleDef(
      version: version is int ? version : scheduleFormatVersion,
      trigger: ScheduleTrigger.fromJson(trigger, tz: tz),
      runner: runner is String && runner.isNotEmpty ? runner : null,
      tools: tools is List ? tools.whereType<String>().toList() : const [],
      toolSet: tools is String && tools.isNotEmpty ? tools : null,
      steps: steps is List
          ? [
              for (final (i, s) in steps.indexed)
                if (s is Map<String, Object?>) ScheduleStep.fromJson(s, i),
            ]
          : const [],
      extra: extra,
    );
  }

  /// A new schedule with no step yet.
  factory ScheduleDef.blank({required String tz, required String at}) =>
      ScheduleDef(
        trigger: ScheduleTrigger(
          rrule: 'FREQ=WEEKLY;BYDAY=MO',
          at: at,
          tz: tz,
        ),
      );

  final int version;
  final ScheduleTrigger trigger;
  final String? runner;

  /// The tool names of a schedule written before tool sets; empty when
  /// [toolSet] is set.
  final List<String> tools;

  /// The built-in tool set the steps may use (`read`, `write` or
  /// `everything`), stored as the `tools` string. Null for a tool list.
  final String? toolSet;
  final List<ScheduleStep> steps;
  final Map<String, Object?> _extra;

  /// The first step, or null when there is none.
  ScheduleStep? get firstStep => steps.isEmpty ? null : steps.first;

  ScheduleDef copyWith({
    int? version,
    ScheduleTrigger? trigger,
    Object? runner = _keep,
    String? toolSet,
    List<ScheduleStep>? steps,
  }) => ScheduleDef(
    version: version ?? this.version,
    trigger: trigger ?? this.trigger,
    runner: identical(runner, _keep) ? this.runner : runner as String?,
    // Picking a set replaces a tool list.
    tools: toolSet == null ? tools : const [],
    toolSet: toolSet ?? this.toolSet,
    steps: steps ?? this.steps,
    extra: _extra,
  );

  /// The stored area. Keys this client does not know are written back
  /// unchanged.
  Map<String, Object?> toJson() => {
    ..._extra,
    'version': version,
    'trigger': trigger.toJson(),
    if (runner != null) 'runner': runner,
    if (toolSet != null)
      'tools': toolSet
    else if (tools.isNotEmpty)
      'tools': tools,
    if (steps.isNotEmpty) 'steps': [for (final s in steps) s.toJson()],
  };
}

/// Top-level keys of the area that differ between [before] and [after]. Only
/// these are written, so an edit never rewrites what the user did not touch.
/// A key that disappears maps to null.
Map<String, Object?> changedKeys(
  Map<String, Object?> before,
  Map<String, Object?> after,
) {
  final out = <String, Object?>{};
  for (final key in {...before.keys, ...after.keys}) {
    if (jsonEncode(_canonical(before[key])) !=
        jsonEncode(_canonical(after[key]))) {
      out[key] = after[key];
    }
  }
  return out;
}

/// Canonical JSON form (keys sorted), so [changedKeys] compares content, not
/// key order.
Object? _canonical(Object? value) {
  if (value is Map) {
    final keys = value.keys.map((k) => '$k').toList()..sort();
    return {for (final k in keys) k: _canonical(value[k])};
  }
  if (value is List) return [for (final v in value) _canonical(v)];
  return value;
}

/// Health of a schedule on its runner, as the card and the status band show
/// it. A refusal outranks the others. [unreachable] means the runner that
/// should answer (the hub, or this computer) did not; [elsewhere] means the
/// runner is another desktop, which this client never calls.
enum ScheduleHealth {
  ok,
  needsAcceptance,
  paused,
  runnerOff,
  refused,
  noRunner,
  unreachable,
  elsewhere,
}

/// The last run written back into `schedule_status`.
class ScheduleLastRun {
  const ScheduleLastRun({
    this.occurrence,
    this.at,
    required this.result,
    this.page,
    this.error,
  });

  factory ScheduleLastRun.fromJson(Map<String, Object?> json) =>
      ScheduleLastRun(
        occurrence: _date(json['occurrence']),
        at: _date(json['at']),
        result: json['result'] is String ? json['result'] as String : '',
        page: json['page'] is String ? json['page'] as String : null,
        error: json['error'] is String ? json['error'] as String : null,
      );

  final DateTime? occurrence, at;

  /// `created`, `skipped`, `failed` or another result the engine writes.
  final String result;
  final String? page, error;

  bool get failed => result == 'failed';
}

DateTime? _date(Object? v) => v is String ? DateTime.tryParse(v) : null;

/// The runner's status for a schedule (`schedule_status`).
class ScheduleStatus {
  const ScheduleStatus({
    this.lastRun,
    this.nextRun,
    this.runnerState,
    this.reason,
  });

  static const none = ScheduleStatus();

  factory ScheduleStatus.fromJson(Map<String, Object?>? json) {
    if (json == null) return none;
    final last = json['last_run'];
    final state = json['runner_state'];
    final reason = json['reason'];
    return ScheduleStatus(
      lastRun: last is Map<String, Object?>
          ? ScheduleLastRun.fromJson(last)
          : null,
      nextRun: _date(json['next_run']),
      runnerState: switch (state) {
        'ok' => ScheduleHealth.ok,
        'needs_acceptance' => ScheduleHealth.needsAcceptance,
        'paused' => ScheduleHealth.paused,
        'runner_off' => ScheduleHealth.runnerOff,
        'refused' => ScheduleHealth.refused,
        _ => null,
      },
      reason: reason is String && reason.isNotEmpty ? reason : null,
    );
  }

  final ScheduleLastRun? lastRun;
  final DateTime? nextRun;
  final ScheduleHealth? runnerState;
  final String? reason;
}

/// Validates `HH:MM` (24-hour). Returns the problem, or null when valid.
String? validateTime(String value) {
  final m = RegExp(r'^(\d{2}):(\d{2})$').firstMatch(value.trim());
  if (m == null) return 'Use 24-hour time, for example 08:00.';
  final h = int.parse(m.group(1)!);
  final min = int.parse(m.group(2)!);
  if (h > 23 || min > 59) return 'Use a time between 00:00 and 23:59.';
  return null;
}

/// Sets the `HH:MM` part of a local `YYYY-MM-DDTHH:MM` string.
String withTime(String at, String hhmm) {
  final date = at.length >= 10 ? at.substring(0, 10) : _today();
  return '${date}T${hhmm.trim()}';
}

/// Today's local date as `YYYY-MM-DD`.
String _today() {
  final now = DateTime.now();
  return dateString(now);
}

/// `YYYY-MM-DD` for a local date.
String dateString(DateTime d) =>
    '${d.year.toString().padLeft(4, '0')}-${d.month.toString().padLeft(2, '0')}-${d.day.toString().padLeft(2, '0')}';

/// `YYYY-MM-DDTHH:MM` for a local instant.
String localAtString(DateTime d) =>
    '${dateString(d)}T${d.hour.toString().padLeft(2, '0')}:${d.minute.toString().padLeft(2, '0')}';

/// The trigger in plain words, such as "Every Monday at 08:00".
String triggerInWords(ScheduleTrigger trigger) {
  final rule = trigger.rule;
  final time = trigger.time.isEmpty ? '' : ' at ${trigger.time}';
  if (rule == null || !rule.supported) {
    final day = trigger.date;
    return day.isEmpty ? 'Once' : 'Once on $day$time';
  }
  final every = rule.interval > 1 ? 'Every ${rule.interval}' : 'Every';
  return switch (rule.freq) {
    ScheduleFreq.daily =>
      rule.interval > 1
          ? 'Every ${rule.interval} days$time'
          : 'Every day$time',
    ScheduleFreq.weekly => () {
      final days = rule.byDay.map(_weekdayName).join(', ');
      final unit = rule.interval > 1 ? 'weeks' : 'week';
      final head = rule.interval > 1
          ? 'Every ${rule.interval} $unit'
          : 'Every ${days.isEmpty ? 'week' : days}';
      return rule.interval > 1 && days.isNotEmpty
          ? '$head on $days$time'
          : '$head$time';
    }(),
    ScheduleFreq.monthly =>
      '${rule.interval > 1 ? 'Every ${rule.interval} months' : 'Every month'}'
          ' on day ${rule.monthDay ?? '?'}$time',
    ScheduleFreq.once => every,
  };
}

String _weekdayName(String code) {
  final i = weekdayCodes.indexOf(code);
  return i < 0 ? code : weekdayNames[i];
}

/// The `[[id]]` form stored for a page reference (step parent and template).
String pageLink(String id) => '[[$id]]';

/// The page id inside a stored `[[id]]` reference, or null when empty.
String? pageIdOf(String? link) {
  if (link == null) return null;
  var v = link.trim();
  if (v.startsWith('[[') && v.endsWith(']]')) v = v.substring(2, v.length - 2);
  final bar = v.indexOf('|');
  if (bar >= 0) v = v.substring(0, bar);
  return v.trim().isEmpty ? null : v.trim();
}
