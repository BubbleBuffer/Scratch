import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/addons/tasks/calendar_note_drop.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/note_drag_payload.dart';
import 'package:notes_app/ui/ui.dart' show Hover;

import '../../addon_harness.dart';

Finder key(String value) => find.byKey(ValueKey(value));

Future<void> dragSidebarNote(
  WidgetTester tester,
  String page,
  Finder target, {
  bool settleAfter = true,
}) async {
  final from = tester.getCenter(key('tree-$page'));
  final to = tester.getCenter(target);
  final gesture = await tester.startGesture(
    from,
    kind: PointerDeviceKind.mouse,
  );
  for (var step = 1; step <= 12; step++) {
    await gesture.moveTo(Offset.lerp(from, to, step / 12)!);
    await tester.pump(const Duration(milliseconds: 16));
  }
  await gesture.up();
  if (settleAfter) await settle(tester);
}

/// Pumps in small fake-time steps until [finder] shows, waiting on real time
/// for native writes. Unlike [settle] it does not run fake time ahead while
/// I/O is slow, so a timed toast is still up when this returns.
Future<void> pumpUntilShown(WidgetTester tester, Finder finder) async {
  final waited = Stopwatch()..start();
  while (finder.evaluate().isEmpty &&
      waited.elapsed < const Duration(seconds: 20)) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 5)),
    );
    await tester.pump(const Duration(milliseconds: 16));
  }
  expect(finder, findsOneWidget);
}

Future<void> openTestCalendar(WidgetTester tester) async {
  unawaited(openCalendar(tester.element(find.byType(Hover).first)));
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  appTest('cancelling a sidebar drag leaves the page at its original parent', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final budget = tv.pages['Budget']!;
    final from = tester.getCenter(key('tree-$budget'));
    final to = tester.getCenter(key('tree-${tv.pages['Garden plan']}'));
    final gesture = await tester.startGesture(
      from,
      kind: PointerDeviceKind.mouse,
    );
    for (var step = 1; step <= 8; step++) {
      await gesture.moveTo(Offset.lerp(from, to, step / 8)!);
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(key('tree-drop-inside'), findsOneWidget);
    await gesture.cancel();
    await settle(tester);
    expect(
      (await tester.runAsync(() => tv.vault.loadPage(budget)))!.parent,
      isNull,
    );
    expect(key('tree-drop-inside'), findsNothing);
  });

  appTest(
    'sidebar note drop adds one month mention, duplicates explain, Undo removes only the addition',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      final page = (await tester.runAsync(() async {
        final page = await tv.vault.createPage(title: 'Calendar note');
        await tv.vault.apply(page, [
          const BOp(
            op: OpKind.insert,
            id: 'body',
            kind: Kinds.paragraph,
            text: 'Keep this body',
            marks: [],
            checked: false,
          ),
        ]);
        await tv.vault.flush();
        return page;
      }))!;
      await settle(tester);
      await openTestCalendar(tester);
      final target = key('calendar-day-2026-10-07-10');
      await dragSidebarNote(tester, page, target, settleAfter: false);
      await pumpUntilShown(tester, find.text('Added to calendar.'));
      var data = (await tester.runAsync(() => tv.vault.loadPage(page)))!;
      expect(data.parent, isNull);
      expect(data.blocks.map((b) => b.text), ['Keep this body', '@2026-10-07']);
      expect(data.blocks.last.marks.single.kind, BMarkKind.date);
      expect(data.blocks.last.marks.single.url, '2026-10-07');
      await tester.tap(find.text('Undo'));
      await settle(tester);
      data = (await tester.runAsync(() => tv.vault.loadPage(page)))!;
      expect(data.blocks.map((b) => b.text), ['Keep this body']);
      await dragSidebarNote(tester, page, target);
      await dragSidebarNote(tester, page, target, settleAfter: false);
      await pumpUntilShown(tester, find.text('Already scheduled on this day.'));
      await settle(tester);
      data = (await tester.runAsync(() => tv.vault.loadPage(page)))!;
      expect(
        data.blocks
            .where(
              (b) => b.marks.any(
                (m) => m.kind == BMarkKind.date && m.url == '2026-10-07',
              ),
            )
            .length,
        1,
      );
      expect(find.text('Undo'), findsNothing);
    },
  );

  appTest(
    'sidebar task drop onto week hour sets the slot and Undo restores date only',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      final page = (await tester.runAsync(() async {
        final page = await tv.vault.createPage(
          title: 'Week task',
          pageType: 'task',
        );
        await saveTask(
          tv.vault,
          page,
          TaskData(
            date: taskDateString(DateTime(2026, 10, 6, 8, 30), timed: true),
            durationMin: 70,
            calendar: 'personal',
            assignee: 'agent',
          ),
        );
        return page;
      }))!;
      await settle(tester);
      await openTestCalendar(tester);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyW);
      await settle(tester);
      // The Undo toast closes after six seconds of fake time; settle could
      // spend that while native writes are slow, so wait for it directly.
      await dragSidebarNote(
        tester,
        page,
        key('calendar-slot-2026-10-07-10'),
        settleAfter: false,
      );
      await pumpUntilShown(tester, find.text('Undo'));
      var task = (await tester.runAsync(() => loadTask(tv.vault, page)))!;
      expect(task.dateTime, DateTime(2026, 10, 7, 10));
      expect(task.durationMin, 70);
      expect(task.assignee, 'agent');
      Future<void> checkDayLinks() async {
        for (final day in [6, 7]) {
          final note = await tv.vault.run(
            (v) => v.dailyNote(
              dayMs: DateTime.utc(2026, 10, day).millisecondsSinceEpoch,
              title: '',
              create: false,
            ),
          );
          expect(note, isNotNull);
          final blocks = (await tv.vault.loadPage(note!)).blocks;
          expect(blocks.first.text, 'Tasks');
          expect(
            blocks.where((b) => b.kind == Kinds.linkToPage && b.target == page),
            hasLength(1),
          );
        }
      }

      await tester.runAsync(checkDayLinks);
      await tester.runAsync(
        () => tv.vault.typeDataSet(page, 'tasks', ['status'], '"doing"'),
      );
      await tester.tap(find.text('Undo'));
      await settle(tester);
      task = (await tester.runAsync(() => loadTask(tv.vault, page)))!;
      expect(task.dateTime, DateTime(2026, 10, 6, 8, 30));
      expect(task.status, 'doing');
      expect(task.durationMin, 70);
      await tester.runAsync(checkDayLinks);
    },
  );

  appTest('day hour accepts a plain note and keeps it an all-day mention', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final page = (await tester.runAsync(
      () => tv.vault.createPage(title: 'Day note'),
    ))!;
    await settle(tester);
    await openTestCalendar(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyD);
    await settle(tester);
    await dragSidebarNote(tester, page, key('calendar-slot-2026-10-06-10'));
    final data = (await tester.runAsync(() => tv.vault.loadPage(page)))!;
    expect(data.pageType, documentType);
    expect(data.blocks.last.marks.single.url, '2026-10-06');
    expect(find.text('Added to calendar.'), findsOneWidget);
  });

  appTest(
    'duplicate check includes nested marks and concurrent drops; undo preserves another paragraph',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      await tester.runAsync(() async {
        final page = await tv.vault.createPage(title: 'Concurrent note');
        final payload = NoteDragPayload(
          pageId: page,
          title: 'Concurrent note',
          pageType: documentType,
        );
        final results = await Future.wait([
          dropNoteOnCalendar(tv.vault, payload, DateTime(2026, 10, 7)),
          dropNoteOnCalendar(tv.vault, payload, DateTime(2026, 10, 7)),
        ]);
        expect(results.where((r) => r.duplicate).length, 1);
        await tv.vault.apply(page, [
          const BOp(
            op: OpKind.insert,
            id: 'later',
            kind: Kinds.paragraph,
            text: 'Another edit',
            marks: [],
            checked: false,
          ),
        ]);
        await results.firstWhere((r) => !r.duplicate).undo!();
        expect((await tv.vault.loadPage(page)).blocks.map((b) => b.text), [
          'Another edit',
        ]);
        await tv.vault.apply(page, [
          const BOp(
            op: OpKind.insert,
            id: 'nested-parent',
            kind: Kinds.toggle,
            text: 'Details',
            marks: [],
            checked: false,
          ),
          const BOp(
            op: OpKind.insert,
            id: 'nested-date',
            parent: 'nested-parent',
            kind: Kinds.paragraph,
            text: '@2026-10-07',
            marks: [
              BMark(start: 0, end: 11, kind: BMarkKind.date, url: '2026-10-07'),
            ],
            checked: false,
          ),
        ]);
        expect(
          (await dropNoteOnCalendar(
            tv.vault,
            payload,
            DateTime(2026, 10, 7),
          )).duplicate,
          isTrue,
        );
      });
    },
  );

  appTest('Undo rejects an appended paragraph with a newly added child', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    await tester.runAsync(() async {
      final page = await tv.vault.createPage(title: 'Edited addition');
      final result = await dropNoteOnCalendar(
        tv.vault,
        NoteDragPayload(pageId: page),
        DateTime(2026, 10, 7),
      );
      final added = (await tv.vault.loadPage(page)).blocks.last.id;
      await tv.vault.apply(page, [
        BOp(
          op: OpKind.insert,
          id: 'child-edit',
          parent: added,
          kind: Kinds.paragraph,
          text: 'Keep this child',
          marks: const [],
          checked: false,
        ),
      ]);
      await expectLater(result.undo!(), throwsStateError);
      final blocks = (await tv.vault.loadPage(page)).blocks;
      expect(blocks.map((b) => b.text), ['@2026-10-07', 'Keep this child']);
      expect(blocks.last.depth, 1);
    });
  });
}
