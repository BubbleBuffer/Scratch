import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/addons/tasks/calendar_reschedule.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/ui/database/values.dart';
import 'package:notes_app/ui/ui.dart' show Hover;

import '../../addon_harness.dart';

typedef RowFixture = ({String database, String row, String dateKey});

Future<RowFixture> seedRow(
  Vault vault, {
  required String title,
  required PropValue date,
  AgendaRole role = AgendaRole.events,
}) async {
  final database = await vault.createPage(title: 'Plans');
  await vault.run((v) => v.makeCollection(page: database), writes: true);
  final schema = await vault.run(
    (v) => v.setSchema(
      collection: database,
      schema: const [
        PropDef(
          key: 'other-date',
          name: 'Other date',
          kind: PropKind.date,
          options: [],
          config: '',
        ),
        PropDef(
          key: 'scheduled',
          name: 'Scheduled',
          kind: PropKind.date,
          options: [],
          config: '',
        ),
        PropDef(
          key: 'completed',
          name: 'Complete',
          kind: PropKind.checkbox,
          options: [],
          config: '',
        ),
        PropDef(
          key: 'memo',
          name: 'Memo',
          kind: PropKind.text,
          options: [],
          config: '',
        ),
      ],
    ),
    writes: true,
  );
  final dateKey = schema[1].key;
  await vault.run(
    (v) => v.setAgendaSettings(
      collection: database,
      settings: AgendaSettings(role: role, dateProp: dateKey),
    ),
    writes: true,
  );
  final row = await vault.createPage(parent: database, title: title);
  for (final property in [
    Property(
      key: schema[0].key,
      value: Values.date(DateTime.utc(2026, 9, 1).millisecondsSinceEpoch),
    ),
    Property(key: dateKey, value: date),
    Property(key: schema[2].key, value: Values.checkbox(true)),
    Property(key: schema[3].key, value: Values.text('Preserve this memo')),
  ]) {
    await vault.run(
      (v) => v.setProperty(page: row, key: property.key, value: property.value),
      writes: true,
    );
  }
  await vault.flush();
  return (database: database, row: row, dateKey: dateKey);
}

Future<List<AgendaItem>> calendarItems(Vault vault) => vault.calendarItems(
  timedFromMs: DateTime(2026).millisecondsSinceEpoch,
  timedToMs: DateTime(2027).millisecondsSinceEpoch,
  dayFromMs: DateTime.utc(2026).millisecondsSinceEpoch,
  dayToMs: DateTime.utc(2027).millisecondsSinceEpoch,
);

Future<AgendaItem> rowItem(Vault vault, RowFixture fixture) async =>
    (await calendarItems(vault)).singleWhere(
      (item) => item.page == fixture.row && item.collection == fixture.database,
    );

PropValue scheduled(PageData page, String key) =>
    page.properties.singleWhere((p) => p.key == key).value;

List<Object?> propertySnapshot(PageData page, {String? except}) => [
  for (final p in page.properties)
    if (p.key != except)
      [
        p.key,
        p.value.kind,
        p.value.text,
        p.value.number,
        p.value.startMs,
        p.value.endMs,
        p.value.hasTime,
        p.value.checked,
        p.value.ids,
      ],
];

List<Object?> blockSnapshot(PageData page) => [
  for (final b in page.blocks)
    [
      b.id,
      b.kind,
      b.text,
      b.checked,
      b.depth,
      b.target,
      b.color,
      b.width,
      [
        for (final m in b.marks) [m.start, m.end, m.kind, m.url],
      ],
    ],
];

void main() {
  setUpAll(initCore);

  for (final role in [AgendaRole.events, AgendaRole.tasks]) {
    appTest('database ${role.name} row drags to a day and persists its range', (
      tester,
    ) async {
      await pumpApp(tester, mobile: false, empty: true);
      final fixture = await core(
        tester,
        (v) => seedRow(
          v,
          title: 'Move database row',
          date: Values.date(
            DateTime(2026, 10, 6, 10, 30).millisecondsSinceEpoch,
            end: DateTime(2026, 10, 6, 12, 5).millisecondsSinceEpoch,
            hasTime: true,
          ),
          role: role,
        ),
      );
      final before = await core(tester, (v) => v.loadPage(fixture.row));
      unawaited(openCalendar(tester.element(find.byType(Hover).first)));
      await settle(tester);
      final drag = find
          .ancestor(
            of: find.text('Move database row').first,
            matching: find.byType(Draggable<AgendaItem>),
          )
          .first;
      final target = find.byKey(const ValueKey('calendar-day-2026-10-07-10'));
      await tester.dragFrom(
        tester.getCenter(drag),
        tester.getCenter(target) - tester.getCenter(drag),
      );
      await settle(tester);
      final after = await core(tester, (v) => v.loadPage(fixture.row));
      final date = scheduled(after, fixture.dateKey);
      expect(
        date.startMs,
        DateTime(2026, 10, 7, 10, 30).millisecondsSinceEpoch,
      );
      expect(date.endMs, DateTime(2026, 10, 7, 12, 5).millisecondsSinceEpoch);
      expect(date.hasTime, isTrue);
      expect(
        propertySnapshot(after, except: fixture.dateKey),
        propertySnapshot(before, except: fixture.dateKey),
      );
      expect(blockSnapshot(after), blockSnapshot(before));
      final item = await core(tester, (v) => rowItem(v, fixture));
      expect(item.startMs, date.startMs);
      expect(item.endMs, date.endMs);
      expect(
        item.kind,
        role == AgendaRole.tasks ? AgendaKind.task : AgendaKind.event,
      );
      expect(item.done, role == AgendaRole.tasks);
    });
  }

  appTest('day move retains all-day range and uses current date cell', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final result = await core(tester, (v) async {
      final fixture = await seedRow(
        v,
        title: 'Range',
        date: Values.date(
          DateTime.utc(2026, 10, 6).millisecondsSinceEpoch,
          end: DateTime.utc(2026, 10, 8).millisecondsSinceEpoch,
        ),
      );
      final item = await rowItem(v, fixture);
      // A concurrent edit completed after this drag payload was built.
      // Rescheduling must use the fresh cell, rather than the item's old end.
      await v.run(
        (r) => r.setProperty(
          page: fixture.row,
          key: fixture.dateKey,
          value: Values.date(
            DateTime.utc(2026, 10, 6).millisecondsSinceEpoch,
            end: DateTime.utc(2026, 10, 10).millisecondsSinceEpoch,
          ),
        ),
        writes: true,
      );
      await v.flush();
      await rescheduleCalendarItem(v, item, DateTime(2026, 10, 25));
      return scheduled(await v.loadPage(fixture.row), fixture.dateKey);
    });
    expect(result.startMs, DateTime.utc(2026, 10, 25).millisecondsSinceEpoch);
    expect(result.endMs, DateTime.utc(2026, 10, 29).millisecondsSinceEpoch);
    expect(result.hasTime, isFalse);
  });

  appTest('time slot move keeps row minutes, precision and elapsed duration', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final date = await core(tester, (v) async {
      final start = DateTime(
        2026,
        10,
        24,
        23,
        45,
        13,
        125,
      ).millisecondsSinceEpoch;
      final fixture = await seedRow(
        v,
        title: 'Overnight',
        date: Values.date(start, end: start + 90 * 60000, hasTime: true),
      );
      await rescheduleCalendarItem(
        v,
        await rowItem(v, fixture),
        DateTime(2026, 10, 26),
        hour: 14,
      );
      return scheduled(await v.loadPage(fixture.row), fixture.dateKey);
    });
    final expected = DateTime(
      2026,
      10,
      26,
      14,
      45,
      13,
      125,
    ).millisecondsSinceEpoch;
    expect(date.startMs, expected);
    expect(date.endMs, expected + 90 * 60000);
    expect(date.hasTime, isTrue);
  });

  appTest('stale drag rejects moved row and preserves all its properties', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    await core(tester, (v) async {
      final fixture = await seedRow(
        v,
        title: 'Moved row',
        date: Values.date(DateTime.utc(2026, 10, 6).millisecondsSinceEpoch),
      );
      final item = await rowItem(v, fixture);
      await v.run((r) => r.movePage(id: fixture.row), writes: true);
      final before = await v.loadPage(fixture.row);
      await expectLater(
        rescheduleCalendarItem(v, item, DateTime(2026, 10, 7)),
        throwsStateError,
      );
      expect(
        propertySnapshot(await v.loadPage(fixture.row)),
        propertySnapshot(before),
      );
    });
  });

  appTest('task day move preserves precision and unrelated type data', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    await core(tester, (v) async {
      final page = await v.createPage(title: 'Task', pageType: taskType);
      await saveTask(
        v,
        page,
        TaskData(
          date: taskDateString(
            DateTime(2026, 10, 6, 23, 30, 17, 125),
            timed: true,
          ),
          durationMin: 90,
          status: 'doing',
          calendar: 'personal',
          assignee: 'agent',
          checkbox: false,
        ),
      );
      await v.typeDataSet(page, taskAddonId, ['future'], '{"nested":"keep"}');
      final before = jsonDecode(await v.typeData(page, taskAddonId)) as Map;
      final item = (await calendarItems(v)).singleWhere((i) => i.page == page);
      await rescheduleCalendarItem(v, item, DateTime(2026, 10, 7));
      final after = jsonDecode(await v.typeData(page, taskAddonId)) as Map;
      expect(
        after['date'],
        taskDateString(DateTime(2026, 10, 7, 23, 30, 17, 125), timed: true),
      );
      before.remove('date');
      after.remove('date');
      expect(after, before);
    });
  });

  appTest(
    'todo and mention stay non-draggable and rejected writes preserve blocks',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final page = await core(tester, (v) async {
        final page = await v.createPage(title: 'References');
        const text = '😀 Plan @6 October 2026 suffix';
        final dateStart = text.indexOf('@');
        final dateEnd = text.indexOf(' suffix');
        final marks = [
          const BMark(start: 0, end: 2, kind: BMarkKind.bold, url: ''),
          BMark(
            start: dateStart,
            end: dateEnd,
            kind: BMarkKind.date,
            url: '2026-10-06',
          ),
          BMark(
            start: dateEnd + 1,
            end: text.length,
            kind: BMarkKind.italic,
            url: '',
          ),
        ];
        await v.apply(page, [
          BOp(
            op: OpKind.insert,
            id: 'todo',
            kind: Kinds.todo,
            text: text,
            marks: marks,
            checked: false,
          ),
          const BOp(
            op: OpKind.insert,
            id: 'child',
            parent: 'todo',
            kind: Kinds.paragraph,
            text: 'Keep nested child',
            marks: [],
            checked: false,
          ),
          BOp(
            op: OpKind.insert,
            id: 'mention',
            after: 'todo',
            kind: Kinds.paragraph,
            text: text,
            marks: marks,
            checked: false,
          ),
        ]);
        await v.flush();
        final before = await v.loadPage(page);
        final items = (await calendarItems(v))
            .where((i) => i.page == page)
            .toList();
        expect(items, hasLength(2));
        expect(
          items.map((i) => i.kind),
          containsAll([AgendaKind.task, AgendaKind.mention]),
        );
        for (final item in items) {
          expect(canRescheduleCalendarItem(item), isFalse);
          await expectLater(
            rescheduleCalendarItem(v, item, DateTime(2026, 10, 7)),
            throwsStateError,
          );
        }
        expect(blockSnapshot(await v.loadPage(page)), blockSnapshot(before));
        return page;
      });
      unawaited(openCalendar(tester.element(find.byType(Hover).first)));
      await settle(tester);
      expect(find.byType(Draggable<AgendaItem>), findsNothing);
      final blocks = await core(tester, (v) => v.loadPage(page));
      expect(
        blocks.blocks.any(
          (b) => b.id == 'child' && b.text == 'Keep nested child',
        ),
        isTrue,
      );
    },
  );
}
