// Number: right-aligned, in the property's format (number, commas,
// percent, currencies); sum, average, median, min, max and range in the
// table footer.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/db_icons.dart';
import '../editors/text_editor.dart';
import '../../../ui/database/values.dart';
import '../../../registry/property_registry.dart';
import '../../../state/collection_store.dart';
import 'filter_values.dart';

/// Number formats in menu order: (id, label).
const numberFormats = [
  ('number', 'Number'),
  ('commas', 'Number with commas'),
  ('percent', 'Percent'),
  ('dollar', 'US dollar'),
  ('euro', 'Euro'),
  ('pound', 'Pound'),
  ('yen', 'Yen'),
];

String _format(PropValue v, PropDef d) =>
    v.kind == ValueKind.number ? formatNumberValue(v.number, PropConfig(d.config).numberFormat) : '';

String _plain(PropValue v) {
  if (v.kind != ValueKind.number) return '';
  final n = v.number;
  return n == n.roundToDouble() ? n.toInt().toString() : '$n';
}

final numberProperty = PropertyType(
  kind: PropKind.number,
  label: 'Number',
  icon: DbIco.number,
  format: _format,
  alignEnd: true,
  cell: (a) => a.mode == CellMode.table
      ? Align(alignment: Alignment.centerRight, child: textCell(a, _format(a.value, a.def), align: TextAlign.right))
      : textCell(a, _format(a.value, a.def)),
  edit: (a) => editAsText(a, initial: _plain(a.value), multiline: false, parse: (s) {
    if (s.trim().isEmpty) return Values.empty;
    final n = parseNumber(s);
    return n == null ? null : Values.number(n);
  }),
  filterOps: const [
    FilterOp.is_,
    FilterOp.isNot,
    FilterOp.greater,
    FilterOp.less,
    FilterOp.greaterOrEqual,
    FilterOp.lessOrEqual,
    FilterOp.isEmpty,
    FilterOp.isNotEmpty,
  ],
  filterValue: numberFilterValue,
  calcs: const [Calc.sum, Calc.average, Calc.median, Calc.min, Calc.max, Calc.range],
);
