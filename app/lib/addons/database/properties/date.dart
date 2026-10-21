// Date: a day or a time, optionally with an end ("July 23, 2020 → July 31,
// 2020"); edited in the date picker; drives calendars and timelines.

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/db_icons.dart';
import '../editors/date_picker.dart';
import '../../../registry/property_registry.dart';
import 'filter_values.dart';

/// Date formats in menu order: (id, label).
const dateFormats = [
  ('full', 'Full date'),
  ('md', 'Month/Day'),
  ('short', 'Month/Day/Year'),
  ('iso', 'Year/Month/Day'),
  ('relative', 'Relative'),
];

const dateOps = [
  FilterOp.is_,
  FilterOp.before,
  FilterOp.after,
  FilterOp.onOrBefore,
  FilterOp.onOrAfter,
  FilterOp.isEmpty,
  FilterOp.isNotEmpty,
];

final dateProperty = PropertyType(
  kind: PropKind.date,
  label: 'Date',
  icon: DbIco.date,
  format: dateText,
  cell: (a) => textCell(a, dateText(a.value, a.def)),
  edit: editDate,
  filterOps: dateOps,
  filterValue: dateFilterValue,
  calcs: const [Calc.earliest, Calc.latest],
);
