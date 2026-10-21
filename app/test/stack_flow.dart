// The flow from the UX review, as a person would do it on desktop: a new
// page, made a database; a new row, opened; a sub-page block in the row,
// opened; a link in it, opened with a pointer tap. Every page lands on top
// at full width and nothing below is squeezed. [step] runs after each step
// (a screenshot in the golden test).

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;

import 'db_harness.dart';
import 'harness.dart' as h show typeText;

/// The narrowest a top sheet may be at 1440 wide with a 240 rail: the desk
/// margin, one spine and the right margin's worth of edges.
const minTopWidth = 1440.0 - 240 - 12 - 58 - 12;

nav.PageStackNavigator stackOf(WidgetTester tester) => nav.PageStackScope.of(
  tester.element(find.byType(HomeScreen, skipOffstage: false)),
);

PageEditorState editorOf(WidgetTester tester, String id) =>
    tester.state<PageEditorState>(
      find.byWidgetPredicate(
        (w) => w is PageScreen && w.pageId == id && w.snapshot == null,
      ),
    );

void expectOnTop(WidgetTester tester, List<String> ids) {
  final stack = stackOf(tester);
  expect(stack.pages.map(stack.idOf).toList(), ids);
  final top = tester.getRect(find.byKey(ValueKey('stack-sheet-${ids.last}')));
  expect(top.width, greaterThanOrEqualTo(minTopWidth));
  expect(top.top, 12);
  expect(top.right, 1440);
  // Every sheet below is at least as wide as the top one: nothing squeezes.
  for (final id in ids.take(ids.length - 1)) {
    final r = tester.getRect(find.byKey(ValueKey('stack-sheet-$id')));
    expect(r.width, greaterThanOrEqualTo(top.width), reason: id);
    expect(r.left, lessThan(top.left), reason: id);
  }
}

Future<void> runStackFlow(
  WidgetTester tester,
  Future<void> Function(String step) step,
) async {
  // 1. A new page.
  final created = nav.createPage(tester.element(find.byType(HomeScreen)));
  await settle(tester);
  final page = stackOf(tester).topId!;
  expectOnTop(tester, [page]);
  expect(byKey('page-starters'), findsOneWidget);
  await step('1-page');

  // 2. Made a database from its starters.
  await tapKey(tester, 'start-database');
  await settleUntil(tester, () => byKey('db-new').evaluate().isNotEmpty);
  expectOnTop(tester, [page]);
  await step('2-database');

  // 3. A new row, opened on top of its database.
  await tapKey(tester, 'db-new');
  final row = stackOf(tester).topId!;
  expect(row, isNot(page));
  expectOnTop(tester, [page, row]);
  // Its title is ready for typing, as a new page's is.
  expect(editorOf(tester, row).titleFocus.hasFocus, isTrue);
  // Its breadcrumb leads back to the database.
  expect(
    find.descendant(
      of: byKey('stack-sheet-$row'),
      matching: byKey('crumb-$page'),
    ),
    findsOneWidget,
  );
  await step('3-row');

  // 4. A sub-page block in the row, opened on top.
  await tapKey(tester, 'start-blank');
  await h.typeText(tester, '/page');
  await tapKey(tester, 'slash-subpage');
  final child = stackOf(tester).topId!;
  expect(child, isNot(row));
  expectOnTop(tester, [page, row, child]);
  await step('4-sub-page');

  // 5. A link in it, opened on top.
  await tapKey(tester, 'start-blank');
  await h.typeText(tester, 'See [[Weekly');
  await tapKey(tester, 'mention-page-${await _id(tester, 'Weekly review')}');
  await settle(tester, min: const Duration(milliseconds: 500));
  // A real pointer tap on the inline mention.
  final ed = editorOf(tester, child);
  final block = ed.doc!.flat.firstWhere((n) => n.text.contains('Weekly'));
  final start = block.text.indexOf('Weekly');
  final re = ed.editableKey(block).currentState!.renderEditable;
  final boxes = re.getBoxesForSelection(
    TextSelection(baseOffset: start, extentOffset: start + 'Weekly'.length),
  );
  expect(boxes, isNotEmpty);
  await tester.tapAt(
    re.localToGlobal(boxes.first.toRect().center),
    kind: PointerDeviceKind.mouse,
  );
  await settle(tester);
  final linked = await _id(tester, 'Weekly review');
  expectOnTop(tester, [page, row, child, linked]);
  await step('5-link');

  stackOf(tester).clearImmediately();
  await settle(tester);
  await created;
}

Future<String> _id(WidgetTester tester, String title) async {
  final pages = await core(tester, (v) => v.listChildren(null));
  return pages.firstWhere((p) => p.title == title).id;
}
