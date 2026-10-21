// Created time: when the row was created (computed, read-only).

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/db_icons.dart';
import '../../../registry/property_registry.dart';
import 'date.dart';
import 'filter_values.dart';

final createdTimeProperty = PropertyType(
  kind: PropKind.createdTime,
  label: 'Created time',
  icon: DbIco.createdTime,
  advanced: true,
  format: dateText,
  cell: (a) => textCell(a, dateText(a.value, a.def)),
  filterOps: dateOps,
  filterValue: dateFilterValue,
  calcs: const [Calc.earliest, Calc.latest],
);
