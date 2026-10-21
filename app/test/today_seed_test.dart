import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/ui/database/values.dart';
import 'package:notes_app/features/demo.dart';
import 'package:notes_app/features/getting_started.dart';

import 'harness.dart' show initCore, resetTestClock;
import 'today_fixture.dart';

Future<List<AgendaItem>> _agenda(Vault vault, Day from, Day to) => vault.run(
  (v) => v.agenda(
    timedFromMs: DateTime(from.year, from.month, from.day).millisecondsSinceEpoch,
    timedToMs: DateTime(to.year, to.month, to.day).millisecondsSinceEpoch,
    dayFromMs: from.ms,
    dayToMs: to.ms,
  ),
);

void main() {
  setUpAll(initCore);
  tearDown(() => Clock.fixedAt = null);

  test('fresh vault starts with Today guidance, Tasks and Calendar', () async {
    resetTestClock();
    final dir = await Directory.systemTemp.createTemp('notes_first_run_test_');
    final vault = await Vault.open('${dir.path}${Platform.pathSeparator}vault');
    try {
      final gettingStarted = await createGettingStarted(vault);
      await vault.flush();
      final roots = await vault.listChildren(null);
      expect(roots.map((p) => p.title), [gettingStartedTitle, 'Tasks', 'Calendar']);
      final page = await vault.loadPage(gettingStarted);
      expect(page.blocks.any((b) => b.text == 'Plan your day with Today'), isTrue);
      expect(page.blocks.any((b) => b.text.contains('@today')), isTrue);
      expect(page.blocks.any((b) => b.text.contains('daily note')), isTrue);
      final tasks = roots.firstWhere((p) => p.title == 'Tasks').id;
      final schema = await vault.run((v) => v.getSchema(collection: tasks));
      final status = schema.firstWhere((d) => d.name == 'Status');
      expect(status.options.map((o) => o.name), ['To do', 'Doing', 'Done']);
      expect((jsonDecode(status.config) as Map)['groups'], {
        status.options[0].id: 'todo',
        status.options[1].id: 'inProgress',
        status.options[2].id: 'complete',
      });
      final views = await vault.run((v) => v.listViews(collection: tasks));
      expect(views.map((v) => v.kind), [ViewKind.table, ViewKind.board]);
      expect(views.last.groupBy, status.key);
      final calendar = roots.firstWhere((p) => p.title == 'Calendar').id;
      final calendarViews = await vault.run((v) => v.listViews(collection: calendar));
      expect(calendarViews.map((v) => v.kind), [ViewKind.calendar, ViewKind.table]);
      final today = Day.of(clockNow());
      final agenda = await _agenda(vault, today, today.add(2));
      expect(agenda.where((i) => i.kind == AgendaKind.task).length, 2);
      expect(agenda.where((i) => i.kind == AgendaKind.event).length, 1);
      final taskDays = agenda.where((i) => i.kind == AgendaKind.task).map((i) => i.startMs).toSet();
      expect(taskDays, {today.ms, today.add(1).ms});
      final event = agenda.singleWhere((i) => i.kind == AgendaKind.event);
      expect(event.hasTime, isTrue);
      expect(DateTime.fromMillisecondsSinceEpoch(event.startMs).hour, 18);
    } finally {
      vault.dispose();
      await dir.delete(recursive: true);
    }
  });

  test('demo seed has relative agenda, rich pages, spaces and checklist progress', () async {
    final fixture = await createTodayFixture();
    final vault = fixture.vault;
    final ids = fixture.pages;
    try {
      final roots = await vault.listChildren(null);
      expect(roots.map((p) => p.title), demoSpaces);
      for (final title in demoSpaces) {
        expect((await vault.loadPage(ids[title]!)).color, isNotNull);
      }
      final today = Day.of(clockNow());
      final tasks = await vault.run((v) => v.query(scope: ids['Tasks'], filter: null, sorts: const [], offset: 0, limit: 50));
      expect(tasks.rows.map((r) => r.title).toSet(), demoTaskTitles.toSet());
      expect(tasks.rows.length, 12);
      final todayItems = await _agenda(vault, today, today.add(1));
      final due = todayItems.where((i) => i.kind == AgendaKind.task && !i.done);
      expect(due.length, 5);
      expect(todayItems.where((i) => i.kind == AgendaKind.task && i.done).map((i) => i.title), ['Water the seedlings']);
      final events = todayItems.where((i) => i.kind == AgendaKind.event).toList();
      expect(events.map((i) => i.title), ['Standup', 'Dentist', 'Pottery class']);
      expect(events.map((i) => DateTime.fromMillisecondsSinceEpoch(i.startMs).hour), [9, 14, 19]);
      expect(events.every((i) => i.hasTime && i.endMs != null), isTrue);
      final overdue = await vault.run(
        (v) =>
            v.overdue(timedBeforeMs: DateTime(today.year, today.month, today.day).millisecondsSinceEpoch, dayBeforeMs: today.ms, limit: 50),
      );
      expect(overdue.map((i) => i.title), ['Book Lisbon flights', 'Call plumber', 'Return library books']);
      final overdueCount = await vault.run(
        (v) => v.overdueCount(timedBeforeMs: DateTime(today.year, today.month, today.day).millisecondsSinceEpoch, dayBeforeMs: today.ms),
      );
      expect(overdueCount, 3);
      expect(events.every((i) => i.space == ids['Work'] && i.spaceColor == 'duvet'), isTrue);
      final progress = await vault.run((v) => v.inMotion(limit: 5));
      expect(progress.length, 4);
      final taskProgress = progress.singleWhere((p) => p.page == ids['Tasks']);
      expect((taskProgress.done, taskProgress.total), (1, 12));
      final gardenProgress = progress.singleWhere((p) => p.page == ids['Garden plan']);
      expect((gardenProgress.done, gardenProgress.total), (1, 3));
      final packing = progress.singleWhere((p) => p.page == ids['Packing checklist']);
      expect((packing.done, packing.total), (2, 7));
      final garden = await vault.loadPage(ids['Garden plan']!);
      final garlic = garden.blocks.singleWhere((b) => b.text.startsWith('Plant garlic '));
      expect(garlic.kind, Kinds.todo);
      expect(garlic.marks.single.kind, BMarkKind.date);
      expect(garlic.marks.single.url, '2026-10-06');
      final lisbon = await vault.loadPage(ids['Lisbon in May']!);
      expect(lisbon.icon, '🚋');
      expect(lisbon.cover, isNotNull);
      expect(
        lisbon.blocks.map((b) => b.kind),
        containsAll([Kinds.callout, Kinds.columnList, Kinds.column, Kinds.toggle, 'database', Kinds.image]),
      );
      expect(lisbon.blocks.where((b) => b.kind == Kinds.todo).length, 7);
      expect(lisbon.blocks.where((b) => b.kind == Kinds.todo && b.checked).length, 2);
      final bookings = await vault.run((v) => v.listViews(collection: ids['Bookings']!));
      expect(bookings.single.kind, ViewKind.board);
      final backlinks = await vault.backlinks(ids['Lisbon in May']!);
      expect(backlinks.map((p) => p.title), contains('Weekly review'));
      // Completing a Doing task resolves the seed's configured Complete group.
      await vault.run((v) => v.setAgendaDone(page: ids['Draft Q4 goals']!, done: true), writes: true);
      final updatedProgress = await vault.run((v) => v.inMotion(limit: 5));
      final updatedTasks = updatedProgress.singleWhere((p) => p.page == ids['Tasks']);
      expect((updatedTasks.done, updatedTasks.total), (2, 12));
      expect((await _agenda(vault, today, today.add(1))).singleWhere((i) => i.page == ids['Draft Q4 goals']).done, isTrue);
    } finally {
      await fixture.dispose();
    }
  });
}
