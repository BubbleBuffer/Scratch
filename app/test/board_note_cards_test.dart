import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/database/api.dart' show RowCard;
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import 'package:notes_app/ui/ui.dart';

import 'db_harness.dart';

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    dbTest(
      'Board ${mobile ? 'phone' : 'desktop'} uses the real task card and keeps document RowCards',
      (tester) async {
        await pumpDb(tester, mobile: mobile);
        final task = await core(tester, (v) async {
          final id = await v.createPage(
            parent: db.projects,
            title: 'Review the launch',
            pageType: taskType,
          );
          await saveTask(v, id, const TaskData(date: '2026-10-08'));
          return id;
        });
        await openView(tester, 'Board', mobile: mobile);
        final surface = byKey('board-card-$task');
        await tester.ensureVisible(surface);
        await settle(tester);
        final card = find.descendant(
          of: surface,
          matching: find.byType(ShortItemCard),
        );
        expect(card, findsOneWidget);
        expect(
          find.descendant(of: surface, matching: find.text('8 Oct')),
          findsOneWidget,
        );
        expect(
          find.descendant(of: surface, matching: find.byType(RowCard)),
          findsNothing,
        );
        expect(find.byType(RowCard), findsWidgets);

        await tester.tap(
          find.descendant(of: surface, matching: find.byType(StacksCheckbox)),
        );
        await settle(tester);
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

  dbTest(
    'Board retains RowCard when an installed note type has no board builder',
    (tester) async {
      await pumpDb(tester, mobile: false);
      const id = 'board-test-ordinary';
      Widget otherPresentation(BuildContext context, NoteArgs args) =>
          const Text('Standalone card');
      noteTypeRegistry.register(
        NoteType(
          id: id,
          label: 'Ordinary',
          chip: otherPresentation,
          card: otherPresentation,
          fullPage: otherPresentation,
        ),
      );
      final page = await core(
        tester,
        (v) => v.createPage(
          parent: db.projects,
          title: 'Ordinary typed row',
          pageType: id,
        ),
      );
      await openView(tester, 'Board');
      final surface = byKey('board-card-$page');
      expect(
        find.descendant(of: surface, matching: find.byType(RowCard)),
        findsOneWidget,
      );
      expect(find.text('Standalone card'), findsNothing);
      expect(
        find.descendant(of: surface, matching: find.text('Ordinary typed row')),
        findsOneWidget,
      );

      // Installing a board presentation takes effect without reopening the
      // database, including a claim before persisted page metadata catches up.
      noteTypeRegistry.register(
        NoteType(
          id: 'board-test-claimed',
          label: 'Claimed',
          chip: otherPresentation,
          card: otherPresentation,
          boardCard: (context, args) => Text('Board card for ${args.title}'),
          fullPage: otherPresentation,
          claimsPage: (id) => id == page,
        ),
      );
      await settle(tester);
      expect(
        find.descendant(of: surface, matching: find.byType(RowCard)),
        findsNothing,
      );
      expect(
        find.descendant(
          of: surface,
          matching: find.text('Board card for Ordinary typed row'),
        ),
        findsOneWidget,
      );
    },
  );
}
