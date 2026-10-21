// Creating kanban notes: the New kanban command makes a top-level board, and
// the /Kanban slash item in a document makes a child board and inserts a kit
// embed that points to it. Both run on the real vault and the real menus.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/registry/command_registry.dart';
import 'package:notes_app/registry/embed_block.dart';

import 'kanban_fixture.dart';

void main() {
  setUpAll(initCore);

  appTest(
    'the New kanban command makes a top-level kanban note and opens it with the starter columns',
    (tester) async {
      await pumpApp(tester, mobile: false);
      final before = await core(tester, (v) => v.listChildren(null));
      expect(before.where((p) => p.pageType == kanbanType), isEmpty);

      final command = commandRegistry.all.singleWhere(
        (c) => c.label == newKanbanLabel,
      );
      // Any widget under the app's Overlay gives the command its context; the
      // command runs as the menu would run it.
      expect(
        commandRegistry.run(
          command.id,
          tester.element(find.byType(Overlay).first),
        ),
        isTrue,
      );
      await settleUntil(
        tester,
        () => columnNamed('Doing').evaluate().isNotEmpty,
      );

      final after = await core(tester, (v) => v.listChildren(null));
      final created = after.where((p) => p.pageType == kanbanType).toList();
      expect(created, hasLength(1));
      expect(created.single.parent, isNull);

      final data = await storedKanban(tester, created.single.id);
      expect(storedTitles(data), starterColumns);
      expect(columnsOnScreen(tester), starterColumns);
      expect(storedCards(data, 'To do'), isEmpty);
      expect(storedCards(data, 'Doing'), isEmpty);
      expect(storedCards(data, 'Done'), isEmpty);
    },
  );

  for (final mobile in [false, true]) {
    appTest(
      '/Kanban in a document makes a child board and inserts an embed to it on ${mobile ? 'mobile' : 'desktop'}',
      (tester) async {
        await pumpApp(tester, mobile: mobile);
        final host = await core(
          tester,
          (v) => v.createPage(title: 'Sprint notes'),
        );
        await openNote(tester, host, 'Sprint notes');
        await focusAt(tester, editableWith('').last, 0);
        await typeText(tester, '/kanban');

        // The kanban slash item is a block type (kanbanInsertBlock), so its
        // menu row is keyed slash-kanban like any other block's.
        await tapAndSettle(tester, find.byKey(slashKanbanKey));
        await settleUntil(
          tester,
          () => columnNamed('To do').evaluate().isNotEmpty,
        );

        final children = await core(tester, (v) => v.listChildren(host));
        expect(children, hasLength(1));
        final child = children.single;
        expect(child.pageType, kanbanType);
        expect(child.parent, host);
        expect(
          storedTitles(await storedKanban(tester, child.id)),
          starterColumns,
        );

        final saved = await core(tester, (v) => v.loadPage(host));
        final embeds = saved.blocks.where((b) => b.kind == embedKind).toList();
        expect(embeds, hasLength(1));
        expect(embeds.single.target, child.id);
        expect(saved.blocks.where((b) => b.text.contains('/kanban')), isEmpty);
        expect(
          find.byKey(ValueKey('embed-${embeds.single.id}')),
          findsOneWidget,
        );
        for (final title in starterColumns) {
          expect(columnNamed(title), findsOneWidget);
        }
      },
    );
  }
}
