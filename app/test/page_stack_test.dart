import 'dart:async';
import 'dart:convert';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart';
import 'package:notes_app/shell/page_stack.dart' show PileGeometry;

import 'harness.dart';

Finder screen(String id) => find.byWidgetPredicate(
  (w) => w is PageScreen && w.pageId == id && w.snapshot == null,
);
PageEditorState editor(WidgetTester t, String id) =>
    t.state<PageEditorState>(screen(id));
PageStackNavigator sheets(WidgetTester t) =>
    PageStackScope.of(t.element(find.byType(HomeScreen)));
Future<void> open(WidgetTester t, String id, {bool stack = false}) async {
  final ctx = sheets(t).pages.isEmpty
      ? t.element(find.byType(HomeScreen))
      : editor(t, sheets(t).topId!).context;
  unawaited(
    openPage(ctx, id, current.app.pages.meta(id)?.title ?? '', stack: stack),
  );
  await settle(t);
}

Future<void> spine(WidgetTester t, String id) =>
    tapAndSettle(t, find.byKey(ValueKey('stack-spine-$id')));

void main() {
  setUpAll(initCore);
  appTest('an internal page link pushes a second sheet', (t) async {
    final s = await pumpApp(t, mobile: false);
    final a = s.pages['Recipes']!, b = s.pages['Budget']!;
    await open(t, a);
    editor(t, a).openPage(b);
    await settle(t);
    expect(sheets(t).pages.map(sheets(t).idOf), [a, b]);
    expect(screen(a), findsOneWidget);
    expect(
      t.getTopLeft(find.byKey(ValueKey('stack-sheet-$b'))),
      const Offset(240 + 12 + 58, 12),
    );
  });
  appTest('the rail replaces all page sheets', (t) async {
    final s = await pumpApp(t, mobile: false);
    await open(t, s.pages['Recipes']!);
    await open(t, s.pages['Budget']!, stack: true);
    final target = s.pages['Weekly review']!;
    await tapAndSettle(t, find.byKey(ValueKey('tree-$target')));
    expect(sheets(t).pages.map(sheets(t).idOf), [target]);
  });
  appTest('opening a page already on the stack pops to its existing editor', (
    t,
  ) async {
    final s = await pumpApp(t, mobile: false);
    final a = s.pages['Recipes']!, b = s.pages['Budget']!;
    await open(t, a);
    final original = editor(t, a);
    await open(t, b, stack: true);
    await open(t, a, stack: true);
    expect(sheets(t).pages.map(sheets(t).idOf), [a]);
    expect(editor(t, a), same(original));
    expect(s.app.ui.lastPage, a);
  });
  appTest('a fifth sheet drops the bottom sheet and keeps Home', (t) async {
    final s = await pumpApp(t, mobile: false);
    final ids = [
      'Recipes',
      'Budget',
      'Weekly review',
      'Garden plan',
      'Reading notes',
    ].map((n) => s.pages[n]!).toList();
    for (final id in ids) {
      await open(t, id, stack: true);
    }
    expect(sheets(t).pages.map(sheets(t).idOf), ids.skip(1));
    expect(screen(ids.first), findsNothing);
    expect(find.byType(HomeScreen), findsOneWidget);
  });
  appTest('the spine pops one level; the fan-out pops to any sheet', (
    t,
  ) async {
    final s = await pumpApp(t, mobile: false);
    final a = s.pages['Recipes']!,
        b = s.pages['Budget']!,
        c = s.pages['Weekly review']!;
    await open(t, a);
    await open(t, b, stack: true);
    await open(t, c, stack: true);
    // Only the sheet directly below the top shows a spine.
    expect(find.byKey(ValueKey('stack-spine-$a')), findsNothing);
    await spine(t, b);
    expect(sheets(t).pages.map(sheets(t).idOf), [a, b]);
    await open(t, c, stack: true);
    // A click on the paper edges lists every sheet, top first, and Home.
    await tapAndSettle(t, find.byKey(const ValueKey('stack-edge-0')));
    expect(find.byKey(const ValueKey('stack-fan')), findsOneWidget);
    final entries = [c, b, a, 'home'];
    final ys = [
      for (final id in entries)
        t.getTopLeft(find.byKey(ValueKey('stack-fan-entry-$id'))).dy,
    ];
    expect(ys, [...ys]..sort());
    // Nothing reflows while the list is open.
    expect(
      t.getTopLeft(find.byKey(ValueKey('stack-sheet-$c'))),
      const Offset(240 + 12 + 4 + 58, 12),
    );
    await tapAndSettle(t, find.byKey(ValueKey('stack-fan-entry-$a')));
    expect(find.byKey(const ValueKey('stack-fan')), findsNothing);
    expect(sheets(t).pages.map(sheets(t).idOf), [a]);
  });
  appTest('the fan-out opens on a resting pointer, Enter on the spine, and '
      'closes on Escape or a click outside', (t) async {
    final s = await pumpApp(t, mobile: false);
    final a = s.pages['Recipes']!, b = s.pages['Budget']!;
    await open(t, a);
    await open(t, b, stack: true);
    final fan = find.byKey(const ValueKey('stack-fan'));
    final g = await t.createGesture(kind: PointerDeviceKind.mouse);
    addTearDown(g.removePointer);
    await g.addPointer(location: const Offset(1300, 600));
    await g.moveTo(t.getCenter(find.byKey(ValueKey('stack-spine-$a'))));
    await t.pump(const Duration(milliseconds: 200));
    expect(fan, findsNothing);
    await t.pump(const Duration(milliseconds: 150));
    await t.pump();
    expect(fan, findsOneWidget);
    // Escape closes the list and leaves the pile as it was.
    await t.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(t);
    expect(fan, findsNothing);
    expect(sheets(t).pages.map(sheets(t).idOf), [a, b]);
    // Leaving before the delay opens nothing.
    await g.moveTo(const Offset(1300, 600));
    await t.pump(const Duration(milliseconds: 400));
    expect(fan, findsNothing);
    // Enter on the focused spine opens it; a click outside closes it.
    final spineFocus = Focus.of(
      t.element(find.byKey(ValueKey('stack-spine-$a'))),
    );
    spineFocus.requestFocus();
    await t.pump();
    await t.sendKeyEvent(LogicalKeyboardKey.enter);
    await settle(t);
    expect(fan, findsOneWidget);
    await tapAndSettle(t, find.byKey(const ValueKey('stack-fan-barrier')));
    expect(fan, findsNothing);
    expect(sheets(t).pages.map(sheets(t).idOf), [a, b]);
  });
  test('pile geometry at depth 1, 2, 3 and 5', () {
    // Depth 1: no spine, the 12 px desk margin.
    expect(PileGeometry.topLeft(1), 12);
    expect(PileGeometry.edges(1), 0);
    // Depth 2: the spine of the sheet below, no paper edges.
    expect(PileGeometry.edges(2), 0);
    expect(PileGeometry.spineLeft(2), 12);
    expect(PileGeometry.topLeft(2), 12 + 58);
    expect(PileGeometry.sheetLeft(1, 2), 12);
    // Depth 3: one paper edge.
    expect(PileGeometry.edges(3), 1);
    expect(PileGeometry.topLeft(3), 12 + 58 + 4);
    expect(PileGeometry.sheetLeft(1, 3), 12);
    expect(PileGeometry.sheetLeft(2, 3), 16);
    // Depth 5: three edges, the most there ever are.
    expect(PileGeometry.edges(5), 3);
    expect(PileGeometry.edges(9), 3);
    expect(PileGeometry.topLeft(5), 12 + 58 + 12);
    expect(
      [for (var i = 1; i <= 5; i++) PileGeometry.sheetLeft(i, 5)],
      [12, 16, 20, 24, 82],
    );
  });
  appTest('the pile on the desk at depth 1, 2 and 3; four pages at most', (
    t,
  ) async {
    final s = await pumpApp(t, mobile: false);
    final ids = [
      'Recipes',
      'Budget',
      'Weekly review',
      'Garden plan',
    ].map((n) => s.pages[n]!).toList();
    Rect sheet(String id) =>
        t.getRect(find.byKey(ValueKey('stack-sheet-$id')));
    await open(t, ids[0]);
    expect(sheet(ids[0]), const Rect.fromLTRB(240 + 12, 12, 1440, 900));
    expect(find.byKey(ValueKey('stack-spine-${ids[0]}')), findsNothing);
    expect(find.byKey(const ValueKey('stack-edge-0')), findsNothing);
    await open(t, ids[1], stack: true);
    expect(sheet(ids[1]).left, 240 + 12 + 58);
    expect(sheet(ids[1]).top, 12);
    expect(
      t.getRect(find.byKey(ValueKey('stack-spine-${ids[0]}'))),
      const Rect.fromLTWH(240 + 12, 12, 58, 900 - 12),
    );
    expect(find.byKey(const ValueKey('stack-edge-0')), findsNothing);
    await open(t, ids[2], stack: true);
    expect(sheet(ids[2]).left, 240 + 12 + 4 + 58);
    expect(sheet(ids[2]).width, greaterThanOrEqualTo(1440 - 240 - 12 - 58 - 12));
    expect(
      t.getTopLeft(find.byKey(ValueKey('stack-spine-${ids[1]}'))),
      const Offset(240 + 12 + 4, 12),
    );
    expect(
      t.getRect(find.byKey(const ValueKey('stack-edge-0'))),
      const Rect.fromLTWH(240 + 12, 12, 4, 900 - 12),
    );
    expect(find.byKey(const ValueKey('stack-edge-1')), findsNothing);
    await open(t, ids[3], stack: true);
    expect(sheet(ids[3]).left, 240 + 12 + 8 + 58);
    expect(find.byKey(const ValueKey('stack-edge-1')), findsOneWidget);
    // A fifth page drops the bottom one: the pile never passes four.
    await open(t, s.pages['Reading notes']!, stack: true);
    expect(sheets(t).pages.length, 4);
    expect(sheet(s.pages['Reading notes']!).left, 240 + 12 + 8 + 58);
  });
  appTest('Escape pops one sheet but a focused text field retains it', (
    t,
  ) async {
    final s = await pumpApp(t, mobile: false);
    final a = s.pages['Recipes']!, b = s.pages['Budget']!;
    await open(t, a);
    await open(t, b, stack: true);
    final ed = editor(t, b);
    ed.titleFocus.requestFocus();
    await t.pump();
    await t.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(t);
    expect(sheets(t).pages.length, 2);
    FocusManager.instance.primaryFocus?.unfocus();
    await t.pump();
    await t.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(t);
    expect(sheets(t).pages.map(sheets(t).idOf), [a]);
  });
  appTest('the close control clears the stack back to Home', (t) async {
    final s = await pumpApp(t, mobile: false);
    final a = s.pages['Recipes']!, b = s.pages['Budget']!;
    await open(t, a);
    await open(t, b, stack: true);
    await tapAndSettle(
      t,
      find.descendant(
        of: screen(b),
        matching: find.byKey(const ValueKey('back')),
      ),
    );
    expect(sheets(t).pages, isEmpty);
    expect(s.app.ui.lastPage, isNull);
  });
  appTest('covered editors ignore pointer input', (t) async {
    final s = await pumpApp(t, mobile: false);
    await open(t, s.showcaseId);
    final lower = editor(t, s.showcaseId);
    final title = lower.titleCtl.text;
    await open(t, s.pages['Budget']!, stack: true);
    final lowerTitle = find.descendant(
      of: screen(s.showcaseId),
      matching: find.byKey(const ValueKey('page-title')),
    );
    await t.tap(lowerTitle, warnIfMissed: false);
    await settle(t);
    expect(lower.titleFocus.hasFocus, isFalse);
    expect(lower.titleCtl.text, title);
  });
  appTest('covered editor state scroll and undo survive a push and pop', (
    t,
  ) async {
    final s = await pumpApp(t, mobile: false);
    await open(t, s.showcaseId);
    final lower = editor(t, s.showcaseId);
    final block = lower.doc!.byId['sc03']!;
    final before = lower.ctl(block).text;
    final editable = find.descendant(
      of: screen(s.showcaseId),
      matching: find.byWidgetPredicate(
        (w) => w is EditableText && w.controller == lower.ctl(block),
      ),
    );
    await t.enterText(editable, '$before Kept.');
    await settle(t);
    lower.scroll.jumpTo(lower.scroll.position.maxScrollExtent.clamp(0, 100));
    await t.pump();
    final offset = lower.scroll.offset;
    await open(t, s.pages['Budget']!, stack: true);
    expect(lower.scroll.offset, offset);
    await spine(t, s.showcaseId);
    expect(editor(t, s.showcaseId), same(lower));
    expect(lower.scroll.offset, offset);
    expect(lower.ctl(block).text, '$before Kept.');
    expect(lower.canUndo, isTrue);
    lower.undo();
    await settle(t);
    expect(lower.ctl(block).text, before);
  });
  appTest('a reminder reveals an offscreen block on its retained sheet', (
    t,
  ) async {
    final s = await pumpApp(t, mobile: false, big: true);
    final id = s.bigId!;
    await open(t, id);
    final original = editor(t, id);
    final target = original.doc!.flat.lastWhere((n) => n.kind == Kinds.todo);
    expect(original.rows.containsKey(target.id), isFalse);
    await open(t, s.pages['Budget']!, stack: true);
    final context = editor(t, s.pages['Budget']!).context;
    unawaited(
      openReminder(context, jsonEncode({'page': id, 'block': target.id})),
    );
    // Observe the temporary highlight frame by frame. Global quiescence can
    // advance its two-second lifetime while the native request is pending.
    final revealDeadline = Stopwatch()..start();
    while (original.highlightedBlock != target.id &&
        revealDeadline.elapsed < const Duration(seconds: 30)) {
      await t.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 1)),
      );
      await t.pump();
    }
    expect(original.highlightedBlock, target.id);
    expect(sheets(t).pages.map(sheets(t).idOf), [id]);
    expect(editor(t, id), same(original));
    expect(original.rows[target.id], isNotNull);
    expect(original.scroll.offset, greaterThan(0));
    final row = original.rows[target.id]!;
    final box = row.context.findRenderObject() as RenderBox;
    final origin = box.localToGlobal(Offset.zero);
    expect(origin.dy, greaterThanOrEqualTo(52));
    expect(origin.dy + box.size.height, lessThanOrEqualTo(900));
  });
  appTest('reduced motion pushes and pops sheets immediately', (t) async {
    t.platformDispatcher.accessibilityFeaturesTestValue =
        const FakeAccessibilityFeatures(disableAnimations: true);
    addTearDown(t.platformDispatcher.clearAccessibilityFeaturesTestValue);
    final s = await pumpApp(t, mobile: false);
    final a = s.pages['Recipes']!, b = s.pages['Budget']!;
    await open(t, a);
    unawaited(openPage(editor(t, a).context, b, 'Budget', stack: true));
    await t.pump();
    final route = sheets(t).pages.last as PageRoute;
    expect(route.transitionDuration, Duration.zero);
    expect(route.animation!.value, 1);
    expect(
      t.getTopLeft(find.byKey(ValueKey('stack-sheet-$b'))),
      const Offset(240 + 12 + 58, 12),
    );
    sheets(t).navigator!.pop();
    await t.pump();
    expect(sheets(t).pages.map(sheets(t).idOf), [a]);
    expect(
      t.getTopLeft(find.byKey(ValueKey('stack-sheet-$a'))),
      const Offset(240 + 12, 12),
    );
    await settle(t);
  });
}
