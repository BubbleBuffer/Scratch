// Small helpers over the bridge's value types: building values, filters and
// sorts, reading a row's properties, and dates (whole-day dates are
// midnight UTC in the core; dates with a time are instants).

import '../../core/models.dart';

/// Built-in property keys the core understands in filters and sorts.
abstract final class BuiltinKeys {
  static const title = r'$title';
  static const created = r'$created';
  static const updated = r'$updated';
  static const cover = r'$cover';
}

abstract final class Values {
  static const empty = PropValue(kind: ValueKind.empty, text: '', number: 0, startMs: 0, hasTime: false, checked: false, ids: []);

  static PropValue text(String s) => s.isEmpty ? empty : PropValue(kind: ValueKind.text, text: s, number: 0, startMs: 0, hasTime: false, checked: false, ids: const []);
  static PropValue url(String s) => s.isEmpty ? empty : PropValue(kind: ValueKind.url, text: s, number: 0, startMs: 0, hasTime: false, checked: false, ids: const []);
  static PropValue number(double n) => PropValue(kind: ValueKind.number, text: '', number: n, startMs: 0, hasTime: false, checked: false, ids: const []);
  static PropValue checkbox(bool b) => PropValue(kind: ValueKind.checkbox, text: '', number: 0, startMs: 0, hasTime: false, checked: b, ids: const []);
  static PropValue select(String id) => id.isEmpty ? empty : PropValue(kind: ValueKind.select, text: id, number: 0, startMs: 0, hasTime: false, checked: false, ids: const []);
  static PropValue status(String id) => id.isEmpty ? empty : PropValue(kind: ValueKind.status, text: id, number: 0, startMs: 0, hasTime: false, checked: false, ids: const []);
  static PropValue multi(List<String> ids) => ids.isEmpty ? empty : PropValue(kind: ValueKind.multiSelect, text: '', number: 0, startMs: 0, hasTime: false, checked: false, ids: ids);
  static PropValue files(List<String> ids) => ids.isEmpty ? empty : PropValue(kind: ValueKind.files, text: '', number: 0, startMs: 0, hasTime: false, checked: false, ids: ids);
  static PropValue relation(List<String> ids) => ids.isEmpty ? empty : PropValue(kind: ValueKind.relation, text: '', number: 0, startMs: 0, hasTime: false, checked: false, ids: ids);
  static PropValue date(int start, {int? end, bool hasTime = false}) =>
      PropValue(kind: ValueKind.date, text: '', number: 0, startMs: start, endMs: end, hasTime: hasTime, checked: false, ids: const []);

  /// The value kind a property of [kind] stores.
  static ValueKind storedKind(PropKind kind) => switch (kind) {
        PropKind.text || PropKind.email || PropKind.phone => ValueKind.text,
        PropKind.url => ValueKind.url,
        PropKind.number => ValueKind.number,
        PropKind.date || PropKind.createdTime || PropKind.updatedTime => ValueKind.date,
        PropKind.checkbox => ValueKind.checkbox,
        PropKind.select => ValueKind.select,
        PropKind.multiSelect => ValueKind.multiSelect,
        PropKind.status => ValueKind.status,
        PropKind.relation => ValueKind.relation,
        PropKind.files => ValueKind.files,
        PropKind.formula || PropKind.rollup => ValueKind.text,
      };

  /// Text of a text-like value in the kind [kind] stores ([s] may be empty).
  static PropValue ofText(PropKind kind, String s) => kind == PropKind.url ? url(s) : text(s);
}

extension PropValueX on PropValue {
  bool get isEmpty => switch (kind) {
        ValueKind.empty => true,
        ValueKind.text || ValueKind.url || ValueKind.select || ValueKind.status => text.isEmpty,
        ValueKind.multiSelect || ValueKind.relation || ValueKind.files => ids.isEmpty,
        _ => false,
      };

  /// Option ids of a select, status or multi-select value.
  List<String> get optionIds => switch (kind) {
        ValueKind.select || ValueKind.status => text.isEmpty ? const [] : [text],
        ValueKind.multiSelect => ids,
        _ => const [],
      };
}

extension PageRowX on PageRow {
  PropValue value(String key) {
    for (final p in properties) {
      if (p.key == key) return p.value;
    }
    return Values.empty;
  }

  /// The value of [def], computed for created and edited time.
  PropValue valueOf(PropDef def) => switch (def.kind) {
        PropKind.createdTime => Values.date(createdMs, hasTime: true),
        PropKind.updatedTime => Values.date(updatedMs, hasTime: true),
        _ => value(def.key),
      };

  PageRow copyWith({String? title, String? icon, List<Property>? properties, int? updatedMs}) => PageRow(
        id: id,
        title: title ?? this.title,
        parent: parent,
        updatedMs: updatedMs ?? this.updatedMs,
        createdMs: createdMs,
        childCount: childCount,
        icon: icon ?? this.icon,
        pageType: pageType,
        favorite: favorite,
        properties: properties ?? this.properties,
      );

  /// This row with [key] set to [v] (removed when empty).
  PageRow withValue(String key, PropValue v) => copyWith(properties: [
        for (final p in properties)
          if (p.key != key) p,
        if (!v.isEmpty) Property(key: key, value: v),
      ]);
}

extension PropDefX on PropDef {
  SelectOption? option(String id) {
    for (final o in options) {
      if (o.id == id) return o;
    }
    return null;
  }

  PropDef copyWith({String? name, PropKind? kind, List<SelectOption>? options, String? config}) => PropDef(
        key: key,
        name: name ?? this.name,
        kind: kind ?? this.kind,
        options: options ?? this.options,
        config: config ?? this.config,
      );

  /// Values come from the core, not the user (times, formulas, rollups).
  bool get computed =>
      kind == PropKind.createdTime || kind == PropKind.updatedTime || kind == PropKind.formula || kind == PropKind.rollup;

  /// Created or last edited time.
  bool get isTime => kind == PropKind.createdTime || kind == PropKind.updatedTime;
  bool get hasOptions => kind == PropKind.select || kind == PropKind.multiSelect || kind == PropKind.status;
}

extension ViewDefX on ViewDef {
  ViewDef copyWith({
    String? id,
    String? name,
    ViewKind? kind,
    Filter? filter,
    bool clearFilter = false,
    List<SortSpec>? sorts,
    String? groupBy,
    bool clearGroup = false,
    String? dateProp,
    List<String>? visibleProps,
    String? cardCover,
    String? config,
  }) =>
      ViewDef(
        id: id ?? this.id,
        name: name ?? this.name,
        kind: kind ?? this.kind,
        filter: clearFilter ? null : (filter ?? this.filter),
        sorts: sorts ?? this.sorts,
        groupBy: clearGroup ? null : (groupBy ?? this.groupBy),
        dateProp: dateProp ?? this.dateProp,
        visibleProps: visibleProps ?? this.visibleProps,
        cardCover: cardCover ?? this.cardCover,
        config: config ?? this.config,
      );
}

// ---------------------------------------------------------------- filters

abstract final class Filters {
  static Filter cond(String prop, FilterOp op, [PropValue value = Values.empty]) =>
      Filter(kind: FilterKind.cond, children: const [], cond: Condition(prop: prop, op: op, value: value));
  static Filter and(List<Filter> children) => Filter(kind: FilterKind.and, children: children);
  static Filter or(List<Filter> children) => Filter(kind: FilterKind.or, children: children);

  /// [a] and [b] (either may be null).
  static Filter? both(Filter? a, Filter? b) {
    if (a == null) return b;
    if (b == null) return a;
    return and([a, b]);
  }

  /// [f] without rules that are still being written (an op that needs a
  /// value and has none) and empty groups; null when nothing is left.
  static Filter? prune(Filter? f) {
    if (f == null) return null;
    if (f.kind == FilterKind.cond) {
      final c = f.cond;
      if (c == null) return null;
      const noValue = {FilterOp.isEmpty, FilterOp.isNotEmpty, FilterOp.checked, FilterOp.unchecked};
      if (!noValue.contains(c.op) && c.value.kind == ValueKind.empty) return null;
      return f;
    }
    final kids = [for (final k in f.children) ?prune(k)];
    if (kids.isEmpty) return null;
    if (kids.length == 1) return kids.first;
    return Filter(kind: f.kind, children: kids);
  }

  /// The top-level rules of a simple filter (a condition, or an And of
  /// conditions); null when [f] is an advanced filter (Or, or groups).
  static List<Condition>? simpleRules(Filter? f) {
    if (f == null) return const [];
    if (f.kind == FilterKind.cond) return [if (f.cond != null) f.cond!];
    if (f.kind == FilterKind.and && f.children.every((c) => c.kind == FilterKind.cond)) {
      return [for (final c in f.children) if (c.cond != null) c.cond!];
    }
    return null;
  }

  /// A simple filter of [rules] (null when there are none).
  static Filter? ofRules(List<Condition> rules) {
    if (rules.isEmpty) return null;
    final conds = [for (final r in rules) Filter(kind: FilterKind.cond, children: const [], cond: r)];
    return conds.length == 1 ? conds.first : and(conds);
  }

  /// Number of conditions in [f].
  static int count(Filter? f) {
    if (f == null) return 0;
    if (f.kind == FilterKind.cond) return 1;
    return f.children.fold(0, (n, c) => n + count(c));
  }
}

// ------------------------------------------------------------------ dates

const _months = [
  'January', 'February', 'March', 'April', 'May', 'June', //
  'July', 'August', 'September', 'October', 'November', 'December',
];
const monthsShort = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const weekdaysShort = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

String monthName(int m) => _months[m - 1];

/// A calendar day (no time zone): the unit of calendars and timelines.
class Day implements Comparable<Day> {
  const Day(this.year, this.month, this.day);
  factory Day.of(DateTime t) => Day(t.year, t.month, t.day);

  /// The day of a value's [ms]: whole-day dates in UTC, times locally.
  factory Day.ofMs(int ms, bool hasTime) => Day.of(DateTime.fromMillisecondsSinceEpoch(ms, isUtc: !hasTime));

  final int year, month, day;

  DateTime get utc => DateTime.utc(year, month, day);
  int get ms => utc.millisecondsSinceEpoch;

  /// Days since the epoch.
  int get index => ms ~/ 86400000;
  static Day fromIndex(int i) => Day.of(DateTime.fromMillisecondsSinceEpoch(i * 86400000, isUtc: true));

  Day add(int days) => Day.of(utc.add(Duration(days: days)));
  int weekday0Sunday() => utc.weekday % 7;
  int operator -(Day o) => index - o.index;

  @override
  bool operator ==(Object other) => other is Day && other.year == year && other.month == month && other.day == day;
  @override
  int get hashCode => Object.hash(year, month, day);
  @override
  int compareTo(Day o) => index.compareTo(o.index);
  @override
  String toString() => '$year-$month-$day';
}

String formatDay(Day d) => '${monthName(d.month)} ${d.day}, ${d.year}';

String formatTime(DateTime t) {
  final h = t.hour % 12 == 0 ? 12 : t.hour % 12;
  return '$h:${t.minute.toString().padLeft(2, '0')} ${t.hour < 12 ? 'AM' : 'PM'}';
}

/// "July 23, 2020", "July 22, 2020 2:13 PM", ranges with " → ".
String formatDate(PropValue v, {String format = 'full'}) {
  if (v.kind != ValueKind.date) return '';
  String one(int ms) {
    final t = DateTime.fromMillisecondsSinceEpoch(ms, isUtc: !v.hasTime);
    final d = Day.of(t);
    final day = switch (format) {
      'short' => '${d.month}/${d.day}/${d.year}',
      'iso' => '${d.year}-${d.month.toString().padLeft(2, '0')}-${d.day.toString().padLeft(2, '0')}',
      'md' => '${monthsShort[d.month - 1]} ${d.day}',
      _ => formatDay(d),
    };
    return v.hasTime ? '$day ${formatTime(t)}' : day;
  }

  final end = v.endMs;
  return end == null ? one(v.startMs) : '${one(v.startMs)} → ${one(end)}';
}
