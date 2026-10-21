// Checkbox: a tap toggles it; groups a board into unchecked and checked.

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/db_icons.dart';
import '../../../ui/database/values.dart';
import '../../../registry/property_registry.dart';

final checkboxProperty = PropertyType(
  kind: PropKind.checkbox,
  label: 'Checkbox',
  icon: DbIco.checkbox,
  format: (v, d) => v.kind == ValueKind.checkbox && v.checked ? 'Yes' : 'No',
  cell: checkboxCell,
  tap: (v) => Values.checkbox(!(v.kind == ValueKind.checkbox && v.checked)),
  filterOps: const [FilterOp.checked, FilterOp.unchecked],
  calcs: const [Calc.checked, Calc.unchecked, Calc.percentChecked],
  groupable: true,
);
