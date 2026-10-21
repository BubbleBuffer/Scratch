import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/state/agenda_store.dart';
import 'package:notes_app/ui/ui.dart' show StacksCheckbox;

import 'harness.dart';
import 'today_status_fixture.dart';

Finder row(AgendaItem item) =>
    find.byKey(ValueKey('today-item-${AgendaStore.itemKey(item)}'));
Finder check(AgendaItem item) =>
    find.descendant(of: row(item), matching: find.byType(StacksCheckbox));
Future<void> showItem(WidgetTester tester, AgendaItem item) async {
  while (row(item).evaluate().isEmpty &&
      find.byKey(const ValueKey('today-more')).evaluate().isNotEmpty) {
    await reveal(tester, find.byKey(const ValueKey('today-more')));
    await tapAndSettle(tester, find.byKey(const ValueKey('today-more')));
  }
  await reveal(tester, row(item));
}

Future<void> reveal(WidgetTester tester, Finder target) async {
  await Scrollable.ensureVisible(tester.element(target), alignment: .4);
  await tester.pump();
}

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';
      appTest(
        'Today short cards preserve real status completion and reopening $tag',
        (tester) async {
          final f = await pumpApp(
            tester,
            mobile: mobile,
            dark: dark,
            empty: true,
          );
          await seedTodayStatuses(tester);
          final agenda = f.app.agenda;
          AgendaItem byTitle(String title) => [
            ...agenda.items,
            ...agenda.overdueItems,
          ].firstWhere((item) => item.title == title);
          final queued = byTitle('Plan the launch');
          expect(
            (queued.status, queued.statusGroup, queued.statusColor),
            ('Queued', 'todo', 'red'),
          );
          final progress = byTitle('Review the design');
          expect(
            (progress.status, progress.statusGroup, progress.statusColor),
            ('Underway', 'progress', 'green'),
          );
          final overdue = byTitle('Book the room');
          expect(overdue.status, 'Underway');
          expect(row(overdue), findsNothing);
          await tapAndSettle(
            tester,
            find.byKey(const ValueKey('today-overdue')),
          );
          await showItem(tester, overdue);
          expect(
            tester.widget<StacksCheckbox>(check(overdue)).checked,
            isFalse,
          );
          for (final title in [
            'Bring the notebook',
            'Water seedlings',
            'Unassigned task',
          ]) {
            final item = byTitle(title);
            expect(
              (item.status, item.statusGroup, item.statusColor),
              (null, null, null),
            );
          }
          final delivered = byTitle('Send the brief');
          await showItem(tester, delivered);
          expect(
            tester.widget<StacksCheckbox>(check(delivered)).checked,
            isTrue,
          );
          await showItem(tester, queued);
          await tapAndSettle(tester, check(queued));
          final completed = byTitle('Plan the launch');
          expect(completed.done, isTrue);
          expect(
            (completed.status, completed.statusGroup, completed.statusColor),
            ('Delivered', 'complete', 'blue'),
          );
          await showItem(tester, completed);
          await tapAndSettle(tester, check(completed));
          final reopened = byTitle('Plan the launch');
          expect(reopened.done, isFalse);
          expect(
            (reopened.status, reopened.statusGroup, reopened.statusColor),
            ('Queued', 'todo', 'red'),
          );
          expect(tester.takeException(), isNull);
        },
      );
    }
  }
}
