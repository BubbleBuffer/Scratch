import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/ui/ui.dart';

import 'addons/kanban/kanban_fixture.dart';

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    appTest(
      'Kanban ${mobile ? 'phone' : 'desktop'} shows a real task checkbox and date',
      (tester) async {
        await pumpApp(tester, mobile: mobile, empty: true);
        final task = await core(tester, (v) async {
          final page = await v.createPage(
            title: 'Review the launch',
            pageType: taskType,
          );
          await saveTask(v, page, const TaskData(date: '2026-10-08'));
          return page;
        });
        final board = await seedKanban(
          tester,
          title: 'Task board',
          columns: {
            'To do': [task],
          },
        );
        await openNote(tester, board, 'Task board');
        final surface = find.byKey(ValueKey('board-card-$task'));
        final card = find.descendant(
          of: surface,
          matching: find.byType(ShortItemCard),
        );
        expect(card, findsOneWidget);
        expect(
          find.descendant(of: surface, matching: find.text('8 Oct')),
          findsOneWidget,
        );
        await tapAndSettle(
          tester,
          find.descendant(of: surface, matching: find.byType(StacksCheckbox)),
        );
        expect((await core(tester, (v) => loadTask(v, task))).done, isTrue);
        expect(
          surface,
          findsOneWidget,
          reason: 'completion must not open the note',
        );
        expect(tester.widget<ShortItemCard>(card).item.done, isTrue);
        expect(tester.takeException(), isNull);
      },
    );
  }
}
