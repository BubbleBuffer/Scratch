// Text: plain text, wraps when the column wraps; edited in a box over the
// cell.

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/db_icons.dart';
import '../editors/text_editor.dart';
import '../../../registry/property_registry.dart';
import 'filter_values.dart';

const textOps = [
  FilterOp.contains,
  FilterOp.notContains,
  FilterOp.is_,
  FilterOp.isNot,
  FilterOp.startsWith,
  FilterOp.endsWith,
  FilterOp.isEmpty,
  FilterOp.isNotEmpty,
];

String _format(PropValue v, PropDef d) => v.kind == ValueKind.text || v.kind == ValueKind.url ? v.text : '';

final textProperty = PropertyType(
  kind: PropKind.text,
  label: 'Text',
  icon: DbIco.text,
  format: _format,
  cell: (a) => textCell(a, _format(a.value, a.def)),
  edit: (a) => editAsText(a, initial: _format(a.value, a.def), parse: (s) => textValue(PropKind.text, s)),
  filterOps: textOps,
  filterValue: textFilterValue,
);
