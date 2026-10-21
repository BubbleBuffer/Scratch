import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import 'package:notes_app/ui/ui.dart';

import '../../addon_harness.dart';
import 'task_fixture.dart';

Future<List<String>> dayTaskLinks(Vault vault, DateTime date) async {
  final note = await vault.run(
    (v) => v.dailyNote(
      dayMs: DateTime.utc(
        date.year,
        date.month,
        date.day,
      ).millisecondsSinceEpoch,
      title: '',
      create: false,
    ),
  );
  if (note == null) return [];
  final data = await vault.loadPage(note);
  expect(data.blocks.first.text, 'Tasks');
  return data.blocks
      .skip(1)
      .takeWhile((b) => b.depth > 0)
      .where((b) => b.kind == Kinds.linkToPage && b.target != null)
      .map((b) => b.target!)
      .toList();
}

class FailingDayLinkVault extends Vault {
  FailingDayLinkVault(Vault vault) : super(vault.raw) {
    onWrite = vault.onWrite;
  }

  @override
  Future<bool> linkTaskToDay({
    required int dayMs,
    required String taskPageId,
  }) async {
    throw StateError('Journal unavailable');
  }
}

void main() {
  setUpAll(initCore);

  for (final fromToday in [true, false]) {
    appTest(
      '${fromToday ? 'Today' : 'Calendar'} composer links its edited task day',
      (tester) async {
        final fixture = await pumpApp(tester, mobile: false, empty: true);
        String? created;
        await pumpTaskHost(
          tester,
          fixture,
          TaskComposer(
            initialDate: DateTime(2026, 10, 8),
            initialHour: 23,
            fromToday: fromToday,
            onCreated: (id) => created = id,
          ),
          width: 440,
        );
        await tester.enterText(taskField('title'), 'Review tomorrow');
        await tester.enterText(taskField('date'), '2026-10-09');
        await tapAndSettle(tester, find.text('Create task'));
        expect(created, isNotNull);
        expect(
          (await core(tester, (v) => loadTask(v, created!))).dateTime,
          DateTime(2026, 10, 9, 23),
        );
        expect(
          await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 9))),
          [created],
        );
        expect(
          await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 8))),
          isEmpty,
        );
      },
    );
  }

  appTest(
    'dated saves link once and clearing a date retains historical links',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final page = await core(tester, (v) async {
        final page = await v.createPage(
          title: 'Retained links',
          pageType: taskType,
        );
        await saveTask(v, page, const TaskData());
        expect(
          await v.dayNotesInRange(
            fromMs: DateTime.utc(2026, 10, 8).millisecondsSinceEpoch,
            toMs: DateTime.utc(2026, 10, 11).millisecondsSinceEpoch,
          ),
          isEmpty,
        );
        const task = TaskData(date: '2026-10-08');
        await saveTask(v, page, task);
        await saveTask(v, page, task);
        expect(await dayTaskLinks(v, DateTime(2026, 10, 8)), [page]);
        await saveTask(v, page, const TaskData(date: '2026-10-09'));
        await saveTask(v, page, const TaskData());
        return page;
      });
      expect((await core(tester, (v) => loadTask(v, page))).date, isNull);
      expect(
        await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 8))),
        [page],
      );
      expect(
        await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 9))),
        [page],
      );
    },
  );

  appTest(
    'Journal link failures are logged and task saves and rescheduling continue',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final messages = <String>[];
      final previousDebugPrint = debugPrint;
      debugPrint = (String? message, {int? wrapWidth}) {
        if (message != null) messages.add(message);
      };
      try {
        final page = await core(tester, (v) async {
          final page = await v.createPage(
            title: 'Still saved',
            pageType: taskType,
          );
          await saveTask(
            FailingDayLinkVault(v),
            page,
            const TaskData(date: '2026-10-08'),
          );
          expect((await loadTask(v, page)).date, '2026-10-08');
          await rescheduleTask(
            FailingDayLinkVault(v),
            page,
            DateTime(2026, 10, 10),
          );
          return page;
        });
        expect(
          (await core(tester, (v) => loadTask(v, page))).date,
          '2026-10-10',
        );
        expect(
          await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 8))),
          isEmpty,
        );
        expect(
          await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 10))),
          isEmpty,
        );
        expect(
          messages.where(
            (message) =>
                message.contains('Task Journal link failed for $page') &&
                message.contains('Journal unavailable'),
          ),
          hasLength(2),
        );
      } finally {
        debugPrint = previousDebugPrint;
      }
    },
  );

  appTest(
    'registered Today primary action and task command preserve the selected day',
    (tester) async {
      final fixture = await pumpApp(tester, mobile: false, empty: true);
      final day = DateTime(2026, 10, 11);
      fixture.app.agenda.selectDay(day);
      await pumpTaskHost(
        tester,
        fixture,
        Builder(
          builder: (context) => TextBtn(
            'Registered task',
            onTap: () => appAddonContext.shellPrimaryActions
                .of('task.new')!
                .run(context),
          ),
        ),
      );
      await tapAndSettle(tester, find.text('Registered task'));
      final composer = tester.widget<TaskComposer>(find.byType(TaskComposer));
      expect(composer.fromToday, isTrue);
      expect(composer.initialDate, day);
      await tapAndSettle(tester, find.bySemanticsLabel('Close new task'));
      await pumpTaskHost(
        tester,
        fixture,
        Builder(
          builder: (context) => TextBtn(
            'Registered command',
            onTap: () => appAddonContext.noteTypes
                .of(taskType)!
                .commands
                .firstWhere((command) => command.id == 'task.new')
                .run(context),
          ),
        ),
      );
      await tapAndSettle(tester, find.text('Registered command'));
      final commandComposer = tester.widget<TaskComposer>(
        find.byType(TaskComposer),
      );
      expect(commandComposer.fromToday, isTrue);
      expect(commandComposer.initialDate, day);
    },
  );

  appTest(
    'composer validates then creates a timed agent task in the chosen calendar',
    (tester) async {
      final fixture = await pumpApp(tester, mobile: false, empty: true);
      String? created;
      await pumpTaskHost(
        tester,
        fixture,
        TaskComposer(
          initialDate: DateTime(2026, 10, 8),
          onCreated: (id) => created = id,
        ),
        width: 440,
      );
      await tapAndSettle(tester, find.text('Create task'));
      expect(find.text('Give the task a title.'), findsOneWidget);
      await tester.enterText(taskField('title'), 'Review the trip plan');
      await tester.enterText(taskField('date'), '2026-02-31');
      await tapAndSettle(tester, find.text('Create task'));
      expect(find.text('Choose a valid date (YYYY-MM-DD).'), findsOneWidget);
      await tester.enterText(taskField('date'), '2026-10-08');
      await tapAndSettle(tester, find.text('Me'));
      await tapAndSettle(tester, find.text('Agent'));
      await tapAndSettle(tester, find.text('● Personal'));
      await tapAndSettle(tester, find.text('● Work'));
      final addTime = find.byWidgetPredicate(
        (w) => w is StacksCheckbox && !w.checked,
      );
      await tapAndSettle(tester, addTime);
      await tester.enterText(taskField('time'), '10:30');
      await tester.enterText(taskField('duration'), '90');
      await tapAndSettle(tester, find.text('Create task'));
      expect(created, isNotNull);
      final page = await core(tester, (v) => v.loadPage(created!));
      final task = await core(tester, (v) => loadTask(v, created!));
      expect(page.pageType, taskType);
      expect(page.title, 'Review the trip plan');
      expect(task.status, 'todo');
      expect(task.checkbox, isTrue);
      expect(task.assignee, 'agent');
      // Assigning the agent started one run, linked to the task.
      expect(task.agentRun, isNotNull);
      expect(
        task.date,
        DateTime(2026, 10, 8, 10, 30).toUtc().toIso8601String(),
      );
      expect(task.durationMin, 90);
      final calendars = await core(tester, (v) => v.listCalendars());
      expect(
        task.calendar,
        calendars.calendars.firstWhere((c) => c.name == 'Work').id,
      );
      final item = await indexedTask(tester, created!, DateTime(2026, 10, 8));
      expect(item.hasTime, isTrue);
      expect((item.endMs! - item.startMs).toInt(), 90 * 60000);
      expect(item.assignee, 'agent');
    },
  );

  appTest(
    'task page commits date and time submissions and status controls without losing extra data or body',
    (tester) async {
      final fixture = await pumpApp(tester, mobile: false, empty: true);
      final page = await core(tester, (v) async {
        final page = await v.createPage(
          title: 'Plan the garden',
          pageType: taskType,
        );
        await saveTask(
          v,
          page,
          const TaskData(date: '2026-10-08', calendar: null),
        );
        await v.typeDataSet(page, taskAddonId, [
          'future_field',
        ], '{"keep":true}');
        return page;
      });
      await pumpTaskHost(
        tester,
        fixture,
        SizedBox(
          height: 700,
          child: TaskNote(
            args: NoteArgs(
              pageId: page,
              title: 'Plan the garden',
              editor: const Center(child: Text('Real note body host')),
            ),
            presentation: TaskPresentation.page,
          ),
        ),
      );
      await tester.enterText(taskField('date'), '2026-10-09');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await settle(tester);
      expect((await core(tester, (v) => loadTask(v, page))).date, '2026-10-09');
      expect(
        await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 8))),
        [page],
      );
      expect(
        await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 9))),
        [page],
      );
      await tester.ensureVisible(find.text('To do'));
      await tapAndSettle(tester, find.text('To do'));
      await tapAndSettle(tester, find.text('In progress'));
      await tester.ensureVisible(find.text('Hide checkbox'));
      await tapAndSettle(tester, find.text('Hide checkbox'));
      final task = await core(tester, (v) => loadTask(v, page));
      expect(task.status, 'doing');
      expect(task.checkbox, isFalse);
      final data = jsonDecode(
        await core(tester, (v) => v.typeData(page, taskAddonId)),
      ) as Map<String, dynamic>;
      expect(data['future_field'], {'keep': true});
      expect(find.text('Real note body host'), findsOneWidget);
      final addTime = find.byWidgetPredicate(
        (w) => w is StacksCheckbox && !w.checked,
      );
      await tester.ensureVisible(addTime);
      await tapAndSettle(tester, addTime);
      await tester.enterText(taskField('time'), '10:45');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await settle(tester);
      expect(
        (await core(tester, (v) => loadTask(v, page))).date,
        DateTime(2026, 10, 9, 10, 45).toUtc().toIso8601String(),
      );
      await tester.enterText(taskField('duration'), '45');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await settle(tester);
      expect((await core(tester, (v) => loadTask(v, page))).durationMin, 45);
      await tester.ensureVisible(find.text('Clear'));
      await tapAndSettle(tester, find.text('Clear'));
      expect((await core(tester, (v) => loadTask(v, page))).date, isNull);
      expect(
        await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 8))),
        [page],
      );
      expect(
        await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 9))),
        [page],
      );
    },
  );

  appTest(
    'rescheduling a task preserves its duration, calendar, assignee and status',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final page = await core(tester, (v) async {
        final page = await v.createPage(
          title: 'Late review',
          pageType: taskType,
        );
        await saveTask(
          v,
          page,
          TaskData(
            date: DateTime(2026, 10, 8, 23, 30).toUtc().toIso8601String(),
            durationMin: 90,
            status: 'doing',
            assignee: 'agent',
            calendar: 'personal',
            checkbox: false,
          ),
        );
        await rescheduleTask(v, page, DateTime(2026, 10, 10));
        return page;
      });
      final task = await core(tester, (v) => loadTask(v, page));
      expect(
        task.date,
        DateTime(2026, 10, 10, 23, 30).toUtc().toIso8601String(),
      );
      expect(task.durationMin, 90);
      expect(task.calendar, 'personal');
      expect(task.assignee, 'agent');
      expect(task.status, 'doing');
      expect(task.checkbox, isFalse);
      expect(
        await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 8))),
        [page],
      );
      expect(
        await core(tester, (v) => dayTaskLinks(v, DateTime(2026, 10, 10))),
        [page],
      );
    },
  );

  appTest('a task in a deleted calendar can still update status and date', (
    tester,
  ) async {
    final fixture = await pumpApp(tester, mobile: false, empty: true);
    String? deletedCalendar;
    final page = await core(tester, (v) async {
      await v.listCalendars();
      deletedCalendar = await v.createCalendar(
        name: 'Old plans',
        color: 'moss',
        showInCalendar: true,
        showInToday: true,
      );
      final page = await v.createPage(
        title: 'Retained plan',
        pageType: taskType,
      );
      await saveTask(
        v,
        page,
        TaskData(date: '2026-10-08', calendar: deletedCalendar),
      );
      await v.deleteCalendar(deletedCalendar!);
      return page;
    });
    await pumpTaskHost(
      tester,
      fixture,
      SizedBox(
        height: 700,
        child: TaskNote(
          args: NoteArgs(pageId: page, title: 'Retained plan'),
          presentation: TaskPresentation.page,
        ),
      ),
    );
    expect(find.text('Unavailable calendar'), findsOneWidget);
    await tapAndSettle(tester, find.text('To do'));
    await tapAndSettle(tester, find.text('In progress'));
    expect((await core(tester, (v) => loadTask(v, page))).status, 'doing');
    await tester.enterText(taskField('date'), '2026-10-09');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await settle(tester);
    final task = await core(tester, (v) => loadTask(v, page));
    expect(task.date, '2026-10-09');
    expect(task.calendar, deletedCalendar);
    expect(find.text('Choose an available calendar.'), findsNothing);
  });
}
