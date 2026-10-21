// Real-core status rows shared by functional and golden Today coverage.
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/ui/database/values.dart';
import 'package:notes_app/state/agenda_store.dart';

import 'harness.dart';

Future<Map<String, String>> seedTodayStatuses(WidgetTester tester) async {
  final ids = await core(tester, (vault) async {
    final ids = <String, String>{};
    final today = dayStamp(fixedNow);
    const date = PropDef(
      key: 'due',
      name: 'Due',
      kind: PropKind.date,
      options: [],
      config: '',
    );
    const state = PropDef(
      key: 'state',
      name: 'Status',
      kind: PropKind.status,
      // Groups disagree with ordering and colours, proving that neither is guessed.
      options: [
        SelectOption(id: 'complete', name: 'Delivered', color: 'blue'),
        SelectOption(id: 'todo', name: 'Queued', color: 'red'),
        SelectOption(id: 'progress', name: 'Underway', color: 'green'),
      ],
      config: '{"groups":{"complete":"complete","todo":"todo","progress":"inProgress"}}',
    );
    final tasks = await createFixtureSpace(vault, title: 'Status tasks');
    await vault.run(
      (v) => v.setSchema(collection: tasks, schema: [date, state]),
      writes: true,
    );
    for (final (title, status, offset) in [
      ('Plan the launch', 'todo', 0),
      ('Review the design', 'progress', 0),
      ('Send the brief', 'complete', 0),
      ('Book the room', 'progress', -1),
      ('Unassigned task', '', 0),
    ]) {
      final id = await vault.createPage(parent: tasks, title: title);
      ids[title] = id;
      await vault.run(
        (v) => v.setProperty(
          page: id,
          key: 'due',
          value: Values.date(today + offset * Duration.millisecondsPerDay),
        ),
        writes: true,
      );
      if (status.isNotEmpty) {
        await vault.run(
          (v) => v.setProperty(
            page: id,
            key: 'state',
            value: Values.status(status),
          ),
          writes: true,
        );
      }
    }
    final checkTasks = await createFixtureSpace(vault, title: 'Checkbox tasks');
    await vault.run(
      (v) => v.setSchema(
        collection: checkTasks,
        schema: [
          date,
          const PropDef(
            key: 'checked',
            name: 'Done',
            kind: PropKind.checkbox,
            options: [],
            config: '',
          ),
        ],
      ),
      writes: true,
    );
    final check = await vault.createPage(
      parent: checkTasks,
      title: 'Bring the notebook',
    );
    ids['Bring the notebook'] = check;
    await vault.run(
      (v) => v.setProperty(page: check, key: 'due', value: Values.date(today)),
      writes: true,
    );
    final garden = await createFixtureSpace(vault, title: 'Garden note');
    final block = newId();
    ids['Water seedlings'] = '$garden/$block';
    await vault.apply(garden, [
      BOp(
        op: OpKind.insert,
        id: block,
        kind: Kinds.todo,
        text: 'Water seedlings @today',
        checked: false,
        marks: [
          BMark(start: 16, end: 22, kind: BMarkKind.date, url: '2026-10-06'),
        ],
      ),
    ]);
    await vault.flush();
    return ids;
  });
  await finishAsync(tester, current.app.agenda.reload());
  await settle(tester);
  return ids;
}
