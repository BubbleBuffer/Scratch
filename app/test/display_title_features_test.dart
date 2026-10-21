// Page names in the features that list pages by name without the page
// stack's label: trash rows, their parent path, the purge prompt and the
// and reminder
// source names (the first line, else "Untitled").

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/reminders/reminder_plan.dart';
import 'package:notes_app/features/trash/trash_panel.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';
import 'reminder_store_test.dart'
    as reminders
    show fixture, addEvent, drain, closeFixture;

Finder key(String k) => find.byKey(ValueKey(k));

const line = 'Groceries for Sunday';

/// An untitled page whose body starts with [text]; returns its id.
Future<String> untitledPage(WidgetTester tester, String text) =>
    core(tester, (v) async {
      final id = await v.createPage();
      await v.apply(id, [
        BOp(
          op: OpKind.insert,
          id: newId(),
          kind: Kinds.paragraph,
          text: text,
          marks: const [],
          checked: false,
        ),
      ]);
      return id;
    });

void main() {
  setUpAll(initCore);

  test(
    'agenda and reminder stores load an untitled source first line',
    () async {
      final f = await reminders.fixture();
      try {
        await f.app.vault.renamePage(f.calendar, '');
        await f.app.vault.apply(f.calendar, [
          BOp(
            op: OpKind.insert,
            id: newId(),
            kind: Kinds.paragraph,
            text: line,
            marks: const [],
            checked: false,
          ),
        ]);
        final schema = await f.app.vault.run(
          (v) => v.getSchema(collection: f.calendar),
        );
        await f.app.vault.run(
          (v) => v.setSchema(
            collection: f.calendar,
            schema: [
              ...schema,
              const PropDef(
                key: '',
                name: 'Done',
                kind: PropKind.checkbox,
                options: [],
                config: '',
              ),
            ],
          ),
          writes: true,
        );
        await f.app.vault.run(
          (v) => v.setAgendaSettings(
            collection: f.calendar,
            settings: const AgendaSettings(role: AgendaRole.tasks),
          ),
          writes: true,
        );
        await reminders.addEvent(f.app, f.calendar, f.date);
        await reminders.drain(f.app);
        await f.app.agenda.reload();
        expect(f.app.agenda.sourceTitles[f.calendar], line);
        expect(
          f.app.reminders.plan
              .singleWhere((r) => r.kind == ReminderKind.task)
              .body,
          'Due now · $line',
        );
      } finally {
        await reminders.closeFixture(f.app, f.dir);
      }
    },
  );

  appTest('trash rows, their parent path and purge prompt use the first line', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final parent = await untitledPage(tester, '\n  $line\nsecond line');
    final gone = await untitledPage(tester, '\n  $line\nsecond line');
    final child = await core(
      tester,
      (v) => v.createPage(parent: parent, title: 'Shop list'),
    );
    await core(tester, (v) => v.deletePage(gone));
    await core(tester, (v) => v.deletePage(child));

    unawaited(showTrash(tester.element(find.byType(HomeScreen))));
    await settleUntil(
      tester,
      () => key('trash-row-$gone').evaluate().isNotEmpty,
    );

    // An untitled trashed page: its row shows the first line, not "Untitled".
    final row = find.descendant(
      of: key('trash-row-$gone'),
      matching: find.text(line),
    );
    expect(row, findsOneWidget);
    expect(
      find.descendant(
        of: key('trash-row-$gone'),
        matching: find.text('Untitled'),
      ),
      findsNothing,
    );
    // A titled page keeps its title; its parent path names the untitled
    // parent by its first line.
    expect(
      find.descendant(
        of: key('trash-row-$child'),
        matching: find.text('Shop list'),
      ),
      findsOneWidget,
    );
    expect(tester.widget<Text>(key('trash-path-$child')).data, line);

    // The purge prompt names the page the same way.
    await tapAndSettle(tester, key('trash-delete-$gone'));
    expect(find.text('Delete "$line" forever?'), findsOneWidget);
    await tapAndSettle(tester, find.text('Cancel'));
  });

  test('a reminder names its source by the first line, or "Untitled" when none is loaded', () {
    AgendaItem item(String page) => AgendaItem(
      page: page,
      block: 'block',
      title: 'Buy milk',
      kind: AgendaKind.task,
      startMs: DateTime(2026, 10, 6, 12).millisecondsSinceEpoch,
      endMs: null,
      hasTime: true,
      done: false,
      updatedMs: 0,
      collection: null,
    );
    List<PlannedReminder> tasks(Map<String, String> names) => planReminders(
      items: [item('page'), item('none')],
      overdueNow: 0,
      settings: ReminderSettings(sourceNames: names),
      now: DateTime(2026, 10, 6, 11, 20),
    ).where((r) => r.kind == ReminderKind.task).toList();

    final named = tasks({'page': displayTitle('', firstLine: line)});
    expect(
      named.map((r) => r.body),
      unorderedEquals(['Due now · $line', 'Due now · Untitled']),
    );
    expect(
      tasks({'page': ''}).map((r) => r.body),
      everyElement('Due now · Untitled'),
    );
  });
}
