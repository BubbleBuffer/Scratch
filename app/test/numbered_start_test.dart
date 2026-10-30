import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/blocks/numbered.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/clipboard.dart';
import 'package:notes_app/editor/model.dart';
import 'package:notes_app/shell/addons.dart';

import 'harness.dart';

BOp _insert(
  String id, {
  String? after,
  String? parent,
  int? start,
  String kind = Kinds.numbered,
}) => BOp(
      op: OpKind.insert,
      id: id,
      after: after,
      parent: parent,
      kind: kind,
      text: id,
      marks: const [],
      checked: false,
      numberedStart: start,
    );

void main() {
  group('numbered run model', () {
    setUp(registerBuiltins);

    test('starts at stored number, continues siblings and resets after text', () {
      final doc = Doc('page', '')..applyOps([
        _insert('first', start: 3),
        _insert('second', after: 'first', start: 99),
        _insert('child', parent: 'first', start: 7),
        _insert('break', after: 'second', kind: Kinds.paragraph),
        _insert('default', after: 'break'),
        _insert('next', after: 'default'),
      ]);
      expect(doc.roots.map((n) => n.number), [3, 4, 1, 1, 2]);
      expect(doc.byId['child']!.number, 7);
      expect(numberLabel(doc.roots.first.number, 0), '3.');
      expect(numberLabel(doc.roots[1].number, 0), '4.');
      expect(numberLabel(doc.byId['default']!.number, 0), '1.');
      expect(nodesToMarkdown(doc.roots),
          '3. first\n    7. child\n4. second\nbreak\n1. default\n2. next');
    });

    test('zero is a valid start and does not restart the next sibling', () {
      final doc = Doc('page', '')..applyOps([
        _insert('zero', start: 0),
        _insert('one', after: 'zero', start: 8),
      ]);
      expect(doc.roots.map((n) => n.number), [0, 1]);
    });

    test('text operations preserve metadata and snapshots recreate it', () {
      final doc = Doc('page', '')..applyOps([_insert('first', start: 3)]);
      final original = doc.snapshot();
      final node = doc.roots.single;
      node.text = 'edited';
      doc.applyOps([node.setTextOp()]);
      expect(node.numberedStart, 3);
      expect(node.toFlat().numberedStart, 3);

      doc.delete(node);
      final restore = diffSnapshots(doc.snapshot(), original);
      expect(restore.single.numberedStart, 3);
      doc.applyOps(restore);
      expect(doc.roots.single.numberedStart, 3);
      expect(doc.roots.single.number, 3);

      final before = doc.snapshot();
      doc.roots.single.numberedStart = null;
      final clear = diffSnapshots(before, doc.snapshot());
      expect(clear.single.op, OpKind.setNumberedStart);
      doc.applyOps(clear);
      expect(doc.roots.single.number, 1);
      doc.applyOps(diffSnapshots(doc.snapshot(), before));
      expect(doc.roots.single.number, 3);
    });
  });

  group('numbered run paste', () {
    setUpAll(initCore);

    test('a pasted run keeps the start number of its first item only', () {
      final nodes = markdownToNodes('3. a\n4. b');
      expect([for (final n in nodes) n.numberedStart], [3, null]);
      final doc = Doc('page', '')..applyOps([
        _insert('a', start: nodes[0].numberedStart),
        _insert('b', after: 'a', start: nodes[1].numberedStart),
      ]);
      expect([for (final n in doc.roots) numberLabel(n.number, 0)], ['3.', '4.']);
    });

    test('a run starting at one, a nested run and a run after a bullet', () {
      final nodes = markdownToNodes('1. a\n2. b\n- x\n  5. c\n  6. d\n3. e');
      expect([for (final n in nodes) n.numberedStart], [null, null, null, 3]);
      expect([for (final n in nodes[2].children) n.numberedStart], [5, null]);
    });
  });

  group('numbered run editor', () {
    setUpAll(initCore);

    appTest('shows 3 and 4 and retains start through editing and splitting',
        (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await core(tester, (v) => v.apply(s.showcaseId, [
        BOp(op: OpKind.setNumberedStart, id: 'sc10', kind: Kinds.numbered,
            text: '', marks: const [], checked: false, numberedStart: 3),
      ]));
      await openShowcase(tester);
      await tester.dragUntilVisible(editableWith('Second step'),
          find.byType(CustomScrollView).last, const Offset(0, -200));
      await settle(tester);
      expect(find.text('3.'), findsOneWidget);
      expect(find.text('4.'), findsOneWidget);

      await focusAt(tester, editableWith('First step'), 'First step'.length);
      await typeText(tester, ' edited');
      final block = await blockById(tester, s.showcaseId, 'sc10');
      expect(block.text, 'First step edited');
      expect(block.numberedStart, 3);
      expect(Doc.fromPage(await pageOf(tester, s.showcaseId))
          .byId['sc10']!.number, 3);

      await focusAt(tester, editableWith('First step edited'), 5);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await settle(tester);
      final blocks = await blocksOf(tester, s.showcaseId);
      final i = blocks.indexWhere((b) => b.id == 'sc10');
      expect(blocks[i].text, 'First');
      expect(blocks[i].numberedStart, 3);
      expect(blocks[i + 1].text, ' step edited');
      expect(blocks[i + 1].kind, Kinds.numbered);
      expect(blocks[i + 1].numberedStart, isNull);
      final doc = Doc.fromPage(await pageOf(tester, s.showcaseId));
      expect(doc.byId[blocks[i].id]!.number, 3);
      expect(doc.byId[blocks[i + 1].id]!.number, 4);
      expect(doc.byId['sc11']!.number, 5);
    });
  });
}
