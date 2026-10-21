// Multi-select: several options as pills; a row shows in each of its
// options' board groups.

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/db_icons.dart';
import '../editors/option_picker.dart';
import '../../../registry/property_registry.dart';
import 'filter_values.dart';
import 'select.dart';

final multiSelectProperty = PropertyType(
  kind: PropKind.multiSelect,
  label: 'Multi-select',
  icon: DbIco.multiSelect,
  format: optionNames,
  cell: pillsCell,
  edit: editOptions,
  filterOps: const [FilterOp.contains, FilterOp.notContains, FilterOp.isEmpty, FilterOp.isNotEmpty],
  filterValue: optionFilterValue,
  groupable: true,
);
