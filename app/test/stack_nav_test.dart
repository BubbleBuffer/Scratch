// Navigation details on desktop: one empty state for a new page, no backlink
// from the parent's sub-page block alone, untitled pages named by their
// first line, Ctrl+K and Ctrl+P for search, `[[` for a page mention and leaf
// rows without a chevron.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';

Finder key(String k) => find.byKey(ValueKey(k));

Future<void> keyPress(
  WidgetTester tester,
  LogicalKeyboardKey k, {
  bool ctrl = false,
}) async {
  if (ctrl) await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(k);
  if (ctrl) await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await settle(tester);
}

PageStackNavigator sheets(WidgetTester t) =>
    PageStackScope.of(t.element(find.byType(HomeScreen)));

/// Makes a sub-page of the showcase with /page and returns its id; it is
/// open on top.
Future<String> newSubPage(WidgetTester tester, TestVault s) async {
  await openShowcase(tester);
  final b = editableStarting('Plain paragraph');
  await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
  await keyPress(tester, LogicalKeyboardKey.enter);
  await typeText(tester, '/page');
  await tapAndSettle(tester, key('slash-subpage'));
  final link = (await blocksOf(
    tester,
    s.showcaseId,
  )).firstWhere((x) => x.kind == Kinds.linkToPage);
  return link.target!;
}

void main() {
  setUpAll(initCore);

  appTest('a new page has one empty state and no backlink from its parent', (
    tester,
  ) async {
    final s = await pumpApp(tester, mobile: false);
    final id = await newSubPage(tester, s);
    expect(sheets(tester).topId, id);
    // Starters under the title, the cat below: nothing else offers a start.
    expect(key('page-starters'), findsOneWidget);
    for (final k in ['start-blank', 'start-database', 'template-add']) {
      expect(key(k), findsOneWidget, reason: k);
    }
    expect(key('template-offer'), findsNothing);
    expect(key('empty-page-cat'), findsOneWidget);
    expect(
      tester.getTopLeft(key('empty-page-cat')).dy,
      greaterThan(tester.getTopLeft(key('page-starters')).dy),
    );
    // The parent's sub-page block is the only link here: no "Linked from".
    expect(key('backlinks'), findsNothing);
    expect(find.text('Linked from'), findsNothing);
    // Writing something removes the whole empty state.
    await tapAndSettle(tester, key('start-blank'));
    await typeText(tester, 'Groceries for the week');
    await settle(tester, min: const Duration(milliseconds: 600));
    expect(key('page-starters'), findsNothing);
    expect(key('empty-page-cat'), findsNothing);
  });

  appTest('backlinks sit at the end of the page as "Linked from"', (
    tester,
  ) async {
    final s = await pumpApp(tester, mobile: false);
    await openShowcase(tester);
    final b = editableStarting('Plain paragraph');
    await focusAt(
      tester,
      b,
      editState(tester, b).widget.controller.text.length,
    );
    await typeText(tester, ' @Budg');
    await tapAndSettle(tester, key('mention-page-${s.pages['Budget']}'));
    await settle(tester, min: const Duration(milliseconds: 600));
    unawaited(
      openPage(
        tester.element(find.byType(HomeScreen)),
        s.pages['Budget']!,
        'Budget',
      ),
    );
    await settleUntil(tester, () => key('backlinks').evaluate().isNotEmpty);
    expect(find.text('Linked from'), findsOneWidget);
    expect(key('backlink-${s.showcaseId}'), findsOneWidget);
    // Below the body text, not in the header.
    expect(
      tester.getTopLeft(key('backlinks')).dy,
      greaterThan(
        tester.getTopLeft(editableWith('Some text about budget.')).dy,
      ),
    );
  });

  appTest('an untitled page shows its first line in tertiary italics', (
    tester,
  ) async {
    final s = await pumpApp(tester, mobile: false);
    final id = await newSubPage(tester, s);
    await tapAndSettle(tester, key('start-blank'));
    await typeText(tester, 'Groceries for the week');
    await settle(tester, min: const Duration(milliseconds: 600));
    // The rail row (once its parent is expanded) names it by its first line.
    if (key('tree-$id').evaluate().isEmpty) {
      await tapAndSettle(tester, key('tree-toggle-${s.showcaseId}'));
    }
    final labels = find.byKey(ValueKey('first-line-$id'));
    expect(
      find.descendant(of: key('tree-$id'), matching: labels),
      findsOneWidget,
    );
    final c = AppColors.light;
    for (final e in labels.evaluate()) {
      final t = e.widget as Text;
      expect(t.data, 'Groceries for the week');
      expect(t.style!.fontStyle, FontStyle.italic);
      expect(t.style!.color, anyOf(c.textTertiary, c.railMuted));
    }
    // So do its spine under the next sheet and that sheet's breadcrumb.
    final ed = tester.state<PageEditorState>(
      find.byWidgetPredicate(
        (w) => w is PageScreen && w.pageId == id && w.snapshot == null,
      ),
    );
    ed.openPage(s.pages['Budget']!);
    await settle(tester);
    expect(
      find.descendant(
        of: key('stack-spine-$id'),
        matching: find.text('Groceries for the week'),
      ),
      findsOneWidget,
    );
    for (final where in [
      key('sidebar'),
      key('stack-sheet-${s.pages['Budget']}'),
    ]) {
      expect(
        find.descendant(of: where, matching: find.text('Untitled')),
        findsNothing,
      );
    }
  });

  appTest('Ctrl+K and Ctrl+P open search from anywhere', (tester) async {
    final s = await pumpApp(tester, mobile: false);
    await keyPress(tester, LogicalKeyboardKey.keyK, ctrl: true);
    expect(key('search-overlay'), findsOneWidget);
    await keyPress(tester, LogicalKeyboardKey.escape);
    expect(key('search-overlay'), findsNothing);
    await keyPress(tester, LogicalKeyboardKey.keyP, ctrl: true);
    expect(key('search-overlay'), findsOneWidget);
    await keyPress(tester, LogicalKeyboardKey.escape);
    // From inside a page with a caret but no selection, too.
    await openShowcase(tester);
    final b = editableStarting('Plain paragraph');
    await focusAt(tester, b, 3);
    await keyPress(tester, LogicalKeyboardKey.keyK, ctrl: true);
    expect(key('search-overlay'), findsOneWidget);
    expect(key('link-field'), findsNothing);
    await tester.enterText(
      find.descendant(
        of: key('search-overlay-field'),
        matching: find.byType(EditableText),
      ),
      'Budget',
    );
    await settle(tester, min: const Duration(milliseconds: 400));
    await keyPress(tester, LogicalKeyboardKey.enter);
    expect(key('search-overlay'), findsNothing);
    expect(sheets(tester).topId, s.pages['Budget']);
  });

  appTest('Ctrl+K with a text selection adds a link instead', (tester) async {
    await pumpApp(tester, mobile: false);
    await openShowcase(tester);
    final b = editableStarting('Plain paragraph');
    await focusAt(tester, b, 0, extent: 5);
    await keyPress(tester, LogicalKeyboardKey.keyK, ctrl: true);
    expect(key('link-field'), findsOneWidget);
    expect(key('search-overlay'), findsNothing);
  });

  appTest('[[ picks a page and leaves a mention; Escape leaves the text', (
    tester,
  ) async {
    final s = await pumpApp(tester, mobile: false);
    await openShowcase(tester);
    final b = editableStarting('Plain paragraph');
    await focusAt(
      tester,
      b,
      editState(tester, b).widget.controller.text.length,
    );
    await typeText(tester, ' [[tod');
    // Pages only: no date suggestions.
    expect(
      find.byWidgetPredicate(
        (w) =>
            w.key is ValueKey<String> &&
            (w.key! as ValueKey<String>).value.startsWith('mention-date-'),
      ),
      findsNothing,
    );
    await keyPress(tester, LogicalKeyboardKey.escape);
    expect(editState(tester, b).widget.controller.text, endsWith(' [[tod'));
    // Remove the leftover and pick a page.
    for (var i = 0; i < 6; i++) {
      await keyPress(tester, LogicalKeyboardKey.backspace);
    }
    await typeText(tester, ' [[Budg');
    await tapAndSettle(tester, key('mention-page-${s.pages['Budget']}'));
    await settle(tester, min: const Duration(milliseconds: 600));
    final blk = await blockById(tester, s.showcaseId, 'sc03');
    expect(blk.text, endsWith(' Budget'));
    expect(blk.text, isNot(contains('[[')));
    final m = blk.marks.firstWhere((m) => m.kind == BMarkKind.pageLink);
    expect(m.url, s.pages['Budget']);
    expect(blk.text.substring(m.start, m.end), 'Budget');
  });

  appTest('leaf rows have no chevron and keep their indent', (tester) async {
    final s = await pumpApp(tester, mobile: false);
    final recipes = s.pages['Recipes']!, budget = s.pages['Budget']!;
    expect(key('tree-toggle-$recipes'), findsOneWidget);
    expect(key('tree-toggle-$budget'), findsNothing);
    expect(key('sidebar-new-page'), findsNothing);
    Finder icon(String id) => find
        .descendant(of: key('tree-$id'), matching: find.byType(PageIcon))
        .first;
    expect(
      tester.getTopLeft(icon(budget)).dx,
      tester.getTopLeft(icon(recipes)).dx,
    );
    await tapAndSettle(tester, key('tree-toggle-$recipes'));
    expect(find.text('No pages inside'), findsNothing);
  });
}
