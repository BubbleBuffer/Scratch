// Editor, home and sidebar interactions, checked against what reached the
// real core (a temporary vault per test).

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/getting_started.dart';

import 'harness.dart';

Future<void> key(WidgetTester tester, LogicalKeyboardKey k, {bool shift = false, bool ctrl = false}) async {
  if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  if (ctrl) await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(k);
  if (ctrl) await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await settle(tester);
}

/// Adds an empty paragraph after "Plain paragraph…" and focuses it.
Future<void> newBlockAfterPlain(WidgetTester tester) async {
  final b = editableStarting('Plain paragraph');
  await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
  await key(tester, LogicalKeyboardKey.enter);
}

Future<TestGesture> hoverMouse(WidgetTester tester, Finder target) async {
  final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
  await g.addPointer(location: Offset.zero);
  await g.moveTo(tester.getCenter(target));
  await settle(tester);
  return g;
}

void main() {
  setUpAll(initCore);

  const plain = 'Plain paragraph text under a large heading. Type / for commands.';

  group('desktop editor', () {
    appTest('Enter splits a block, Backspace at the start merges it back', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await focusAt(tester, editableWith(plain), 5);
      await key(tester, LogicalKeyboardKey.enter);
      final blocks = (await blocksOf(tester, s.showcaseId));
      final i = blocks.indexWhere((b) => b.id == 'sc03');
      expect(blocks[i].text, 'Plain');
      expect(blocks[i + 1].text, plain.substring(5));
      expect(blocks[i + 1].kind, Kinds.paragraph);
      // The caret is at the start of the new block.
      expect(editState(tester, editableWith(plain.substring(5))).widget.focusNode.hasFocus, isTrue);
      await key(tester, LogicalKeyboardKey.backspace);
      expect((await blockById(tester, s.showcaseId, 'sc03')).text, plain);
      expect((await blocksOf(tester, s.showcaseId)).length, 19);
    });

    appTest('Tab indents a list item and Shift+Tab outdents it', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await focusAt(tester, editableWith('Another top-level item'), 0);
      await key(tester, LogicalKeyboardKey.tab);
      expect((await blockById(tester, s.showcaseId, 'sc08')).depth, 1);
      await key(tester, LogicalKeyboardKey.tab, shift: true);
      expect((await blockById(tester, s.showcaseId, 'sc08')).depth, 0);
    });

    appTest('the / menu filters and turns the block into the chosen kind', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await newBlockAfterPlain(tester);
      await typeText(tester, '/');
      expect(find.byKey(const ValueKey('slash-menu')), findsOneWidget);
      expect(find.byKey(const ValueKey('slash-quote')), findsOneWidget);
      await typeText(tester, 'h2');
      expect(find.byKey(const ValueKey('slash-quote')), findsNothing);
      expect(find.byKey(const ValueKey('slash-heading2')), findsOneWidget);
      await key(tester, LogicalKeyboardKey.enter);
      expect(find.byKey(const ValueKey('slash-menu')), findsNothing);
      final blocks = (await blocksOf(tester, s.showcaseId));
      final i = blocks.indexWhere((b) => b.id == 'sc03');
      expect(blocks[i + 1].kind, Kinds.heading2);
      expect(blocks[i + 1].text, '');
      // Escape closes it; arrows move the active row.
      await typeText(tester, '/');
      await key(tester, LogicalKeyboardKey.escape);
      expect(find.byKey(const ValueKey('slash-menu')), findsNothing);
    });

    appTest('the / menu picks with the mouse', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await newBlockAfterPlain(tester);
      await typeText(tester, '/');
      await tester.tap(find.byKey(const ValueKey('slash-todo')));
      await settle(tester);
      final blocks = (await blocksOf(tester, s.showcaseId));
      expect(blocks[blocks.indexWhere((b) => b.id == 'sc03') + 1].kind, Kinds.todo);
    });

    appTest('markdown shortcuts turn blocks into their kinds', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final cases = {
        '# ': Kinds.heading1,
        '## ': Kinds.heading2,
        '### ': Kinds.heading3,
        '- ': Kinds.bulleted,
        '1. ': Kinds.numbered,
        '[] ': Kinds.todo,
        '> ': Kinds.toggle,
        '" ': Kinds.quote,
        '```': Kinds.code,
      };
      for (final e in cases.entries) {
        await newBlockAfterPlain(tester);
        await typeText(tester, '${e.key}x');
        final blocks = (await blocksOf(tester, s.showcaseId));
        final nb = blocks[blocks.indexWhere((b) => b.id == 'sc03') + 1];
        expect(nb.kind, e.value, reason: e.key);
        expect(nb.text, 'x', reason: e.key);
      }
      await newBlockAfterPlain(tester);
      await typeText(tester, '---');
      final blocks = (await blocksOf(tester, s.showcaseId));
      expect(blocks[blocks.indexWhere((b) => b.id == 'sc03') + 1].kind, Kinds.divider);
    });

    appTest('to-do checkboxes toggle with a tap', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      expect((await blockById(tester, s.showcaseId, 'sc15')).checked, isFalse);
      await tester.tap(find.byKey(const ValueKey('check-sc15')));
      await settle(tester);
      expect((await blockById(tester, s.showcaseId, 'sc15')).checked, isTrue);
      await tester.tap(find.byKey(const ValueKey('check-sc14')));
      await settle(tester);
      expect((await blockById(tester, s.showcaseId, 'sc14')).checked, isFalse);
    });

    appTest('Ctrl+B and the selection toolbar format the selection', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await focusAt(tester, editableWith(plain), 0, extent: 5);
      expect(find.byKey(const ValueKey('selection-toolbar')), findsOneWidget);
      await key(tester, LogicalKeyboardKey.keyB, ctrl: true);
      var marks = (await blockById(tester, s.showcaseId, 'sc03')).marks;
      expect(marks.any((m) => m.kind == BMarkKind.bold && m.start == 0 && m.end == 5), isTrue);
      await tester.tap(find.byKey(const ValueKey('fmt-italic')));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('fmt-strike')));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('fmt-code')));
      await settle(tester);
      marks = (await blockById(tester, s.showcaseId, 'sc03')).marks;
      for (final k in [BMarkKind.bold, BMarkKind.italic, BMarkKind.strike, BMarkKind.code]) {
        expect(marks.any((m) => m.kind == k && m.start == 0 && m.end == 5), isTrue, reason: '$k');
      }
      // Toggle bold off again.
      await key(tester, LogicalKeyboardKey.keyB, ctrl: true);
      marks = (await blockById(tester, s.showcaseId, 'sc03')).marks;
      expect(marks.any((m) => m.kind == BMarkKind.bold), isFalse);
      // Link through the toolbar's link box.
      await tester.tap(find.byKey(const ValueKey('fmt-link')));
      await settle(tester);
      final field = find.descendant(of: find.byKey(const ValueKey('link-field')), matching: find.byType(EditableText));
      await tester.enterText(field, 'https://example.com');
      await tester.testTextInput.receiveAction(TextInputAction.search);
      await settle(tester);
      marks = (await blockById(tester, s.showcaseId, 'sc03')).marks;
      expect(marks.any((m) => m.kind == BMarkKind.link && m.url == 'https://example.com' && m.end == 5), isTrue);
    });

    appTest('block menu from the drag handle moves and deletes blocks', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final g = await hoverMouse(tester, editableWith('Heading 1'));
      await g.moveTo(tester.getCenter(find.byKey(const ValueKey('grip-sc02'))));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('grip-sc02')));
      await settle(tester);
      expect(find.byKey(const ValueKey('block-menu')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('menu-down')));
      await settle(tester);
      var ids = (await blocksOf(tester, s.showcaseId)).map((b) => b.id).toList();
      expect(ids.indexOf('sc02'), ids.indexOf('sc03') + 1);
      await g.moveTo(tester.getCenter(editableWith('Heading 1')));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('grip-sc02')));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('menu-delete')));
      await settle(tester);
      ids = (await blocksOf(tester, s.showcaseId)).map((b) => b.id).toList();
      expect(ids.contains('sc02'), isFalse);
      await g.removePointer();
    });

    appTest('dragging the handle moves a block', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final g = await hoverMouse(tester, editableWith('Heading 2'));
      final grip = tester.getCenter(find.byKey(const ValueKey('grip-sc04')));
      await g.moveTo(grip);
      await settle(tester);
      await g.down(grip);
      await tester.pump();
      final target = tester.getTopLeft(editableStarting('This page has'));
      for (var i = 1; i <= 10; i++) {
        await g.moveTo(Offset.lerp(grip, target + const Offset(10, 4), i / 10)!);
        await tester.pump();
      }
      await g.up();
      await settle(tester);
      final ids = (await blocksOf(tester, s.showcaseId)).map((b) => b.id).toList();
      expect(ids.first, 'sc04');
      await g.removePointer();
    });

    appTest('edits persist, back returns home, reopening shows them', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await focusAt(tester, editableWith(plain), plain.length);
      await typeText(tester, ' Saved!');
      await tester.tap(find.byKey(const ValueKey('back')));
      await settle(tester);
      expect(find.byKey(const ValueKey('capture-field')), findsOneWidget);
      expect((await blockById(tester, s.showcaseId, 'sc03')).text, '$plain Saved!');
      await openShowcase(tester);
      expect(editableWith('$plain Saved!'), findsOneWidget);
    });

    appTest('Android back (system pop) returns to the list', (tester) async {
      await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await tester.binding.handlePopRoute();
      await settle(tester);
      expect(find.byKey(const ValueKey('capture-field')), findsOneWidget);
    });

    appTest('the big page opens and accepts typing far down', (tester) async {
      final s = await pumpApp(tester, mobile: false, big: true);
      await tester.tap(find.text('Big page').last);
      await settle(tester);
      expect(editableStarting('Block 1:'), findsOneWidget);
      // Only a screenful of rows is built.
      expect(find.byType(EditableText).evaluate().length, lessThan(80));
      await tester.dragUntilVisible(editableStarting('Block 300:'), find.byType(CustomScrollView).last, const Offset(0, -600));
      await settle(tester);
      final b = editableStarting('Block 300:');
      await focusAt(tester, b, 0);
      await typeText(tester, 'typed ');
      expect((await blocksOf(tester, s.bigId!))[299].text.startsWith('typed Block 300:'), isTrue);
    });

    appTest('an edit made outside the editor shows up in the open page', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await tester.dragUntilVisible(
          editableWith('The end of the showcase.'), find.byType(CustomScrollView).last, const Offset(0, -300));
      await settle(tester);
      final block = await blockById(tester, s.showcaseId, 'sc19');
      await core(tester, (v) => v.apply(s.showcaseId, [
            BOp(op: OpKind.setText, id: block.id, kind: block.kind, text: 'Changed elsewhere.', marks: const [], checked: false),
          ]));
      await settleUntil(tester, () => editableWith('Changed elsewhere.').evaluate().isNotEmpty);
      expect(editableWith('Changed elsewhere.'), findsOneWidget);
    });

    appTest('the page menu moves the page to the trash', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu')));
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu-delete')));
      expect(find.byKey(const ValueKey('page-title')), findsNothing);
      expect((await pageOf(tester, s.showcaseId)).inTrash, isTrue);
      expect(find.byKey(ValueKey('tree-${s.showcaseId}')), findsNothing);
    });
  });

  group('home', () {
    appTest('search filters the list through the core', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      expect(find.text('All pages · 9'), findsOneWidget);
      expect(find.byKey(const ValueKey('search-field')), findsNothing);
      await expandAllPages(tester);
      await tester.enterText(
          find.descendant(of: find.byKey(const ValueKey('search-field')), matching: find.byType(EditableText)), 'garden');
      await settle(tester);
      expect(find.text('All pages · 1'), findsOneWidget);
      expect(find.byKey(ValueKey('hit-${s.pages['Garden plan']}')), findsOneWidget);
      expect(find.byKey(ValueKey('hit-${s.showcaseId}')), findsNothing);
    });

    appTest('creating a page opens it with the title focused', (tester) async {
      await pumpApp(tester, mobile: false);
      await tester.tap(find.byKey(const ValueKey('new-page')));
      await settle(tester);
      final title = find.descendant(of: find.byKey(const ValueKey('page-title')), matching: find.byType(EditableText));
      expect(tester.widget<EditableText>(find.byKey(const ValueKey('page-title'))).focusNode.hasFocus, isTrue);
      await tester.enterText(find.byKey(const ValueKey('page-title')), 'Fresh page');
      await tester.pump(const Duration(milliseconds: 400));
      await tester.tap(find.byKey(const ValueKey('back')));
      await settle(tester);
      expect(title, findsNothing);
      final first = await core(tester, (v) => v.listPages(offset: 0, limit: 1));
      expect(first.first.title, 'Fresh page');
      expect(find.text('Fresh page'), findsWidgets);
    });

    appTest('a new vault gets the Getting started page', (tester) async {
      await pumpApp(tester, mobile: false);
      final id = await core(tester, (v) => createGettingStarted(v));
      final p = await pageOf(tester, id);
      expect(p.title, gettingStartedTitle);
      expect(p.blocks.length, greaterThan(3));
      await settleUntil(tester, () => find.byKey(ValueKey('tree-$id')).evaluate().isNotEmpty);
      expect(find.byKey(ValueKey('tree-$id')), findsOneWidget);
    });
  });

  group('sidebar', () {
    Finder row(String id) => find.byKey(ValueKey('tree-$id'));

    Future<TestGesture> hoverRow(WidgetTester tester, String id) async {
      final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await g.addPointer(location: Offset.zero);
      await g.moveTo(tester.getCenter(row(id)));
      await settle(tester, min: Duration.zero);
      return g;
    }

    appTest('shows the page tree and expands children lazily', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      for (final t in fixtureTitles) {
        expect(row(s.pages[t]!), findsOneWidget, reason: t);
      }
      expect(row(s.pages['Soups']!), findsNothing);
      await tapAndSettle(tester, find.byKey(ValueKey('tree-toggle-${s.pages['Recipes']}')));
      expect(row(s.pages['Soups']!), findsOneWidget);
      expect(row(s.pages['Breads']!), findsOneWidget);
      // A page without children has no chevron, so it never opens empty.
      expect(find.byKey(ValueKey('tree-toggle-${s.pages['Budget']}')), findsNothing);
      expect(find.text('No pages inside'), findsNothing);
      await tapAndSettle(tester, find.byKey(ValueKey('tree-toggle-${s.pages['Recipes']}')));
      expect(row(s.pages['Soups']!), findsNothing);
    });

    appTest('opens a page from the tree', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await tapAndSettle(tester, row(s.pages['Garden plan']!));
      expect(editableWith('Some text about garden plan.'), findsOneWidget);
      // And another one from the editor's sidebar.
      await tapAndSettle(tester, row(s.pages['Budget']!));
      expect(editableWith('Some text about budget.'), findsOneWidget);
    });

    appTest('the + button creates a page inside another', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final budget = s.pages['Budget']!;
      final g = await hoverRow(tester, budget);
      await tapAndSettle(tester, find.byKey(ValueKey('tree-add-$budget')));
      await g.removePointer();
      await tester.enterText(find.byKey(const ValueKey('page-title')), 'Taxes');
      await tester.pump(const Duration(milliseconds: 400));
      await tapAndSettle(tester, find.byKey(const ValueKey('back')));
      final kids = await core(tester, (v) => v.listChildren(budget));
      expect(kids.map((p) => p.title), ['Taxes']);
      expect(find.text('Taxes'), findsWidgets);
    });

    appTest('the page actions menu renames a page in place', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final id = s.pages['Garden plan']!;
      final g = await hoverRow(tester, id);
      await tapAndSettle(tester, find.byKey(ValueKey('tree-more-$id')));
      await g.removePointer();
      await tapAndSettle(tester, find.byKey(const ValueKey('tree-menu-rename')));
      final field = find.descendant(of: find.byKey(const ValueKey('tree-rename-field')), matching: find.byType(EditableText));
      await tester.enterText(field, 'Vegetable garden');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await settle(tester);
      expect((await pageOf(tester, id)).title, 'Vegetable garden');
      expect(find.descendant(of: row(id), matching: find.text('Vegetable garden')), findsOneWidget);
    });

    appTest('the page actions menu moves a page and its children to the trash', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final id = s.pages['Recipes']!;
      final g = await hoverRow(tester, id);
      await tapAndSettle(tester, find.byKey(ValueKey('tree-more-$id')));
      await g.removePointer();
      await tapAndSettle(tester, find.byKey(const ValueKey('tree-menu-trash')));
      expect(row(id), findsNothing);
      expect((await pageOf(tester, id)).inTrash, isTrue);
      final trash = await core(tester, (v) => v.listTrash());
      expect(trash.any((t) => t.page.id == id), isTrue);
    });
  });

  group('mobile editor', () {
    appTest('keyboard toolbar sits above the keyboard and edits the block', (tester) async {
      final s = await pumpApp(tester, mobile: true);
      await openShowcase(tester);
      await focusAt(tester, editableWith(plain), 0);
      await showKeyboard(tester);
      final bar = find.byKey(const ValueKey('kb-bullets'));
      expect(bar, findsOneWidget);
      // Bottom of the toolbar = top of the keyboard.
      expect(tester.getBottomLeft(bar).dy, closeTo(812 - keyboardHeight, 1));
      await tester.tap(bar);
      await settle(tester);
      expect((await blockById(tester, s.showcaseId, 'sc03')).kind, Kinds.bulleted);
      await tester.tap(find.byKey(const ValueKey('kb-more')));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('kb-indent')));
      await settle(tester);
      expect((await blockById(tester, s.showcaseId, 'sc03')).depth, 1);
      await tester.tap(find.byKey(const ValueKey('kb-outdent')));
      await settle(tester);
      expect((await blockById(tester, s.showcaseId, 'sc03')).depth, 0);
      await tester.tap(find.byKey(const ValueKey('kb-back')));
      await settle(tester);
      // Formatting row.
      await focusAt(tester, editableWith(plain), 0, extent: 5);
      await tester.tap(find.byKey(const ValueKey('kb-format')));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('kb-bold')));
      await settle(tester);
      expect((await blockById(tester, s.showcaseId, 'sc03')).marks.any((m) => m.kind == BMarkKind.bold && m.end == 5), isTrue);
    });

    appTest('Enter from the soft keyboard (a typed newline) splits the block', (tester) async {
      final s = await pumpApp(tester, mobile: true);
      await openShowcase(tester);
      await focusAt(tester, editableWith(plain), 5);
      await typeText(tester, '\n');
      expect((await blockById(tester, s.showcaseId, 'sc03')).text, 'Plain');
    });

    appTest('long press opens the block sheet: move down, turn into, delete', (tester) async {
      final s = await pumpApp(tester, mobile: true);
      await openShowcase(tester);
      await tester.longPress(editableWith('Heading 1'));
      await settle(tester);
      expect(find.byKey(const ValueKey('sheet-down')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('sheet-down')));
      await settle(tester);
      var ids = (await blocksOf(tester, s.showcaseId)).map((b) => b.id).toList();
      expect(ids.indexOf('sc02'), ids.indexOf('sc03') + 1);

      await tester.longPress(editableWith('Heading 1'));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('sheet-turn-into')));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('sheet-kind-quote')));
      await settle(tester);
      expect((await blockById(tester, s.showcaseId, 'sc02')).kind, Kinds.quote);

      await tester.longPress(editableWith('Heading 1'));
      await settle(tester);
      await tester.tap(find.byKey(const ValueKey('sheet-delete')));
      await settle(tester);
      ids = (await blocksOf(tester, s.showcaseId)).map((b) => b.id).toList();
      expect(ids.contains('sc02'), isFalse);
    });

    appTest('the sheet closes with the back gesture', (tester) async {
      await pumpApp(tester, mobile: true);
      await openShowcase(tester);
      await tester.longPress(editableWith('Heading 1'));
      await settle(tester);
      await tester.binding.handlePopRoute();
      await settle(tester);
      expect(find.byKey(const ValueKey('sheet-down')), findsNothing);
      expect(editableWith('Heading 1'), findsOneWidget);
    });
  });
}
