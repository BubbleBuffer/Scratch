// Last edited time: when the row last changed (computed, read-only).

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/db_icons.dart';
import '../../../registry/property_registry.dart';
import 'date.dart';
import 'filter_values.dart';

final editedTimeProperty = PropertyType(
  kind: PropKind.updatedTime,
  label: 'Last edited time',
  icon: DbIco.editedTime,
  advanced: true,
  format: dateText,
  cell: (a) => textCell(a, dateText(a.value, a.def)),
  filterOps: dateOps,
  filterValue: dateFilterValue,
  calcs: const [Calc.earliest, Calc.latest],
);
