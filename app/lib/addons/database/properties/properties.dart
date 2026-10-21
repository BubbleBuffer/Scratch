// The built-in property types, in the type picker's order. A new type is a
// new file in this folder plus one line here.

import '../../../registry/property_registry.dart';
import 'checkbox.dart';
import 'created_time.dart';
import 'date.dart';
import 'edited_time.dart';
import 'email.dart';
import 'files.dart';
import 'formula.dart';
import 'multi_select.dart';
import 'number.dart';
import 'phone.dart';
import 'relation.dart';
import 'rollup.dart';
import 'select.dart';
import 'status.dart';
import 'text.dart';
import 'url.dart';

void registerBuiltinProperties(PropertyRegistry r) {
  for (final t in [
    textProperty,
    numberProperty,
    selectProperty,
    multiSelectProperty,
    statusProperty,
    dateProperty,
    checkboxProperty,
    urlProperty,
    emailProperty,
    phoneProperty,
    filesProperty,
    relationProperty,
    rollupProperty,
    formulaProperty,
    createdTimeProperty,
    editedTimeProperty,
  ]) {
    r.register(t);
  }
}
