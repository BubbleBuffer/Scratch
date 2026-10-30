import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/services.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/model.dart';
import 'package:notes_app/editor/page_screen.dart';

import 'db_harness.dart';
import 'harness.dart' show editableWith, editState;

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    dbTest(
      'empty page opens without a document re-scan on ${mobile ? 'mobile' : 'desktop'}',
      (tester) async {
        await pumpDb(tester, mobile: mobile, tasks: 200);
        await core(tester, (v) => v.createPage(title: 'Empty'));
        await settle(tester);

        for (final title in ['Tasks', 'Empty']) {
          final seq = db.app.feed.seq;
          var notifications = 0;
          void changed() => notifications++;
          db.app.pages.addListener(changed);
          try {
            await openPage(tester, title);
            // Loading may adopt this page's metadata once. A reload/scan would
            // additionally notify for its first chunk and document chunks.
            expect(
              notifications,
              lessThanOrEqualTo(1),
              reason: 'opening must not restart the document scan',
            );
            expect(db.app.feed.seq, seq, reason: 'opening is read-only');
            final editor = tester.state<PageEditorState>(
              find.byType(PageScreen).last,
            );
            final page = await core(tester, (v) => v.loadPage(editor.pageId));
            if (title == 'Tasks') {
              // A former database opens in the editor with its view blocks,
              // as saved.
              expect(page.blocks.map((b) => b.kind), everyElement('view'));
              expect(editor.doc!.flat.map((n) => n.id), page.blocks.map((b) => b.id));
              continue;
            }
            expect(editor.doc!.flat.single.kind, Kinds.paragraph);
            expect(editor.doc!.flat.single.text, isEmpty);
            expect(page.blocks, isEmpty);
          } finally {
            db.app.pages.removeListener(changed);
          }
        }

        final editor = tester.state<PageEditorState>(
          find.byType(PageScreen).last,
        );
        final id = editor.doc!.flat.single.id;
        final line = editableWith('').last;
        editState(tester, line).widget.focusNode.requestFocus();
        await tester.pump();
        expect(editState(tester, line).widget.focusNode.hasFocus, isTrue);
        // A metadata change must not discard the still-local first line.
        await core(tester, (v) => v.renamePage(editor.pageId, 'Renamed'));
        await settle(tester);
        expect(editor.doc!.flat.single.id, id);
        expect(editState(tester, line).widget.focusNode.hasFocus, isTrue);
        expect(
          (await core(tester, (v) => v.loadPage(editor.pageId))).blocks,
          isEmpty,
        );

        await tester.enterText(line, 'First line');
        await settle(tester);
        var page = await core(tester, (v) => v.loadPage(editor.pageId));
        expect(page.blocks.single.id, id);
        expect(page.blocks.single.text, 'First line');
        editor.undo();
        await settle(tester);
        page = await core(tester, (v) => v.loadPage(editor.pageId));
        expect(page.blocks.single.text, isEmpty);
        editor.redo();
        await settle(tester);
        expect(
          (await core(
            tester,
            (v) => v.loadPage(editor.pageId),
          )).blocks.single.text,
          'First line',
        );
      },
    );
  }

  dbTest('an empty first line supports structural edits and outside content', (
    tester,
  ) async {
    await pumpDb(tester, mobile: false);
    for (final title in ['Split', 'Paste', 'Remote']) {
      await core(tester, (v) => v.createPage(title: title));
    }
    await settle(tester);

    await openPage(tester, 'Split');
    var editor = tester.state<PageEditorState>(find.byType(PageScreen).last);
    final line = editableWith('').last;
    editState(tester, line).widget.focusNode.requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await settle(tester);
    var page = await core(tester, (v) => v.loadPage(editor.pageId));
    expect(page.blocks.length, 2);
    expect(
      page.blocks.every((b) => b.kind == Kinds.paragraph && b.text.isEmpty),
      isTrue,
    );
    editor.undo();
    await settle(tester);
    expect(
      (await core(tester, (v) => v.loadPage(editor.pageId))).blocks.length,
      1,
    );

    await openPage(tester, 'Paste');
    editor = tester.state<PageEditorState>(find.byType(PageScreen).last);
    editor.pasteBlocks([
      Node(id: newId(), kind: Kinds.paragraph, text: 'Pasted'),
    ], after: editor.doc!.flat.single);
    await settle(tester);
    expect(
      (await core(tester, (v) => v.loadPage(editor.pageId))).blocks.single.text,
      'Pasted',
    );

    await openPage(tester, 'Remote');
    editor = tester.state<PageEditorState>(find.byType(PageScreen).last);
    final remoteId = newId();
    await core(
      tester,
      (v) => v.apply(editor.pageId, [
        BOp(
          op: OpKind.insert,
          id: remoteId,
          kind: Kinds.paragraph,
          text: 'Outside',
          marks: const [],
          checked: false,
        ),
      ]),
    );
    await settle(tester);
    expect(editor.doc!.flat.single.id, remoteId);
    await tester.enterText(editableWith('Outside').last, 'Edited outside line');
    await settle(tester);
    page = await core(tester, (v) => v.loadPage(editor.pageId));
    expect(page.blocks.single.id, remoteId);
    expect(page.blocks.single.text, 'Edited outside line');
  });
}
