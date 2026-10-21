// Interface settings of properties and views. The core stores them as an
// opaque JSON string (`PropDef.config`, `ViewDef.config`) and syncs them;
// only the app reads them. Unknown keys are kept when a setting changes.

import 'dart:convert';

import '../../core/models.dart';

Map<String, dynamic> _parse(String s) {
  if (s.isEmpty) return {};
  try {
    final v = jsonDecode(s);
    return v is Map<String, dynamic> ? v : {};
  } catch (_) {
    return {};
  }
}

/// Status groups, in display order.
enum StatusGroup {
  todo('To-do'),
  inProgress('In progress'),
  complete('Complete');

  const StatusGroup(this.label);
  final String label;
}

class PropConfig {
  PropConfig(String json) : _m = _parse(json);
  PropConfig.empty() : _m = {};
  final Map<String, dynamic> _m;

  String encode() => _m.isEmpty ? '' : jsonEncode(_m);

  /// Number: number, commas, percent, dollar, euro, pound, yen.
  String get numberFormat => (_m['format'] as String?) ?? 'number';
  set numberFormat(String v) => _m['format'] = v;

  /// Date: full, short, iso, md, relative.
  String get dateFormat => (_m['dateFormat'] as String?) ?? 'full';
  set dateFormat(String v) => _m['dateFormat'] = v;

  /// Relation: the related collection.
  String? get relationTo => _m['relation'] as String?;
  set relationTo(String? v) => v == null ? _m.remove('relation') : _m['relation'] = v;

  /// Formula: the expression (read by the core).
  String get formula => (_m['formula'] as String?) ?? '';
  set formula(String v) => v.isEmpty ? _m.remove('formula') : _m['formula'] = v;

  /// Rollup: the relation property key, the related property key (or
  /// `$title`) and the aggregation (read by the core).
  String? get rollupRelation => _m['rollupRelation'] as String?;
  set rollupRelation(String? v) => v == null ? _m.remove('rollupRelation') : _m['rollupRelation'] = v;
  String? get rollupProperty => _m['rollupProperty'] as String?;
  set rollupProperty(String? v) => v == null ? _m.remove('rollupProperty') : _m['rollupProperty'] = v;
  String get rollupFunction => (_m['rollupFunction'] as String?) ?? 'show_original';
  set rollupFunction(String v) => _m['rollupFunction'] = v;

  /// Status: option id -> group. Options without an entry are placed by
  /// position (see [statusGroupOf]).
  Map<String, StatusGroup> get statusGroups {
    final raw = _m['groups'];
    if (raw is! Map) return {};
    return {
      for (final e in raw.entries)
        if (e.value is String) '${e.key}': StatusGroup.values.firstWhere((g) => g.name == e.value, orElse: () => StatusGroup.todo),
    };
  }

  set statusGroups(Map<String, StatusGroup> g) => _m['groups'] = {for (final e in g.entries) e.key: e.value.name};

  /// How the property shows at the top of a row's page.
  PropVisibility get visibility =>
      PropVisibility.values.firstWhere((v) => v.name == _m['visibility'], orElse: () => PropVisibility.always);
  set visibility(PropVisibility v) => v == PropVisibility.always ? _m.remove('visibility') : _m['visibility'] = v.name;
}

/// The "Property visibility" setting on a row's page.
enum PropVisibility {
  always('Always show'),
  hideEmpty('Hide when empty'),
  hidden('Always hide');

  const PropVisibility(this.label);
  final String label;
}

/// The group of status option [id] in [def]: the configured one, else the
/// first option is To-do, the last Complete and the rest In progress.
StatusGroup statusGroupOf(PropDef def, String id) {
  final g = PropConfig(def.config).statusGroups[id];
  if (g != null) return g;
  final i = def.options.indexWhere((o) => o.id == id);
  if (i <= 0) return StatusGroup.todo;
  if (i == def.options.length - 1) return StatusGroup.complete;
  return StatusGroup.inProgress;
}

/// Where rows open.
enum OpenIn { sidePeek, centerPeek, fullPage }

/// Calculations of the table footer.
enum Calc {
  none('None', 'Calculate'),
  countAll('Count all', 'Count'),
  countValues('Count values', 'Values'),
  countEmpty('Count empty', 'Empty'),
  countNotEmpty('Count not empty', 'Not empty'),
  percentEmpty('Percent empty', 'Empty'),
  percentNotEmpty('Percent not empty', 'Not empty'),
  sum('Sum', 'Sum'),
  average('Average', 'Average'),
  median('Median', 'Median'),
  min('Min', 'Min'),
  max('Max', 'Max'),
  range('Range', 'Range'),
  earliest('Earliest date', 'Earliest'),
  latest('Latest date', 'Latest'),
  checked('Checked', 'Checked'),
  unchecked('Unchecked', 'Unchecked'),
  percentChecked('Percent checked', 'Checked');

  const Calc(this.label, this.short);
  final String label, short;
}

enum CardSize { small, medium, large }

enum TimelineZoom { week, month, quarter }

class ViewConfig {
  ViewConfig(String json) : _m = _parse(json);
  final Map<String, dynamic> _m;

  String encode() => _m.isEmpty ? '' : jsonEncode(_m);

  T _enum<T extends Enum>(String key, List<T> values, T fallback) {
    final v = _m[key];
    return values.firstWhere((e) => e.name == v, orElse: () => fallback);
  }

  /// Column widths by property key (table).
  Map<String, double> get widths {
    final raw = _m['widths'];
    if (raw is! Map) return {};
    return {for (final e in raw.entries) if (e.value is num) '${e.key}': (e.value as num).toDouble()};
  }

  set widths(Map<String, double> w) => _m['widths'] = w;

  bool get wrap => _m['wrap'] == true;
  set wrap(bool v) => _m['wrap'] = v;

  /// The filter is edited as an advanced filter even when it is a plain
  /// list of rules ("Add to advanced filter").
  bool get advancedFilter => _m['advanced'] == true;
  set advancedFilter(bool v) => v ? _m['advanced'] = true : _m.remove('advanced');

  /// Wrapped columns (table), by key.
  Set<String> get wrapped => {...?(_m['wrapped'] as List?)?.whereType<String>()};
  set wrapped(Set<String> s) => _m['wrapped'] = s.toList();

  bool get verticalLines => _m['vlines'] != false;
  set verticalLines(bool v) => _m['vlines'] = v;

  bool get showIcon => _m['icons'] != false;
  set showIcon(bool v) => _m['icons'] = v;

  /// Columns frozen after the title (table), 0 = only the title.
  int get frozen => (_m['frozen'] as num?)?.toInt() ?? 0;
  set frozen(int v) => _m['frozen'] = v;

  OpenIn openIn(ViewKind kind) => _enum(
      'openIn', OpenIn.values, kind == ViewKind.gallery || kind == ViewKind.calendar ? OpenIn.centerPeek : OpenIn.sidePeek);
  set openInValue(OpenIn v) => _m['openIn'] = v.name;

  /// Footer calculations by property key (table).
  Map<String, Calc> get calcs {
    final raw = _m['calcs'];
    if (raw is! Map) return {};
    return {
      for (final e in raw.entries)
        '${e.key}': Calc.values.firstWhere((c) => c.name == e.value, orElse: () => Calc.none),
    }..removeWhere((k, v) => v == Calc.none);
  }

  set calcs(Map<String, Calc> c) => _m['calcs'] = {for (final e in c.entries) e.key: e.value.name};

  CardSize get cardSize => _enum('cardSize', CardSize.values, CardSize.medium);
  set cardSize(CardSize v) => _m['cardSize'] = v.name;

  /// Gallery/board preview: none, cover, content.
  String get preview => (_m['preview'] as String?) ?? 'none';

  /// The preview of a [kind] view (a gallery shows the cover by default).
  String previewFor(ViewKind kind) => (_m['preview'] as String?) ?? (kind == ViewKind.gallery ? 'cover' : 'none');
  set preview(String v) => _m['preview'] = v;

  /// Hidden board groups, by group key.
  Set<String> get hiddenGroups => {...?(_m['hidden'] as List?)?.whereType<String>()};
  set hiddenGroups(Set<String> s) => _m['hidden'] = s.toList();

  bool get hideEmptyGroups => _m['hideEmpty'] == true;
  set hideEmptyGroups(bool v) => _m['hideEmpty'] = v;

  bool get colorColumns => _m['colorColumns'] != false;
  set colorColumns(bool v) => _m['colorColumns'] = v;

  bool get mondayStart => _m['monday'] == true;
  set mondayStart(bool v) => _m['monday'] = v;

  TimelineZoom get zoom => _enum('zoom', TimelineZoom.values, TimelineZoom.month);
  set zoom(TimelineZoom v) => _m['zoom'] = v.name;

  /// Timeline: the date property of the bar's end, when the start and end
  /// are separate properties (null: the start property's range).
  String? get endProp => _m['endProp'] as String?;

  /// Timeline: show the table panel.
  bool get showTable => _m['table'] != false;
  set showTable(bool v) => _m['table'] = v;
}

extension ViewDefConfig on ViewDef {
  ViewConfig get cfg => ViewConfig(config);
}

extension PropDefConfig on PropDef {
  PropConfig get cfg => PropConfig(config);
}
