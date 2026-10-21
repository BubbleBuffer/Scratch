// Select: one option, drawn as a pill; picked or created in the option
// picker; groups a board.

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/db_icons.dart';
import '../editors/option_picker.dart';
import '../../../ui/database/values.dart';
import '../../../registry/property_registry.dart';
import 'filter_values.dart';

String optionNames(PropValue v, PropDef d) => [for (final id in v.optionIds) d.option(id)?.name ?? id].join(', ');

final selectProperty = PropertyType(
  kind: PropKind.select,
  label: 'Select',
  icon: DbIco.select,
  format: optionNames,
  cell: pillsCell,
  edit: editOptions,
  filterOps: const [FilterOp.is_, FilterOp.isNot, FilterOp.isEmpty, FilterOp.isNotEmpty],
  filterValue: optionFilterValue,
  groupable: true,
);
