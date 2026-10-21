@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/state/agenda_store.dart';
import 'package:notes_app/core/models.dart';

import 'harness.dart';

Future<void> shot(String name) => expectLater(
  find.byType(Directionality).first,
  matchesGoldenFile('../screenshots/today-$name.png'),
);

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';
      appTest('Today home $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark, demo: true);
        await shot('$tag-home');
      });
      appTest('Today another day $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark, demo: true);
        await tapAndSettle(
          tester,
          find.byKey(const ValueKey('today-date-2026-10-08')),
        );
        await shot('$tag-other-day');
      });
      appTest('Today empty day $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark, empty: true);
        await shot('$tag-empty');
      });
      appTest('Today day with note $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark, empty: true);
        late String taskPage;
        final taskBlock = newId();
        final note = await core(tester, (v) async {
          taskPage = await createFixtureSpace(v, title: 'Garden plan');
          await v.apply(taskPage, [
            BOp(
              op: OpKind.insert,
              id: taskBlock,
              kind: Kinds.todo,
              text: 'Water seedlings @today',
              checked: false,
              marks: const [
                BMark(
                  start: 16,
                  end: 22,
                  kind: BMarkKind.date,
                  url: '2026-10-06',
                ),
              ],
            ),
          ]);
          final id = (await v.run(
            (r) => r.dailyNote(
              dayMs: dayStamp(fixedNow),
              title: 'Tuesday 6 October 2026',
              create: true,
            ),
            writes: true,
          ))!;
          final page = await v.loadPage(id);
          final log = page.blocks.firstWhere(
            (b) => b.kind == Kinds.toggle && b.text == 'Log',
          );
          final before = page.blocks.indexOf(log);
          await v.apply(id, [
            BOp(
              op: OpKind.insert,
              id: newId(),
              after: before > 0 ? page.blocks[before - 1].id : null,
              kind: Kinds.paragraph,
              text: 'A quiet morning to plan the garden.',
              checked: false,
              marks: const [],
            ),
          ]);
          return id;
        });
        await finishAsync(tester, current.app.agenda.reload());
        final paragraph = editableWith('A quiet morning to plan the garden.');
        await settleUntil(tester, () => paragraph.evaluate().isNotEmpty);
        expect(paragraph, findsOneWidget);
        await tester.ensureVisible(paragraph);
        await settle(tester);
        expect(
          (await pageOf(
            tester,
            note,
          )).blocks.any((b) => b.text == 'A quiet morning to plan the garden.'),
          isTrue,
        );
        expect(
          find.byKey(const ValueKey('today-note-mark-2026-10-06')),
          findsOneWidget,
        );
        final card = find.byKey(ValueKey('today-item-$taskPage/$taskBlock'));
        expect(card.hitTestable(), findsOneWidget);
        expect(paragraph.hitTestable(), findsOneWidget);
        expect(
          tester.getTopLeft(paragraph).dy,
          greaterThan(tester.getBottomLeft(card).dy),
          reason:
              'the editable day note is visible below the selected-day item',
        );
        await shot('$tag-day-with-note');
      });
    }
  }
}
