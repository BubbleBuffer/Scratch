import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/blocks/page_link.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import 'package:notes_app/shell/page_stack.dart';
import 'package:notes_app/shell/search_widgets.dart';
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/ui/board/board_renderer.dart';
import 'package:notes_app/ui/board/board_source.dart';
import 'package:notes_app/ui/note_preview.dart';
import 'package:notes_app/ui/ui.dart';

import 'board_renderer_test.dart' show MemoryBoardSource;
import 'harness.dart';
import 'search_hits_fixture.dart';

const line = 'Groceries for Sunday';

Future<String> groceries(WidgetTester tester) => core(tester, (v) async {
  final id = await v.createPage();
  await v.apply(id, [
    BOp(
      op: OpKind.insert,
      id: newId(),
      kind: Kinds.paragraph,
      text: line,
      marks: const [],
      checked: false,
    ),
  ]);
  return id;
});

Future<void> mount(WidgetTester tester, AppState state, Widget child) async {
  await tester.pumpWidget(
    AppScope(
      state: state,
      child: AppTheme(
        colors: AppColors.light,
        formFactor: FormFactor.desktop,
        child: WidgetsApp(
          color: const Color(0xFFFFFFFF),
          pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
            settings: settings,
            pageBuilder: (context, _, _) => builder(context),
          ),
          home: Center(child: SizedBox(width: 600, height: 400, child: child)),
        ),
      ),
    ),
  );
  await settle(tester);
}

void main() {
  test('display title prefers the title, then a first line, then Untitled', () {
    expect(displayTitle('Shopping', firstLine: line), 'Shopping');
    expect(displayTitle('  Shopping  ', firstLine: line), '  Shopping  ');
    expect(displayTitle(' \n\t', firstLine: line), line);
    expect(displayTitle('', firstLine: '  $line  '), line);
    expect(displayTitle(''), 'Untitled');
    expect(displayTitle(' \t', firstLine: '\n  '), 'Untitled');
    expect(displayTitle('', firstLine: 'Untitled'), 'Untitled');
  });

  test('stack labels share the rule and preserve first-line styling flags', () {
    final stack = PageStackNavigator();
    addTearDown(stack.dispose);
    stack.firstLines['page'] = 'Live body';
    expect(pageLabel(stack, 'page', 'Title', firstLine: line), (
      text: 'Title',
      firstLine: false,
    ));
    expect(pageLabel(stack, 'page', ' \t', firstLine: line), (
      text: 'Live body',
      firstLine: true,
    ));
    stack.firstLines.clear();
    expect(pageLabel(stack, 'page', '', firstLine: line), (
      text: line,
      firstLine: true,
    ));
    expect(pageLabel(stack, 'page', ''), (text: 'Untitled', firstLine: false));
  });

  setUpAll(initCore);

  appTest(
    'link chip and note preview show stored first lines with upright text',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false, empty: true);
      final id = await groceries(tester);
      await mount(
        tester,
        tv.app,
        Column(
          children: [
            PageLinkRow(pageId: id, fromPage: null),
            Builder(
              builder: (context) => noteChip(context, NoteArgs(pageId: id)),
            ),
          ],
        ),
      );
      await settleUntil(tester, () => find.text(line).evaluate().length == 2);
      expect(find.text(line), findsNWidgets(2));
      expect(find.text('Untitled'), findsNothing);
      for (final text in tester.widgetList<Text>(find.text(line))) {
        expect(text.style?.fontStyle, isNot(FontStyle.italic));
      }
    },
  );

  appTest('search ancestor path uses the stored first line', (tester) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final parent = await groceries(tester);
    final child = await core(
      tester,
      (v) => v.createPage(parent: parent, title: 'Shop list'),
    );
    await mount(
      tester,
      tv.app,
      SearchResultRow(hit: pageHit(child, 'Shop list'), onTap: () {}),
    );
    final path = find.byKey(ValueKey('hit-path-$child'));
    await settleUntil(
      tester,
      () =>
          path.evaluate().isNotEmpty && tester.widget<Text>(path).data == line,
    );
    expect(tester.widget<Text>(path).data, line);
    expect(tester.widget<Text>(path).style?.fontStyle, isNot(FontStyle.italic));
    expect(find.text('Untitled'), findsNothing);
  });

  appTest('board card uses stored first line without changing its style', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final id = await groceries(tester);
    final source = MemoryBoardSource();
    addTearDown(source.dispose);
    source.cards['todo'] = [BoardCard(id: id, title: '')];
    source.cards['done'] = [];
    await mount(
      tester,
      tv.app,
      BoardRenderer(source: source, columnWidth: 220),
    );
    final card = find.byKey(ValueKey('board-card-$id'));
    final title = find.descendant(of: card, matching: find.text(line));
    await settleUntil(tester, () => title.evaluate().isNotEmpty);
    expect(title, findsOneWidget);
    expect(
      tester.widget<Text>(title).style?.fontStyle,
      isNot(FontStyle.italic),
    );
    expect(
      find.descendant(of: card, matching: find.text('Untitled')),
      findsNothing,
    );
  });
}
