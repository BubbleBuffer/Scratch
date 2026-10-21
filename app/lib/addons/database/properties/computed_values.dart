// Displays shared by formula and rollup properties, whose values the core
// computes: a number, text, checkbox, date or an error message, whatever
// the expression gives.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/widgets.dart';
import '../../../registry/property_registry.dart';
import '../../../state/collection_store.dart';
import '../../../ui/ui.dart';

/// [v] as text; [suffix] follows numbers (a rollup's "%").
String computedText(PropValue v, PropDef d, {String suffix = ''}) => switch (v.kind) {
      ValueKind.number => '${formatNumberValue(v.number, PropConfig(d.config).numberFormat)}$suffix',
      ValueKind.checkbox => v.checked ? 'Checked' : 'Unchecked',
      ValueKind.date => dateText(v, d),
      ValueKind.text || ValueKind.url || ValueKind.error => v.text,
      _ => '',
    };

/// The cell of a computed value. Errors show in the cell, in the
/// destructive colour.
Widget computedCell(CellArgs a, {String suffix = ''}) {
  final v = a.value;
  switch (v.kind) {
    case ValueKind.error:
      final c = a.context.colors;
      return Text(v.text,
          key: const ValueKey('computed-error'),
          maxLines: a.wrap || a.mode == CellMode.page ? null : 1,
          overflow: a.wrap || a.mode == CellMode.page ? null : TextOverflow.ellipsis,
          style: cellStyle(a.context, a.mode, color: c.inlineCodeText));
    case ValueKind.checkbox:
      return checkboxCell(a);
    case ValueKind.number when a.mode == CellMode.table:
      return Align(
          alignment: Alignment.centerRight, child: textCell(a, computedText(v, a.def, suffix: suffix), align: TextAlign.right));
    default:
      return textCell(a, computedText(v, a.def, suffix: suffix));
  }
}

/// Filter operators of computed values: text and number comparisons (the
/// core compares numbers typed as text as numbers).
const computedFilterOps = [
  FilterOp.contains,
  FilterOp.notContains,
  FilterOp.is_,
  FilterOp.isNot,
  FilterOp.greater,
  FilterOp.less,
  FilterOp.greaterOrEqual,
  FilterOp.lessOrEqual,
  FilterOp.isEmpty,
  FilterOp.isNotEmpty,
];

Widget computedFilterValue(FilterValueArgs a) => MenuField(
      key: ValueKey('filter-value-${a.def.key}'),
      initial: switch (a.value.kind) {
        ValueKind.number => formatNumberValue(a.value.number, 'number'),
        ValueKind.empty => '',
        _ => a.value.text,
      },
      placeholder: 'Type a value…',
      autofocus: !a.context.mobile,
      onChanged: (s) => a.onChanged(s.isEmpty
          ? const PropValue(kind: ValueKind.empty, text: '', number: 0, startMs: 0, hasTime: false, checked: false, ids: [])
          : PropValue(kind: ValueKind.text, text: s, number: 0, startMs: 0, hasTime: false, checked: false, ids: const [])),
    );

/// A row of the property menu's edit page that opens a sub-panel.
Widget settingsRow(BuildContext context, String label,
        {required Key key, required String value, required void Function(Rect) onTap, required Widget leading}) =>
    Builder(
      builder: (b) => MenuItem(
        key: key,
        label: label,
        leading: leading,
        trailing: menuValue(context, value),
        onTap: () => onTap(globalRectOf(b)),
      ),
    );
