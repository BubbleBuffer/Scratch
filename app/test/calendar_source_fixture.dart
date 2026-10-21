// Shared synthetic dated sources for task/card and Today acceptance tests.
import 'dart:convert';

import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/state/agenda_store.dart';
import 'package:notes_app/ui/database/values.dart';

Future<Map<String, String>> seedCalendarToday(Vault vault) async {
  final settings = await vault.listCalendars();
  final personal = settings.calendars
      .firstWhere((calendar) => calendar.name == 'Personal')
      .id;
  final work = settings.calendars
      .firstWhere((calendar) => calendar.name == 'Work')
      .id;
  final agent = settings.calendars
      .firstWhere((calendar) => calendar.name == 'Agent')
      .id;
  final pages = <String, String>{};
  Future<void> task(
    String title,
    String date, {
    String? calendar,
    bool done = false,
    bool checkbox = true,
    String assignee = 'me',
    int? duration,
  }) async {
    final page = await vault.createPage(title: title, pageType: 'task');
    pages[title] = page;
    for (final entry in <String, Object>{
      'status': done ? 'done' : 'todo',
      'checkbox': checkbox,
      'date': date,
      'calendar': calendar ?? personal,
      'assignee': assignee,
      'duration_min': ?duration,
    }.entries) {
      await vault.typeDataSet(page, 'tasks', [
        entry.key,
      ], jsonEncode(entry.value));
    }
  }

  await task('Water the plants', '2026-10-06');
  await task(
    'Dentist appointment',
    DateTime(2026, 10, 6, 14).toUtc().toIso8601String(),
    calendar: work,
    checkbox: false,
    duration: 45,
  );
  await task('Send overdue invoice', '2026-10-03', calendar: work);
  await task('Yesterday’s completed plan', '2026-10-05', done: true);
  await task('Done today', '2026-10-06', done: true);
  await task('Plan the weekend', '2026-10-10');
  await task('Check November budget', '2026-11-20', calendar: work);
  await task(
    'Late night handover',
    DateTime(2026, 10, 6, 23, 30).toUtc().toIso8601String(),
    calendar: work,
    duration: 90,
  );
  await task(
    'Hidden agent plan',
    '2026-10-06',
    calendar: agent,
    assignee: 'agent',
  );
  final note = await vault.createPage(title: 'Garden notes');
  pages['Garden notes'] = note;
  const todo = 'Buy compost @October 6, 2026';
  const mention = 'Reference only @October 6, 2026';
  await vault.apply(note, [
    BOp(
      op: OpKind.insert,
      id: newId(),
      kind: Kinds.todo,
      text: todo,
      checked: false,
      marks: [
        const BMark(
          start: 12,
          end: todo.length,
          kind: BMarkKind.date,
          url: '2026-10-06',
        ),
      ],
    ),
    BOp(
      op: OpKind.insert,
      id: newId(),
      kind: Kinds.paragraph,
      text: mention,
      checked: false,
      marks: [
        const BMark(
          start: 15,
          end: mention.length,
          kind: BMarkKind.date,
          url: '2026-10-06',
        ),
      ],
    ),
  ]);
  final database = await vault.createPage(title: 'Projects');
  pages['Projects'] = database;
  await vault.run((v) => v.makeCollection(page: database), writes: true);
  await vault.run(
    (v) => v.setSchema(
      collection: database,
      schema: const [
        PropDef(
          key: 'due',
          name: 'Due',
          kind: PropKind.date,
          options: [],
          config: '',
        ),
        PropDef(
          key: 'done',
          name: 'Done',
          kind: PropKind.checkbox,
          options: [],
          config: '',
        ),
      ],
    ),
    writes: true,
  );
  final schema = await vault.run((v) => v.getSchema(collection: database));
  final row = await vault.createPage(
    parent: database,
    title: 'Review project quote',
  );
  pages['Review project quote'] = row;
  await vault.run(
    (v) => v.setProperty(
      page: row,
      key: schema.firstWhere((def) => def.name == 'Due').key,
      value: Values.date(dayStamp(DateTime(2026, 10, 6))),
    ),
    writes: true,
  );
  await vault.flush();
  return pages;
}
