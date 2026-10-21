import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/ui/database/values.dart';
import 'package:notes_app/state/agenda_store.dart';

import 'harness.dart' show initCore;
import 'today_fixture.dart';

// Hold a write open independently of bridge scheduling, then fail it. Reads
// still use the real core so the deferred week's projection is verified.
class FailingToggleVault extends Vault {
  FailingToggleVault(super.raw);
  final gate = Completer<void>();
  @override
  Future<T> run<T>(Future<T> Function(NotesVault v) f, {bool writes = false}) async {
    if (writes) { await gate.future; throw StateError('injected toggle failure'); }
    return super.run(f, writes: writes);
  }
}

Future<void> drain(TodayFixture f) async {
  await f.vault.flush();
  await f.app.feed.poll();
  final wait = Stopwatch()..start();
  while (wait.elapsed < const Duration(seconds: 10)) {
    await Future<void>.delayed(const Duration(milliseconds: 5));
    if (Vault.inFlight.value == 0 && Debounce.pending == 0 && f.app.feed.idle) return;
  }
  fail('Today did not settle');
}

void main() {
  setUpAll(initCore);
  tearDown(() => Clock.fixedAt = null);

  test('week, separate calendar bounds, summary and progress use real core projections', () async {
    final f = await createTodayFixture();
    try {
      final a = f.app.agenda;
      expect(a.weekStart, DateTime(2026, 10, 5));
      expect(a.days.last, DateTime(2026, 10, 11));
      expect(a.summary, 'Tuesday 6 October · 3 events · 5 tasks due · 3 overdue');
      expect(a.overdueItems.map((i) => i.title), ['Book Lisbon flights', 'Call plumber', 'Return library books']);
      expect(a.overdueOn(DateTime(2026, 10, 6)), isTrue);
      expect(a.overdueOn(DateTime(2026, 10, 7)), isFalse);
      expect(a.countOn(DateTime(2026, 10, 6)), 9);
      expect(a.inMotion.singleWhere((p) => p.title == 'Packing checklist').done, 2);
      a.moveWeek(1);
      await a.reload();
      expect(a.weekStart, DateTime(2026, 10, 12));
      expect(a.summary, 'Tuesday 6 October · 3 events · 5 tasks due · 3 overdue');
      a.backToToday();
      await a.reload();
      expect(a.countOn(DateTime(2026, 10, 6)), 9);
    } finally { await f.dispose(); }
  });

  test('optimistic toggles persist database status and dated block completion', () async {
    final f = await createTodayFixture();
    try {
      final a = f.app.agenda;
      final task = a.items.firstWhere((i) => i.title == 'Draft Q4 goals');
      final updates = <bool>[];
      void changed() => updates.add(a.items.firstWhere((i) => i.page == task.page).done);
      a.addListener(changed);
      final write = a.toggleDone(task);
      expect(updates.first, isTrue, reason: 'the row flips before the bridge returns');
      expect(await write, isTrue);
      a.removeListener(changed);
      expect(a.tasksDueToday, 4);
      final schema = await f.vault.run((v) => v.getSchema(collection: f.pages['Tasks']!));
      final status = schema.firstWhere((p) => p.kind == PropKind.status);
      final row = await f.vault.loadPage(task.page);
      expect(row.properties.firstWhere((p) => p.key == status.key).value.text,
          status.options.firstWhere((o) => o.name == 'Done').id);
      final todo = a.items.firstWhere((i) => i.title == 'Plant garlic' && i.block != null);
      expect(await a.toggleDone(todo), isTrue);
      await drain(f);
      expect((await f.vault.loadPage(todo.page)).blocks.singleWhere((b) => b.id == todo.block).checked, isTrue);
      expect(a.tasksDueToday, 3);
      final late = a.overdueItems.first;
      expect(await a.toggleDone(late), isTrue);
      expect(a.overdueCount, 2);
    } finally { await f.dispose(); }
  });

  test('any change batch refreshes progress and collection agenda settings', () async {
    final f = await createTodayFixture();
    try {
      final packing = f.pages['Packing checklist']!;
      final block = (await f.vault.loadPage(packing)).blocks.firstWhere((b) => b.kind == Kinds.todo && !b.checked);
      await f.vault.apply(packing, [BOp(op: OpKind.setChecked, id: block.id, kind: block.kind, text: block.text, marks: block.marks, checked: true)]);
      await drain(f);
      expect(f.app.agenda.inMotion.singleWhere((p) => p.page == packing).done, 3);
      await f.vault.run((v) => v.setAgendaSettings(collection: f.pages['Tasks']!, settings: const AgendaSettings(role: AgendaRole.off)), writes: true);
      await drain(f);
      expect(f.app.agenda.items.any((i) => i.collection == f.pages['Tasks']), isFalse);
      expect(f.app.agenda.overdueCount, 0);
    } finally { await f.dispose(); }
  });

  test('failed dated block write reverts optimistic state and exposes toast error', () async {
    final f = await createTodayFixture();
    try {
      final a = f.app.agenda;
      final todo = a.items.firstWhere((i) => i.title == 'Plant garlic' && i.block != null);
      // Change the native page without waking this isolate's feed, leaving a
      // reproducibly stale projection for setAgendaDone to reject.
      await f.vault.raw.apply(pageId: todo.page, ops: [BOp(op: OpKind.setText, id: todo.block!, kind: Kinds.todo,
          text: 'Plant garlic', marks: const [], checked: false)]);
      await f.vault.raw.flush();
      final updates = <bool>[];
      a.addListener(() => updates.add(a.items.firstWhere((i) => i.block == todo.block).done));
      expect(await a.toggleDone(todo), isFalse);
      expect(updates, [true, false]);
      expect(a.error, 'Could not update the task. Try again.');
      expect(a.tasksDueToday, 5);
    } finally { await f.dispose(); }
  });

  test('a task spanning yesterday to today does not decrement overdue count', () async {
    final f = await createTodayFixture();
    try {
      final task = f.pages['Draft Q4 goals']!;
      final schema = await f.vault.run((v) => v.getSchema(collection: f.pages['Tasks']!));
      final due = schema.firstWhere((p) => p.kind == PropKind.date);
      await f.vault.run((v) => v.setProperty(page: task, key: due.key,
          value: Values.date(dayStamp(DateTime(2026, 10, 5)), end: dayStamp(DateTime(2026, 10, 6)))), writes: true);
      await f.app.agenda.reload();
      final a = f.app.agenda;
      final item = a.items.firstWhere((i) => i.page == task);
      final write = a.toggleDone(item);
      expect(a.overdueCount, 3);
      expect(await write, isTrue);
      expect(a.overdueCount, 3);
    } finally { await f.dispose(); }
  });

  test('an overdue October 5 task completed on October 6 stays done and can be reopened', () async {
    final f = await createTodayFixture();
    try {
      final a = f.app.agenda;
      final task = a.overdueItems.firstWhere((i) => sameDay(a.itemDay(i), DateTime(2026, 10, 5)));
      final count = a.overdueCount;
      final write = a.toggleDone(task);
      expect(a.selectedDoneItems.any((i) => AgendaStore.itemKey(i) == AgendaStore.itemKey(task)), isTrue);
      expect(await write, isTrue);
      expect(a.overdueCount, count - 1);
      expect(a.overdueItems.any((i) => i.page == task.page), isFalse);
      final done = a.selectedDoneItems.singleWhere((i) => i.page == task.page);
      a.selectDay(DateTime(2026, 10, 7));
      expect(a.selectedDoneItems.any((i) => i.page == task.page), isFalse);
      a.backToToday();
      expect(await a.toggleDone(done), isTrue);
      expect(a.selectedDoneItems.any((i) => i.page == task.page), isFalse);
      expect(a.overdueItems.any((i) => i.page == task.page && !i.done), isTrue);
      expect(a.overdueCount, count);
    } finally { await f.dispose(); }
  });

  test('a week reload deferred during a failing toggle runs when the write settles', () async {
    final f = await createTodayFixture();
    final gated = FailingToggleVault(f.vault.raw);
    final a = AgendaStore(gated, f.app.feed);
    try {
      await a.reload();
      final oldPages = a.items.map((i) => AgendaStore.itemKey(i)).toSet();
      final write = a.toggleDone(a.items.firstWhere((i) => i.kind == AgendaKind.task && !i.done));
      a.moveWeek(1);
      await a.reload();
      expect(a.loading, isTrue);
      gated.gate.complete();
      expect(await write, isFalse);
      expect(a.loading, isFalse);
      final from = a.weekStart;
      final to = DateTime(from.year, from.month, from.day + 7);
      final expected = await f.vault.run((v) => v.agenda(timedFromMs: from.millisecondsSinceEpoch,
        timedToMs: to.millisecondsSinceEpoch, dayFromMs: dayStamp(from), dayToMs: dayStamp(to)));
      expect(a.items.map(AgendaStore.itemKey), expected.map(AgendaStore.itemKey));
      expect(a.items.map(AgendaStore.itemKey).toSet(), isNot(oldPages));
      expect(a.error, 'Could not update the task. Try again.');
    } finally { a.dispose(); await f.dispose(); }
  });
}
