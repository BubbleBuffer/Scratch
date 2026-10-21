// Unit tests for the provisional status-label rule (statusLabelOf). Pure data:
// schemas and property values are built by hand.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/search_card/query_status.dart';
import 'package:notes_app/src/rust/api/types.dart';
import 'package:notes_app/ui/database/values.dart';

const doing = SelectOption(id: 'o-doing', name: 'Doing', color: 'blue');
const todo = SelectOption(id: 'o-todo', name: 'Todo', color: 'gray');

PropDef def(
  String name, {
  PropKind kind = PropKind.select,
  String key = 'k-status',
  List<SelectOption> options = const [doing, todo],
}) => PropDef(key: key, name: name, kind: kind, options: options, config: '');

void main() {
  test('a select named Status shows the option name of its value', () {
    final label = statusLabelOf(
      [def('Status')],
      [Property(key: 'k-status', value: Values.select('o-doing'))],
    );
    expect(label, 'Doing');
  });

  test('a status-kind column works the same way', () {
    final label = statusLabelOf(
      [def('Status', kind: PropKind.status)],
      [Property(key: 'k-status', value: Values.status('o-todo'))],
    );
    expect(label, 'Todo');
  });

  test('the name matches after case, spaces, underscores and dashes', () {
    for (final name in ['status', 'STATUS', ' Sta tus ', 'Sta_tus', 'sta-tus']) {
      final label = statusLabelOf(
        [def(name)],
        [Property(key: 'k-status', value: Values.select('o-doing'))],
      );
      expect(label, 'Doing', reason: name);
    }
  });

  test('other names, and kinds other than select or status, show nothing', () {
    final value = [Property(key: 'k-status', value: Values.select('o-doing'))];
    expect(statusLabelOf([def('Priority')], value), isNull);
    expect(statusLabelOf([def('Status', kind: PropKind.text)], value), isNull);
  });

  test('a page without the property, or without a value, shows nothing', () {
    expect(statusLabelOf([def('Status')], const []), isNull);
    expect(
      statusLabelOf(
        [def('Status')],
        [Property(key: 'k-status', value: Values.select(''))],
      ),
      isNull,
    );
  });

  test('a value that is not one of the options shows nothing', () {
    expect(
      statusLabelOf(
        [def('Status')],
        [Property(key: 'k-status', value: Values.select('o-gone'))],
      ),
      isNull,
    );
  });

  test('a property with another key does not count', () {
    expect(
      statusLabelOf(
        [def('Status')],
        [Property(key: 'k-other', value: Values.select('o-doing'))],
      ),
      isNull,
    );
  });
}
