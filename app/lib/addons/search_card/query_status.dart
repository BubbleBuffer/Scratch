// The status label of a result row. Provisional rule, kept in this one
// function: a page's select property whose name normalizes to "status" shows
// its option name as the label. The database's Status column is a status kind
// (same option model), so both kinds count. Pure Dart, unit-tested.

import '../../src/rust/api/types.dart';
import 'query_wording.dart' show normalizeQueryKey;

/// The label for a page's status, or null when it has none. [schema] holds
/// the definitions of the page's collection; [properties] the page's values.
String? statusLabelOf(List<PropDef> schema, List<Property> properties) {
  for (final def in schema) {
    if (normalizeQueryKey(def.name) != 'status') continue;
    if (def.kind != PropKind.select && def.kind != PropKind.status) continue;
    for (final property in properties) {
      if (property.key != def.key) continue;
      final value = property.value;
      if (value.kind != ValueKind.select && value.kind != ValueKind.status) {
        return null;
      }
      for (final option in def.options) {
        if (option.id == value.text) return option.name;
      }
      return null;
    }
  }
  return null;
}
