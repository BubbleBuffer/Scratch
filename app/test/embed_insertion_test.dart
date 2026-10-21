import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/clipboard.dart';
import 'package:notes_app/editor/model.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/registry/embed_block.dart';
import 'package:notes_app/registry/block_codecs.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;

import 'harness.dart';

void main() {
  setUpAll(initCore);

  appTest('a page link can turn into Embed without choosing its target again', (
    tester,
  ) async {
    final f = await pumpApp(tester, mobile: false);
    final host = await core(tester, (v) => v.createPage(title: 'Link host'));
    final target = f.pages['Garden plan']!;
    await core(
      tester,
      (v) => v.apply(host, [
        const BOp(
          op: OpKind.insert,
          id: 'link',
          kind: Kinds.linkToPage,
          text: '',
          marks: [],
          checked: false,
        ),
        BOp(
          op: OpKind.setTarget,
          id: 'link',
          kind: Kinds.linkToPage,
          target: target,
          text: '',
          marks: const [],
          checked: false,
        ),
      ]),
    );
    nav.openPage(tester.element(find.byType(HomeScreen)), host, 'Link host');
    await settle(tester);
    expect(
      blockRegistry.turnIntoTypes
          .singleWhere((t) => t.kind == embedKind)
          .menuLabel,
      'Embed',
    );
    final state = tester.state<PageEditorState>(find.byType(PageScreen).last);
    state.blockTurnInto(state.doc!.byId['link']!, embedKind);
    await settle(tester);
    expect(find.byKey(const ValueKey('page-picker')), findsNothing);
    final saved = await core(tester, (v) => v.loadPage(host));
    final embed = saved.blocks.singleWhere((b) => b.id == 'link');
    expect(embed.kind, embedKind);
    expect(embed.target, target);
    expect(embed.text, isEmpty);
    expect(find.textContaining('garden', findRichText: true), findsWidgets);
  });

  for (final mobile in [false, true]) {
    appTest(
      '/embed uses the existing page picker and stores only the id on ${mobile ? 'mobile' : 'desktop'}',
      (tester) async {
        final f = await pumpApp(tester, mobile: mobile);
        final host = await core(
          tester,
          (v) => v.createPage(title: 'Embed host'),
        );
        nav.openPage(
          tester.element(find.byType(HomeScreen)),
          host,
          'Embed host',
        );
        await settle(tester);
        await focusAt(tester, editableWith('').last, 0);
        await typeText(tester, '/embed');
        await tapAndSettle(tester, find.byKey(const ValueKey('slash-embed')));
        expect(find.byKey(const ValueKey('page-picker')), findsOneWidget);
        final target = f.pages['Garden plan']!;
        await tester.enterText(
          find.descendant(
            of: find.byKey(const ValueKey('page-picker-field')),
            matching: find.byType(EditableText),
          ),
          'Garden plan',
        );
        await settle(tester);
        // Search through the existing picker; older notes may not be recent.
        await tapAndSettle(tester, find.byKey(ValueKey('page-pick-$target')));
        expect(find.byKey(const ValueKey('page-picker')), findsNothing);
        final saved = await core(tester, (v) => v.loadPage(host));
        final block = saved.blocks.singleWhere((b) => b.kind == embedKind);
        expect(block.target, target);
        expect(block.text, isEmpty);
        expect(find.textContaining('garden', findRichText: true), findsWidgets);
      },
    );
  }

  test('clipboard preserves an annotated embed id and ignores edited fallback title', () {
    final node = Node(
      id: 'embed',
      kind: embedKind,
      target: 'missing-雪-->',
      text: 'Never store this',
    );
    final markdown = annotatedBlockMarkdown(
      node,
      'Human readable title',
    ).join('\n');
    final pasted = markdownToNodes(
      markdown.replaceFirst('Human readable title', 'Edited title'),
    );
    expect(pasted, hasLength(1));
    expect(pasted.single.kind, embedKind);
    expect(pasted.single.target, node.target);
    expect(pasted.single.text, isEmpty);
    expect(pasted.single.marks, isEmpty);
  });

  test('malformed embed annotation remains ordinary Markdown', () {
    final pasted = markdownToNodes('> readable <!-- block=zz -->');
    expect(pasted.single.kind, Kinds.quote);
    expect(pasted.single.text, contains('block=zz'));
  });

  test(
    'empty embed clipboard fallback preserves nullable target and metadata',
    () {
      final node = Node(
        id: 'e',
        kind: embedKind,
        color: 'red',
        width: 40,
        checked: true,
      );
      final markdown = annotatedBlockMarkdown(node, '').join('\n');
      expect(markdown, startsWith('> <!-- block='));
      final pasted = markdownToNodes(markdown).single;
      expect(pasted.kind, embedKind);
      expect(pasted.target, isNull);
      expect(pasted.text, isEmpty);
      expect(pasted.checked, isTrue);
      expect(pasted.color, 'red');
      expect(pasted.width, 40);
    },
  );
}
