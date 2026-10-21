// Status: one option in a rounded pill with a dot; options belong to the
// groups To-do, In progress and Complete (kept in the property's config).

import '../../../core/models.dart' hide StatusGroup;
import '../cells.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/db_icons.dart';
import '../editors/option_picker.dart';
import '../../../registry/property_registry.dart';
import 'filter_values.dart';
import 'select.dart';

final statusProperty = PropertyType(
  kind: PropKind.status,
  label: 'Status',
  icon: DbIco.status,
  format: optionNames,
  cell: pillsCell,
  edit: editOptions,
  filterOps: const [FilterOp.is_, FilterOp.isNot, FilterOp.isEmpty, FilterOp.isNotEmpty],
  filterValue: optionFilterValue,
  groupable: true,
);

/// The options of [def] in [group].
List<SelectOption> statusOptionsIn(PropDef def, StatusGroup group) =>
    [for (final o in def.options) if (statusGroupOf(def, o.id) == group) o];
