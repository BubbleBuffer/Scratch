// Relation: links to rows of a database (its own by default, or the one
// set in the property's config); linked titles are underlined.

import '../../../core/models.dart';
import '../cells.dart';
import '../../../ui/database/db_icons.dart';
import '../editors/relation_picker.dart';
import '../../../registry/property_registry.dart';

final relationProperty = PropertyType(
  kind: PropKind.relation,
  label: 'Relation',
  icon: DbIco.relation,
  advanced: true,
  format: (v, d) => [for (final id in v.ids) RelationTitles.of(id)?.$1 ?? ''].join(', '),
  cell: (a) => a.value.ids.isEmpty
      ? emptyCell(a)
      : RelationCell(ids: a.value.ids, style: cellStyle(a.context, a.mode), wrap: a.wrap || a.mode == CellMode.page),
  edit: (a) => editRelation(a, relationTarget(a.def, a.collection)),
  filterOps: const [FilterOp.isEmpty, FilterOp.isNotEmpty],
);
