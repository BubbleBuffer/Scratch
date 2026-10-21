// Widget tests of the Phase 2 page features on the real core: page-wide
// undo, multi-block selection, toggles, columns, sub-pages, mentions,
// colours, clipboard markdown, the page header (icon, cover), the page menu
// (lock, small text, full width, duplicate) and media blocks.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/clipboard.dart';
import 'package:notes_app/features/showcase.dart';
import 'package:notes_app/registry/block_registry.dart';

import 'harness.dart';
import 'pages_fixture.dart';

String? _clip;

void _mockClipboard(WidgetTester tester) {
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
    switch (call.method) {
      case 'Clipboard.setData':
        _clip = (call.arguments as Map)['text'] as String?;
        return null;
      case 'Clipboard.getData':
        return _clip == null ? null : {'text': _clip};
      case 'Clipboard.hasStrings':
        return {'value': _clip != null};
    }
    return null;
  });
  addTearDown(() => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(SystemChannels.platform, null));
}

Future<void> _key(WidgetTester tester, LogicalKeyboardKey k, {bool ctrl = false, bool shift = false}) async {
  if (ctrl) await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(k);
  if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  if (ctrl) await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await settle(tester);
}

Future<List<String>> _texts(WidgetTester tester, String pageId) async =>
    [for (final b in await blocksOf(tester, pageId)) b.text];

/// The open page's title as its field shows it.
String _title(WidgetTester tester) => tester.widget<EditableText>(find.byElementPredicate((e) => e.widget.key == const ValueKey('page-title') && ModalRoute.of(e)?.isCurrent == true)).controller.text;

void main() {
  setUpAll(initCore);
  setUp(() => _clip = null);

  group('clipboard markdown', () {
    test('blocks round-trip through markdown', () {
      final md = [
        '# Title',
        'Plain **bold** and *italic* and `code`',
        '- one',
        '  - nested',
        '1. first',
        '- [x] done',
        '- [ ] open',
        '▸ Toggle',
        '  Inside the toggle',
        '```',
        'let x = 1;',
        '```',
        '---',
        '![Caption](attachment:abc)',
        '[https://example.com](https://example.com)',
      ].join('\n');
      final nodes = markdownToNodes(md);
      final kinds = [for (final n in nodes) n.kind];
      expect(kinds, [
        Kinds.heading1, Kinds.paragraph, Kinds.bulleted, Kinds.numbered, Kinds.todo, Kinds.todo, //
        Kinds.toggle, Kinds.code, Kinds.divider, Kinds.image, Kinds.bookmark,
      ]);
      expect(nodes[1].text, 'Plain bold and italic and code');
      expect(nodes[1].marks.map((m) => m.kind).toSet(), {BMarkKind.bold, BMarkKind.italic, BMarkKind.code});
      expect(nodes[2].children.single.text, 'nested');
      expect(nodes[4].checked, isTrue);
      expect(nodes[6].children.single.text, 'Inside the toggle');
      expect(nodes[7].text, 'let x = 1;');
      expect(nodes[9].target, 'abc');
      expect(nodes[9].text, 'Caption');
      expect(nodes[10].target, 'https://example.com');
      // And back.
      final again = markdownToNodes(nodesToMarkdown(nodes));
      expect([for (final n in again) n.kind], kinds);
      expect([for (final n in again) n.text], [for (final n in nodes) n.text]);
    });
  });

  group('desktop page', () {
    appTest('undo and redo cover typing and structure across blocks', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final b = editableStarting('Plain paragraph');
      final len = editState(tester, b).widget.controller.text.length;
      await focusAt(tester, b, len);
      await typeText(tester, ' More');
      await _key(tester, LogicalKeyboardKey.enter);
      await typeText(tester, 'New block');
      var blocks = await blocksOf(tester, s.showcaseId);
      var t = [for (final b in blocks) b.text];
      expect(t, contains('New block'));
      final newBlockId = blocks.singleWhere((b) => b.text == 'New block').id;
      // Typing coalesces per block over a second of real time, so a slow
      // run may take more than one Ctrl+Z per word.
      Future<void> undoUntil(bool Function(List<String>) done) async {
        for (var i = 0; i < 10 && !done(t); i++) {
          await _key(tester, LogicalKeyboardKey.keyZ, ctrl: true);
          blocks = await blocksOf(tester, s.showcaseId);
          t = [for (final b in blocks) b.text];
        }
        expect(done(t), isTrue);
      }

      // Undo the typing, then the split, then the first typing.
      // Slow per-character typing may undo to "N" or "Ne" before empty.
      // Require this block to remain present until all its typing is undone.
      await undoUntil((_) => blocks.any((b) => b.id == newBlockId && b.text.isEmpty));
      expect(t.where((x) => x.isEmpty).length, 2); // the divider and the new block
      await _key(tester, LogicalKeyboardKey.keyZ, ctrl: true);
      blocks = await blocksOf(tester, s.showcaseId);
      t = [for (final b in blocks) b.text];
      expect(blocks.any((b) => b.id == newBlockId), isFalse);
      expect(t.where((x) => x.isEmpty).length, 1); // only the divider
      expect(t.any((x) => x.endsWith('commands. More')), isTrue);
      await undoUntil((t) => t.any((x) => x.endsWith('commands.')));
      // Redo everything, with both shortcuts.
      for (var i = 0; i < 20 && !t.contains('New block'); i++) {
        await _key(tester, i.isEven ? LogicalKeyboardKey.keyZ : LogicalKeyboardKey.keyY, ctrl: true, shift: i.isEven);
        t = await _texts(tester, s.showcaseId);
      }
      expect(t, contains('New block'));
    });

    appTest('undo brings back a deleted block with its children', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await focusAt(tester, editableStarting('Bulleted list item'), 0);
      await _key(tester, LogicalKeyboardKey.escape);
      await _key(tester, LogicalKeyboardKey.backspace);
      var ids = [for (final b in await blocksOf(tester, s.showcaseId)) b.id];
      expect(ids, isNot(contains('sc05')));
      expect(ids, isNot(contains('sc07')));
      await _key(tester, LogicalKeyboardKey.keyZ, ctrl: true);
      final blocks = await blocksOf(tester, s.showcaseId);
      ids = [for (final b in blocks) b.id];
      expect(ids, containsAll(['sc05', 'sc06', 'sc07']));
      expect(blocks.firstWhere((b) => b.id == 'sc07').depth, 2);
    });

    appTest('shift-click selects a range; Delete removes it; Ctrl+Shift+Down moves it', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await focusAt(tester, editableStarting('First step'), 0);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tap(editableStarting('Second step'));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await settle(tester);
      // sc10, sc11 and its child sc12 are selected; move them below sc13.
      await _key(tester, LogicalKeyboardKey.arrowDown, ctrl: true, shift: true);
      var ids = [for (final b in await blocksOf(tester, s.showcaseId)) b.id];
      expect(ids.indexOf('sc13'), lessThan(ids.indexOf('sc10')));
      expect(ids.indexOf('sc11'), ids.indexOf('sc10') + 1);
      await _key(tester, LogicalKeyboardKey.delete);
      ids = [for (final b in await blocksOf(tester, s.showcaseId)) b.id];
      expect(ids, isNot(contains('sc10')));
      expect(ids, isNot(contains('sc12')));
      expect(ids, contains('sc13'));
    });

    appTest('Ctrl+C copies selected blocks as markdown; Ctrl+V pastes them as blocks', (tester) async {
      _mockClipboard(tester);
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await focusAt(tester, editableStarting('Seed the vault'), 0);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tap(editableStarting('Compare both'));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await settle(tester);
      await _key(tester, LogicalKeyboardKey.keyC, ctrl: true);
      expect(_clip, '- [x] Seed the vault\n- [ ] Compare both prototypes');
      final b = editableStarting('Plain paragraph');
      await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
      await _key(tester, LogicalKeyboardKey.enter);
      await _key(tester, LogicalKeyboardKey.keyV, ctrl: true);
      final blocks = await blocksOf(tester, s.showcaseId);
      final i = blocks.indexWhere((x) => x.text.startsWith('Plain paragraph'));
      expect(blocks[i + 1].kind, Kinds.todo);
      expect(blocks[i + 1].text, 'Seed the vault');
      expect(blocks[i + 1].checked, isTrue);
      expect(blocks[i + 2].text, 'Compare both prototypes');
    });

    appTest('the block menu colours a block', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await g.addPointer(location: Offset.zero);
      await g.moveTo(tester.getCenter(editableStarting('Bulleted list item')));
      await settle(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('grip-sc05')));
      await tapAndSettle(tester, find.byKey(const ValueKey('menu-color')));
      await tapAndSettle(tester, find.byKey(const ValueKey('color-red')));
      expect((await blockById(tester, s.showcaseId, 'sc05')).color, 'red');
      await g.removePointer();
    });

    appTest('Ctrl+U underlines the selection', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await focusAt(tester, editableStarting('Plain paragraph'), 0, extent: 5);
      await _key(tester, LogicalKeyboardKey.keyU, ctrl: true);
      final b = await blockById(tester, s.showcaseId, 'sc03');
      expect(b.marks.any((m) => m.kind == BMarkKind.underline && m.start == 0 && m.end == 5), isTrue);
    });

    appTest('"> " makes a toggle; Enter in an open toggle adds its first child', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final b = editableStarting('Plain paragraph');
      await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
      await _key(tester, LogicalKeyboardKey.enter);
      await typeText(tester, '> Details');
      var blocks = await blocksOf(tester, s.showcaseId);
      final tg = blocks.firstWhere((x) => x.text == 'Details');
      expect(tg.kind, Kinds.toggle);
      await tapAndSettle(tester, find.byKey(ValueKey('toggle-${tg.id}')));
      await focusAt(tester, editableWith('Details'), 'Details'.length);
      await _key(tester, LogicalKeyboardKey.enter);
      await typeText(tester, 'hidden');
      blocks = await blocksOf(tester, s.showcaseId);
      final ti = blocks.indexWhere((x) => x.id == tg.id);
      expect(blocks[ti + 1].text, 'hidden');
      expect(blocks[ti + 1].depth, tg.depth + 1);
      // Closing the toggle hides the child.
      await tapAndSettle(tester, find.byKey(ValueKey('toggle-${tg.id}')));
      expect(editableWith('hidden'), findsNothing);
    });

    appTest('/columns makes a two-column list with the caret in the first column', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final b = editableStarting('Plain paragraph');
      await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
      await _key(tester, LogicalKeyboardKey.enter);
      await typeText(tester, '/columns');
      await tapAndSettle(tester, find.byKey(const ValueKey('slash-columns2')));
      await typeText(tester, 'Left');
      final blocks = await blocksOf(tester, s.showcaseId);
      final li = blocks.indexWhere((x) => x.kind == Kinds.columnList);
      final d = blocks[li].depth;
      final cols = [for (final x in blocks.skip(li + 1).takeWhile((x) => x.depth > d)) if (x.depth == d + 1) x];
      expect(cols.length, 2);
      expect(cols.every((c) => c.kind == Kinds.column), isTrue);
      expect(blocks[li + 1].kind, Kinds.column);
      expect(blocks[li + 2].text, 'Left');
      expect(blocks[li + 2].depth, d + 2);
    });

    appTest('/page creates a sub-page, links it and opens it', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final b = editableStarting('Plain paragraph');
      await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
      await _key(tester, LogicalKeyboardKey.enter);
      await typeText(tester, '/page');
      await tapAndSettle(tester, find.byKey(const ValueKey('slash-subpage')));
      expect(_title(tester), isEmpty); // the new page is open
      final link = (await blocksOf(tester, s.showcaseId)).firstWhere((x) => x.kind == Kinds.linkToPage);
      final child = await pageOf(tester, link.target!);
      expect(child.parent, s.showcaseId);
    });

    appTest('@ inserts a page mention that opens the page', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final b = editableStarting('Plain paragraph');
      await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
      await typeText(tester, ' @Budg');
      await settle(tester);
      await tapAndSettle(tester, find.byKey(ValueKey('mention-page-${s.pages['Budget']}')));
      final blk = await blockById(tester, s.showcaseId, 'sc03');
      expect(blk.text.endsWith(' Budget'), isTrue);
      final m = blk.marks.firstWhere((m) => m.kind == BMarkKind.pageLink);
      expect(m.url, s.pages['Budget']);
      // Backlinks on Budget list the showcase.
      await tapAndSettle(tester, find.byKey(const ValueKey('back')));
      await openByTitle(tester, 'Budget');
      expect(find.byKey(ValueKey('backlink-${s.showcaseId}')), findsOneWidget);
    });

    appTest('@today inserts a date mention', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final b = editableStarting('Plain paragraph');
      await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
      await typeText(tester, ' @tod');
      await _key(tester, LogicalKeyboardKey.enter);
      final blk = await blockById(tester, s.showcaseId, 'sc03');
      final m = blk.marks.firstWhere((m) => m.kind == BMarkKind.date);
      expect(blk.text.substring(m.start, m.end).startsWith('@'), isTrue);
      expect(RegExp(r'^\d{4}-\d{2}-\d{2}$').hasMatch(m.url), isTrue);
    });

    appTest('the emoji picker sets and removes the page icon', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await g.addPointer(location: Offset.zero);
      await g.moveTo(tester.getCenter(find.byKey(const ValueKey('page-title'))));
      await settle(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('add-icon')));
      await tester.enterText(find.byKey(const ValueKey('emoji-filter')), 'rocket');
      await settle(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('emoji-🚀')));
      expect((await pageOf(tester, s.showcaseId)).icon, '🚀');
      await tapAndSettle(tester, find.byKey(const ValueKey('page-icon')));
      await tapAndSettle(tester, find.byKey(const ValueKey('emoji-remove')));
      expect((await pageOf(tester, s.showcaseId)).icon, isNull);
      await g.removePointer();
    });

    appTest('a cover can be removed from the cover buttons', (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final r = await createRichPage(tester, tv);
      await openByTitle(tester, richTitle);
      expect(find.byKey(const ValueKey('page-cover')), findsOneWidget);
      final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await g.addPointer(location: Offset.zero);
      await g.moveTo(tester.getCenter(find.byKey(const ValueKey('page-cover'))));
      await settle(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('cover-remove')));
      expect((await pageOf(tester, r.id)).cover, isNull);
      expect(find.byKey(const ValueKey('page-cover')), findsNothing);
      await g.removePointer();
    });

    appTest('locking the page makes it read-only until unlocked', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu')));
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu-lock')));
      await tester.tapAt(const Offset(800, 880));
      await settle(tester);
      expect(find.byKey(const ValueKey('locked-banner')), findsOneWidget);
      expect(editState(tester, editableStarting('Plain paragraph')).widget.readOnly, isTrue);
      await tapAndSettle(tester, find.byKey(const ValueKey('unlock')));
      expect(editState(tester, editableStarting('Plain paragraph')).widget.readOnly, isFalse);
      expect((await blocksOf(tester, s.showcaseId)).length, greaterThan(10));
    });

    appTest('small text and full width change the page layout', (tester) async {
      await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final before = tester.getSize(editableStarting('Plain paragraph')).width;
      final fontBefore = editState(tester, editableStarting('Plain paragraph')).widget.style.fontSize!;
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu')));
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu-full-width')));
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu-small-text')));
      await tester.tapAt(const Offset(800, 880));
      await settle(tester);
      expect(tester.getSize(editableStarting('Plain paragraph')).width, greaterThan(before + 100));
      expect(editState(tester, editableStarting('Plain paragraph')).widget.style.fontSize, lessThan(fontBefore));
    });

    appTest('duplicate from the page menu makes a copy and opens it', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu')));
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu-duplicate')));
      final pages = await core(tester, (v) => v.listPages(offset: 0, limit: 100));
      final copies = pages.where((p) => p.title.startsWith(showcaseTitle)).toList();
      expect(copies.length, 2);
      final copy = copies.firstWhere((p) => p.id != s.showcaseId);
      expect((await blocksOf(tester, copy.id)).length, (await blocksOf(tester, s.showcaseId)).length);
    });

    appTest('an image resizes from its side handle and keeps the width', (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final r = await createRichPage(tester, tv);
      await openByTitle(tester, richTitle);
      final img = find.byKey(const ValueKey('image-r19'));
      await tester.dragUntilVisible(img, find.byType(CustomScrollView).last, const Offset(0, -200));
      await settle(tester);
      final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await g.addPointer(location: Offset.zero);
      await g.moveTo(tester.getCenter(img));
      await settle(tester);
      final h = find.byKey(const ValueKey('image-resize-r-r19'));
      await tester.drag(h, const Offset(-80, 0), kind: PointerDeviceKind.mouse);
      await settle(tester);
      final w = (await blockById(tester, r.id, 'r19')).width;
      expect(w, isNotNull);
      expect(w!, lessThan(70));
      await g.removePointer();
    });

    appTest('a bookmark block takes its URL from the picker', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      final b = editableStarting('Plain paragraph');
      await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
      await _key(tester, LogicalKeyboardKey.enter);
      await typeText(tester, '/bookm');
      await _key(tester, LogicalKeyboardKey.enter);
      await settle(tester);
      expect(find.byKey(const ValueKey('url-picker')), findsOneWidget);
      await tester.enterText(find.byKey(const ValueKey('picker-field')), 'https://flutter.dev/docs');
      await tapAndSettle(tester, find.byKey(const ValueKey('picker-submit')));
      final bm = (await blocksOf(tester, s.showcaseId)).firstWhere((x) => x.kind == Kinds.bookmark);
      expect(bm.target, 'https://flutter.dev/docs');
      expect(find.text('flutter.dev'), findsWidgets);
    });

    appTest('a link to page opens its page; the table of contents lists headings', (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      await createRichPage(tester, tv);
      await openByTitle(tester, richTitle);
      expect(find.byKey(const ValueKey('toc-r02')), findsOneWidget);
      expect(find.byKey(const ValueKey('toc-r17')), findsOneWidget);
      final link = find.byKey(const ValueKey('pagelink-r22'));
      await tester.dragUntilVisible(link, find.byType(CustomScrollView).last, const Offset(0, -200));
      await settle(tester);
      await tapAndSettle(tester, link);
      expect(_title(tester), 'Recipes');
    });

    test('unknown kinds get a read-only placeholder type', () {
      final t = blockRegistry.of('mystery_kind');
      expect(t.editable, isFalse);
      expect(t.inMenu, isFalse);
      expect(blockRegistry.has('mystery_kind'), isFalse);
    });
  });

  group('mobile page', () {
    appTest('select mode from the block sheet: pick blocks, then delete them', (tester) async {
      final s = await pumpApp(tester, mobile: true);
      await openShowcase(tester);
      await tester.longPress(editableStarting('Bulleted list item'));
      await settle(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('sheet-select')));
      // In select mode a cover over the row takes the tap, not the text.
      await tester.tap(editableStarting('Another top-level'), warnIfMissed: false);
      await settle(tester);
      expect(find.byKey(const ValueKey('select-bar')), findsOneWidget);
      await tapAndSettle(tester, find.byKey(const ValueKey('select-delete')));
      final ids = [for (final b in await blocksOf(tester, s.showcaseId)) b.id];
      expect(ids, isNot(contains('sc05')));
      expect(ids, isNot(contains('sc08')));
      await tapAndSettle(tester, find.byKey(const ValueKey('select-done-top')));
      expect(find.byKey(const ValueKey('select-bar')), findsNothing);
    });

    appTest('the keyboard toolbar undoes and redoes', (tester) async {
      final s = await pumpApp(tester, mobile: true);
      await openShowcase(tester);
      await focusAt(tester, editableStarting('Plain paragraph'), 0);
      await showKeyboard(tester);
      await typeText(tester, 'X');
      expect((await blockById(tester, s.showcaseId, 'sc03')).text.startsWith('XPlain'), isTrue);
      await tapAndSettle(tester, find.byKey(const ValueKey('kb-undo')));
      expect((await blockById(tester, s.showcaseId, 'sc03')).text.startsWith('Plain'), isTrue);
      await tapAndSettle(tester, find.byKey(const ValueKey('kb-redo')));
      expect((await blockById(tester, s.showcaseId, 'sc03')).text.startsWith('XPlain'), isTrue);
    });

    appTest('the keyboard toolbar fits 375 px in every row', (tester) async {
      await pumpApp(tester, mobile: true);
      await openShowcase(tester);
      await focusAt(tester, editableStarting('Plain paragraph'), 0);
      await showKeyboard(tester);
      final hide = tester.getRect(find.byKey(const ValueKey('kb-hide')));
      expect(hide.right, lessThanOrEqualTo(375));
      for (final (mode, last) in [('main', 'more'), ('more', 'block-color'), ('format', 'color')]) {
        if (mode != 'main') await tapAndSettle(tester, find.byKey(ValueKey('kb-$mode')));
        expect(tester.getRect(find.byKey(ValueKey('kb-$last'))).right, lessThanOrEqualTo(hide.left), reason: mode);
        if (mode != 'main') await tapAndSettle(tester, find.byKey(const ValueKey('kb-back')));
      }
      expect(tester.takeException(), isNull);
    });

    appTest('stacked columns on mobile', (tester) async {
      final tv = await pumpApp(tester, mobile: true);
      await createRichPage(tester, tv);
      await openByTitle(tester, richTitle);
      final m = editableWith('Morning');
      final e = editableWith('Evening');
      await tester.dragUntilVisible(e, find.byType(CustomScrollView).last, const Offset(0, -200));
      await settle(tester);
      expect(tester.getTopLeft(e).dy, greaterThan(tester.getBottomLeft(m).dy));
      expect((tester.getTopLeft(e).dx - tester.getTopLeft(m).dx).abs(), lessThan(2));
    });
  });
}
