import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/addons/tasks/title_dates.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/services/addon_services.dart' show openPage;
import 'package:notes_app/ui/ui.dart';

import '../../addon_harness.dart';
import 'task_fixture.dart';

Future<String> seedDatedNote(WidgetTester tester, List<String> dates) =>
    core(tester, (vault) async {
      final page = await vault.createPage(title: 'Dates in a note');
      String? after;
      for (var i = 0; i < dates.length; i++) {
        final id = 'date-$i';
        final text = '@${dates[i]}';
        await vault.apply(page, [
          BOp(
            op: OpKind.insert,
            id: id,
            after: after,
            kind: i == 1 ? Kinds.todo : Kinds.paragraph,
            text: text,
            marks: [
              BMark(
                start: 0,
                end: text.length,
                kind: BMarkKind.date,
                url: dates[i],
              ),
            ],
            checked: false,
          ),
        ]);
        after = id;
      }
      await vault.flush();
      return page;
    });

Future<void> openNote(WidgetTester tester, String page) async {
  final context = tester.element(find.byType(Hover).first);
  unawaited(openPage(context, page, 'Dates in a note'));
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  test('next day includes today and all-past dates select earliest', () {
    final dates = [
      DateTime(2026, 10, 9),
      DateTime(2026, 10, 1),
      DateTime(2026, 10, 6),
    ];
    expect(
      primaryNoteDate(dates, DateTime(2026, 10, 6, 20)),
      DateTime(2026, 10, 6),
    );
    expect(
      primaryNoteDate(dates, DateTime(2026, 10, 7)),
      DateTime(2026, 10, 9),
    );
    expect(primaryNoteDate(dates, DateTime(2026, 11)), DateTime(2026, 10, 1));
    expect(primaryNoteDate([], DateTime(2026)), isNull);
  });

  appTest('task date and time chip occupies the note title area', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final page = await core(tester, (vault) async {
      final id = await vault.createPage(
        title: 'Scheduled task',
        pageType: taskType,
      );
      await saveTask(
        vault,
        id,
        TaskData(
          date: taskDateString(DateTime(2026, 10, 9, 14, 30), timed: true),
        ),
      );
      return id;
    });
    await openNote(tester, page);
    expect(find.byType(NoteTitleChips), findsOneWidget);
    expect(
      find.descendant(
        of: find.byType(NoteTitleChips),
        matching: find.text('9 Oct · 14:30'),
      ),
      findsOneWidget,
    );
    await tester.tap(find.byKey(const ValueKey('note-title-date')));
    await settle(tester);
    expect(find.byType(VaultCalendarScreen), findsOneWidget);
    expect(current.app.agenda.selectedDay, DateTime(2026, 10, 9));
  });

  appTest(
    'mentions and dated todos show next upcoming +N with date navigation',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final page = await seedDatedNote(tester, [
        '2026-10-01',
        '2026-10-09',
        '2026-10-12',
        '2026-10-09',
      ]);
      await openNote(tester, page);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('note-title-date')),
          matching: find.text('9 Oct'),
        ),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('note-title-more-dates')),
        findsOneWidget,
      );
      expect(find.text('+2'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('note-title-more-dates')));
      await settle(tester);
      expect(
        find.byKey(const ValueKey('title-date-option-2026-10-01')),
        findsOneWidget,
      );
      await tester.tap(
        find.byKey(const ValueKey('title-date-option-2026-10-12')),
      );
      await settle(tester);
      expect(find.byType(VaultCalendarScreen), findsOneWidget);
      expect(current.app.agenda.selectedDay, DateTime(2026, 10, 12));
    },
  );

  appTest('all-past note chip shows earliest date', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final page = await seedDatedNote(tester, ['2026-10-04', '2026-10-01']);
    await openNote(tester, page);
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('note-title-date')),
        matching: find.text('1 Oct'),
      ),
      findsOneWidget,
    );
    expect(find.text('+1'), findsOneWidget);
  });

  appTest(
    'a first date added while a note is open appears and removal clears it',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final page = await seedDatedNote(tester, []);
      await openNote(tester, page);
      expect(find.byKey(const ValueKey('note-title-date')), findsNothing);
      await core(tester, (vault) async {
        await vault.apply(page, [
          const BOp(
            op: OpKind.insert,
            id: 'new-date',
            kind: Kinds.paragraph,
            text: '@2026-10-09',
            marks: [
              BMark(start: 0, end: 11, kind: BMarkKind.date, url: '2026-10-09'),
            ],
            checked: false,
          ),
        ]);
        await vault.flush();
      });
      await settle(tester);
      expect(find.byKey(const ValueKey('note-title-date')), findsOneWidget);
      await core(tester, (vault) async {
        await vault.apply(page, [
          const BOp(
            op: OpKind.delete,
            id: 'new-date',
            kind: '',
            text: '',
            marks: [],
            checked: false,
          ),
        ]);
        await vault.flush();
      });
      await settle(tester);
      expect(find.byKey(const ValueKey('note-title-date')), findsNothing);
    },
  );

  appTest('title providers preserve registration order and skip null widgets', (
    tester,
  ) async {
    final fixture = await pumpApp(tester, mobile: false, empty: true);
    final id = await core(
      tester,
      (vault) => vault.createPage(title: 'Generic slots'),
    );
    final page = await core(tester, (vault) => vault.loadPage(id));
    final registry = AddonContext();
    registry.titleChips.register(
      AddonTitleChip(id: 'first', build: (_, _) => const Text('First chip')),
    );
    registry.titleChips.register(
      AddonTitleChip(id: 'empty', build: (_, _) => null),
    );
    registry.titleChips.register(
      AddonTitleChip(id: 'last', build: (_, _) => const Text('Last chip')),
    );
    await pumpTaskHost(
      tester,
      fixture,
      NoteTitleChips(page: page, providers: registry.titleChips),
    );
    expect(
      tester.getTopLeft(find.text('First chip')).dx,
      lessThan(tester.getTopLeft(find.text('Last chip')).dx),
    );
    final disabled = AddonContext();
    await pumpTaskHost(
      tester,
      fixture,
      NoteTitleChips(page: page, providers: disabled.titleChips),
    );
    expect(find.text('First chip'), findsNothing);
    expect(disabled.titleChips.all, isEmpty);
  });
}
