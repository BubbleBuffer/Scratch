import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/mention_menu.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';

/// Dated to-dos and a paragraph with the same mark, on the real core.
Future<String> openTodoDateFixture(WidgetTester tester) async {
  final now = clockNow();
  final today = DateTime(now.year, now.month, now.day);
  final id = await core(tester, (v) async {
    final id = await createFixtureSpace(v, title: 'Garden plan');
    final ops = <BOp>[];
    String? after;
    for (final (title, delta, done, kind) in [
      ('Plant garlic', 0, false, Kinds.todo),
      ('Water seedlings', -1, false, Kinds.todo),
      ('Order compost', 1, false, Kinds.todo),
      ('Harvest tomatoes', -1, true, Kinds.todo),
      ('Visit the garden', 0, false, Kinds.paragraph),
    ]) {
      final day = DateTime(today.year, today.month, today.day + delta);
      final date = '@${longDate(day)}';
      final block = newId();
      ops.add(BOp(op: OpKind.insert, id: block, after: after, kind: kind,
        text: '$title $date', checked: done,
        marks: [BMark(start: title.length + 1, end: title.length + 1 + date.length, kind: BMarkKind.date, url: isoDate(day))]));
      after = block;
    }
    await v.apply(id, ops);
    await v.flush();
    return id;
  });
  nav.openPage(tester.element(find.byType(HomeScreen)), id, 'Garden plan');
  await settle(tester);
  return id;
}
