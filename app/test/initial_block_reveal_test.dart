import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;

import 'harness.dart';

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    for (final nested in [false, true]) {
      appTest('initial block finds a distant variable-height ${nested ? 'collapsed nested' : 'plain'} to-do on ${mobile ? 'mobile' : 'desktop'}', (tester) async {
        await pumpApp(tester, mobile: mobile, empty: true);
        String? target, outer, inner;
        final page = await core(tester, (v) async {
          final page = await v.createPage(title: 'Long project plan');
          final ops = <BOp>[];
          String? after;
          void paragraph(int i) {
            final id = newId();
            // A tall first half followed by short rows makes the first
            // viewport's average misleading as well as the fixed estimate.
            final lines = i < 70 ? 13 + (i % 3) : 1 + (i % 3);
            ops.add(BOp(op: OpKind.insert, id: id, after: after,
              kind: i % 5 == 0 ? Kinds.heading2 : Kinds.paragraph,
              text: List.generate(lines, (line) => 'Section $i, line $line: a variable-height planning paragraph.').join('\n'),
              marks: const [], checked: false));
            after = id;
          }
          for (var i = 0; i < 120; i++) { paragraph(i); }
          if (nested) {
            outer = newId(); inner = newId();
            ops.add(BOp(op: OpKind.insert, id: outer!, after: after, kind: Kinds.toggle, text: 'Launch checklist', marks: const [], checked: false));
            ops.add(BOp(op: OpKind.insert, id: inner!, parent: outer, kind: Kinds.toggle, text: 'Final review', marks: const [], checked: false));
          }
          target = newId();
          const text = 'Review launch @October 6, 2026';
          ops.add(BOp(op: OpKind.insert, id: target!, parent: nested ? inner : null, after: nested ? null : after,
              kind: Kinds.todo, text: text, checked: false,
              marks: [BMark(start: text.indexOf('@'), end: text.length, kind: BMarkKind.date, url: '2026-10-06')]));
          after = nested ? outer : target;
          for (var i = 120; i < 160; i++) { paragraph(i); }
          await v.apply(page, ops); await v.flush();
          return page;
        });
        nav.openPage(tester.element(find.byType(HomeScreen)), page, 'Long project plan', initialBlock: target);
        await settle(tester);
        final editor = tester.state<PageEditorState>(find.byType(PageScreen));
        expect(editor.highlightedBlock, target);
        expect(editor.rows[target]?.mounted, isTrue);
        final bounds = tester.getRect(editableStarting('Review launch'));
        expect(bounds.top, greaterThanOrEqualTo(44));
        expect(bounds.bottom, lessThanOrEqualTo(mobile ? 812 : 900));
        if (nested) {
          expect(editor.isOpen(editor.doc!.byId[outer]!), isTrue);
          expect(editor.isOpen(editor.doc!.byId[inner]!), isTrue);
        }
        await tester.pump(const Duration(seconds: 3));
        expect(editor.highlightedBlock, isNull);
      });
    }
  }
}
