// A folder exported by the real vault imports into a fresh vault with new
// page ids, while its kanban retains the cards' columns and order.
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/vault.dart';

import 'kanban_fixture.dart';

void main() {
  setUpAll(initCore);

  appTest(
    'folder import remaps kanban cards and displays their original columns and order',
    (tester) async {
      final sourceDir = Directory.systemTemp.createTempSync(
        'notes_kanban_import_source_',
      );
      addTearDown(() => sourceDir.deleteSync(recursive: true));
      late String originalBoard;
      late Map<String, String> originalCards;
      late String exported;
      await tester.runAsync(() async {
        final source = await Vault.open(sourceDir.path);
        try {
          originalBoard = await source.createPage(
            title: 'Alpha board',
            pageType: kanbanType,
          );
          originalCards = {
            for (final title in ['Zulu first', 'Zulu second', 'Zulu finished'])
              title: await source.createPage(title: title),
          };
          await source.typeDataSet(
            originalBoard,
            kanbanAddonId,
            ['columns'],
            jsonEncode([
              {'id': 'done', 'title': 'Done'},
              {'id': 'todo', 'title': 'To do'},
              {'id': 'doing', 'title': 'Doing'},
            ]),
          );
          await source.typeDataSet(
            originalBoard,
            kanbanAddonId,
            ['cards'],
            jsonEncode({
              'done': [originalCards['Zulu finished']],
              'todo': [
                originalCards['Zulu second'],
                originalCards['Zulu first'],
              ],
              'doing': <String>[],
            }),
          );
          await source.setTypeMarkdownBody(
            originalBoard,
            [
              headingLine('Done'),
              cardLinkLine(originalCards['Zulu finished']!, 'Zulu finished'),
              headingLine('To do'),
              cardLinkLine(originalCards['Zulu second']!, 'Zulu second'),
              cardLinkLine(originalCards['Zulu first']!, 'Zulu first'),
              headingLine('Doing'),
              '',
            ].join('\n'),
          );
          await source.flush();
          exported = await source.exportMarkdown(originalBoard);
          final boardFile = Directory('${sourceDir.path}/pages')
              .listSync()
              .whereType<File>()
              .singleWhere((f) => f.path.endsWith('-$originalBoard.md'));
          expect(
            await boardFile.readAsString(),
            exported,
            reason: 'import the vault writer output',
          );
        } finally {
          source.dispose();
        }
      });

      await pumpApp(tester, mobile: false, empty: true);
      expect(await core(tester, (v) => v.listChildren(null)), isEmpty);
      final report = await core(
        tester,
        (v) => v.run(
          (r) => r.importMarkdown(dir: '${sourceDir.path}/pages'),
          writes: true,
        ),
      );
      expect(report.pages, 4);
      expect(report.unresolvedLinks, 0);
      final rows = await core(tester, (v) => v.listChildren(null));
      final imported = {for (final row in rows) row.title: row.id};
      final board = imported['Alpha board']!;
      expect(board, isNot(originalBoard));
      for (final entry in originalCards.entries) {
        expect(imported[entry.key], isNot(entry.value));
      }
      final data = await storedKanban(tester, board);
      expect(storedTitles(data), ['Done', 'To do', 'Doing']);
      expect(storedCards(data, 'Done'), [imported['Zulu finished']]);
      expect(storedCards(data, 'To do'), [
        imported['Zulu second'],
        imported['Zulu first'],
      ]);
      expect(storedCards(data, 'Doing'), isEmpty);
      expect(
        await core(tester, (v) => v.typeMarkdownBody(board)),
        bodyOf(exported),
      );
      expect((await core(tester, (v) => v.loadPage(board))).blocks, isEmpty);

      await openNote(tester, board, 'Alpha board');
      await settleUntil(
        tester,
        () =>
            cardIn('Done', 'Zulu finished').evaluate().isNotEmpty &&
            cardIn('To do', 'Zulu first').evaluate().isNotEmpty &&
            cardIn('To do', 'Zulu second').evaluate().isNotEmpty,
      );
      expect(columnsOnScreen(tester), ['Done', 'To do', 'Doing']);
      expect(cardIn('Done', 'Zulu finished'), findsOneWidget);
      expect(cardSurfacesIn('Done'), findsOneWidget);
      expect(cardSurfacesIn('To do'), findsNWidgets(2));
      expect(cardSurfacesIn('Doing'), findsNothing);
      expect(
        tester.getTopLeft(cardIn('To do', 'Zulu second')).dy,
        lessThan(tester.getTopLeft(cardIn('To do', 'Zulu first')).dy),
      );
    },
  );
}
