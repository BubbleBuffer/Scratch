// Golden screenshots of a kanban: the full board page and the same board
// embedded in a document, at 1440x900 (desktop) and 375x812 (phone), light
// and dark, with the real fonts and the fixed clock of the harness. Files:
// screenshots/kanban-<desktop|phone>-<light|dark>-<page|embed>.png.
// Regenerate with: flutter test --update-goldens test/addons/kanban/kanban_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/embed_block.dart';

import 'kanban_fixture.dart';

Future<void> shot(WidgetTester tester, String name) async {
  // Async embed loading can leave retained layers in the test capture. Paint
  // the whole app again before taking its golden, including the host header.
  tester.renderObject(find.byType(Directionality).first).markNeedsPaint();
  await tester.pump();
  await expectLater(
    find.byType(Directionality).first,
    matchesGoldenFile('../../../screenshots/$name.png'),
  );
}

/// The cards [seedBoard] puts in each column, by title.
const boardCards = {
  'To do': ['Garden plan', 'Recipes'],
  'Doing': ['Budget'],
  'Done': ['Reading notes', 'Travel checklist'],
};

/// Waits until the board has drawn, then checks it, so no screenshot is taken
/// of a board that is still loading. A phone shows about one column and may
/// not have built the others, so only the first column is checked there.
Future<void> expectLoaded(WidgetTester tester, {required bool mobile}) async {
  await settleUntil(
    tester,
    () => cardNamed('Garden plan').evaluate().isNotEmpty,
  );
  expectBoardLoaded(mobile ? {'To do': boardCards['To do']!} : boardCards);
}

/// The board every golden shows: a few cards per column, from the fixture's
/// notes, with one card in the last column.
Future<String> seedBoard(WidgetTester tester, TestVault f) => seedKanban(
  tester,
  title: 'Launch board',
  columns: {
    'To do': [f.pages['Garden plan']!, f.pages['Recipes']!],
    'Doing': [f.pages['Budget']!],
    'Done': [f.pages['Reading notes']!, f.pages['Travel checklist']!],
  },
);

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final form = mobile ? 'phone' : 'desktop';
      final theme = dark ? 'dark' : 'light';

      appTest('kanban $form $theme page', (tester) async {
        final f = await pumpApp(tester, mobile: mobile, dark: dark);
        final board = await seedBoard(tester, f);
        await openNote(tester, board, 'Launch board');
        await expectLoaded(tester, mobile: mobile);
        await shot(tester, 'kanban-$form-$theme-page');
      });

      appTest('kanban $form $theme embed', (tester) async {
        final f = await pumpApp(tester, mobile: mobile, dark: dark);
        final board = await seedBoard(tester, f);
        final host = await core(
          tester,
          (v) => v.createPage(title: 'Launch notes'),
        );
        await core(
          tester,
          (v) => v.apply(host, [
            const BOp(
              op: OpKind.insert,
              id: 'before',
              kind: Kinds.paragraph,
              text: 'This week we ship the launch.',
              marks: [],
              checked: false,
            ),
            const BOp(
              op: OpKind.insert,
              id: 'board',
              after: 'before',
              kind: embedKind,
              text: '',
              marks: [],
              checked: false,
            ),
            BOp(
              op: OpKind.setTarget,
              id: 'board',
              kind: embedKind,
              text: '',
              marks: const [],
              checked: false,
              target: board,
            ),
            const BOp(
              op: OpKind.insert,
              id: 'after',
              after: 'board',
              kind: Kinds.paragraph,
              text: 'Notes after the board.',
              marks: [],
              checked: false,
            ),
          ]),
        );
        await openNote(tester, host, 'Launch notes');
        // The board must be the embed's inline renderer, drawn with its cards.
        final embed = find.byKey(const ValueKey('embed-board'));
        expect(inlineBoardIn(embed), findsOneWidget);
        await expectLoaded(tester, mobile: mobile);
        await shot(tester, 'kanban-$form-$theme-embed');
      });
    }
  }
}
