// Phone: underlined text; edited as text.

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/db_icons.dart';
import '../editors/text_editor.dart';
import '../../../registry/property_registry.dart';
import 'filter_values.dart';
import 'text.dart';

String _format(PropValue v, PropDef d) => v.kind == ValueKind.text ? v.text : '';

final phoneProperty = PropertyType(
  kind: PropKind.phone,
  label: 'Phone',
  icon: DbIco.phone,
  advanced: true,
  format: _format,
  cell: (a) => textCell(a, _format(a.value, a.def), underline: true),
  edit: (a) => editAsText(a, initial: _format(a.value, a.def), multiline: false, parse: (s) => textValue(PropKind.phone, s)),
  filterOps: textOps,
  filterValue: textFilterValue,
);
