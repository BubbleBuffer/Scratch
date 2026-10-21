// The property registry. Each property type (text, number, date, select,
// ...) is one file in lib/addons/database/properties/ that registers a [PropertyType]: how a
// value shows (table cell, card, list, page row), how it is edited (a
// popover on desktop, a sheet on mobile), its filter operators and its
// footer calculations. Database views and page property rows only go
// through this registry.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../ui/database/config.dart';
import '../ui/database/db_icons.dart';

/// Where a value is drawn.
enum CellMode {
  /// A table cell (14px, one line unless wrapped).
  table,

  /// A board or gallery card (12px, small pills).
  card,

  /// A list row, right-aligned.
  list,

  /// A page's property row (shows "Empty" when empty).
  page,
}

/// Arguments of a value's display.
class CellArgs {
  const CellArgs({required this.context, required this.def, required this.value, required this.mode, this.wrap = false});
  final BuildContext context;
  final PropDef def;
  final PropValue value;
  final CellMode mode;

  /// Wrap long text onto more lines.
  final bool wrap;
}

/// Arguments of a value's editor.
class EditArgs {
  const EditArgs({
    required this.context,
    required this.def,
    required this.value,
    required this.anchor,
    required this.onChanged,
    required this.saveDef,
    required this.collection,
  });

  /// A context under the app's navigator (for the popover or sheet).
  final BuildContext context;
  final PropDef def;
  final PropValue value;

  /// The cell's global rectangle (the editor opens over or under it).
  final Rect anchor;

  /// Saves a new value (called as the user edits).
  final ValueChanged<PropValue> onChanged;

  /// Saves a changed definition (new or edited options); returns it as the
  /// core saved it.
  final Future<PropDef> Function(PropDef def) saveDef;

  /// The row's database (relations default to it).
  final String collection;
}

/// Arguments of a filter rule's value input.
class FilterValueArgs {
  const FilterValueArgs({required this.context, required this.def, required this.op, required this.value, required this.onChanged});
  final BuildContext context;
  final PropDef def;
  final FilterOp op;
  final PropValue value;
  final ValueChanged<PropValue> onChanged;
}

class PropertyType {
  const PropertyType({
    required this.kind,
    required this.label,
    required this.icon,
    required this.format,
    required this.cell,
    this.advanced = false,
    this.filterOps = const [FilterOp.isEmpty, FilterOp.isNotEmpty],
    this.edit,
    this.tap,
    this.filterValue,
    this.calcs = const [],
    this.groupable = false,
    this.alignEnd = false,
  });

  final PropKind kind;
  final String label;
  final DbIco icon;

  /// Listed under "Advanced" in the type picker.
  final bool advanced;

  /// Operators offered by the filter editor, in order (the first is the
  /// default).
  final List<FilterOp> filterOps;

  /// The value as plain text (search, calendars, fallback).
  final String Function(PropValue v, PropDef def) format;

  /// The value's display.
  final Widget Function(CellArgs a) cell;

  /// Opens the editor (null: read-only, like created time).
  final Future<void> Function(EditArgs a)? edit;

  /// A tap changes the value directly (checkbox); returns the new value.
  final PropValue Function(PropValue v)? tap;

  /// The value input of a filter rule (null: the op needs no value).
  final Widget Function(FilterValueArgs a)? filterValue;

  /// Footer calculations besides the counts every type has.
  final List<Calc> calcs;

  /// Can group a board.
  final bool groupable;

  /// Right-aligned in table cells (numbers).
  final bool alignEnd;
}

class PropertyRegistry {
  final _types = <PropKind, PropertyType>{};

  /// Adds or replaces a type; the type picker lists them in this order.
  void register(PropertyType t) => _types[t.kind] = t;

  PropertyType? of(PropKind kind) => _types[kind];

  Iterable<PropertyType> get all => _types.values;

  /// [v] as text, through the type's formatter.
  String formatValue(PropValue v, PropDef def) {
    final f = _types[def.kind]?.format;
    if (f != null) return f(v, def);
    return switch (v.kind) {
      ValueKind.text || ValueKind.url => v.text,
      ValueKind.number => '${v.number}',
      _ => '',
    };
  }
}

final propertyRegistry = PropertyRegistry();

/// Filter operator names as the UI shows them.
String filterOpLabel(FilterOp op, PropKind? kind) {
  final num = kind == PropKind.number;
  final date = kind == PropKind.date || kind == PropKind.createdTime || kind == PropKind.updatedTime;
  return switch (op) {
    FilterOp.is_ => num ? '=' : 'Is',
    FilterOp.isNot => num ? '≠' : 'Is not',
    FilterOp.contains => 'Contains',
    FilterOp.notContains => 'Does not contain',
    FilterOp.startsWith => 'Starts with',
    FilterOp.endsWith => 'Ends with',
    FilterOp.isEmpty => 'Is empty',
    FilterOp.isNotEmpty => 'Is not empty',
    FilterOp.greater => '>',
    FilterOp.greaterOrEqual => '≥',
    FilterOp.less => '<',
    FilterOp.lessOrEqual => '≤',
    FilterOp.before => date ? 'Is before' : 'Before',
    FilterOp.after => date ? 'Is after' : 'After',
    FilterOp.onOrBefore => 'Is on or before',
    FilterOp.onOrAfter => 'Is on or after',
    FilterOp.withinRange => 'Is within',
    FilterOp.checked => 'Is checked',
    FilterOp.unchecked => 'Is unchecked',
  };
}

/// Ops that take no value.
bool opNeedsValue(FilterOp op) =>
    op != FilterOp.isEmpty && op != FilterOp.isNotEmpty && op != FilterOp.checked && op != FilterOp.unchecked;
