// Assistant text renders as markdown, on the real core and agent bridge
// (the vault's markdown reader). The reply itself is fake state: BubblePaw's
// fake provider only answers "ok".
// - blocks: headings, nested bullets, numbers, quote, divider, code fence;
// - inline marks: bold, italic, inline code, links;
// - links: web links open outside the app, page links (wikilink, by target,
//   by text) open the page;
// - user messages stay plain text.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/src/rust/api/types.dart';

import 'agent_test.dart' show openAgentScreen, openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

/// A reply that uses every construct the chat renders.
const sampleMarkdown = '''## Garden plan

The **seeds** arrive *Friday*; see [[Garden plan]] and [the guide](https://example.com/guide). Run `water()` daily.

- Tomatoes
  - Cherry
- Basil

1. Dig the beds
2. Plant

> Water in the morning.

---

Also [the recipes](Recipes) and [Weekly review](later).

```js
plant("basil")
```''';

/// A chat where the user asked [question] (a real run; the fake answers
/// "ok") and the assistant then replied [markdown] (fake state: the fake
/// provider cannot answer with given text).
Future<AgentStore> chatWithReply(
  WidgetTester tester,
  String markdown, {
  String question = 'How should I plan the garden?',
}) async {
  final store = await openAgentStore(tester);
  await openAgentScreen(tester);
  await real(tester, () => store.createSession(title: 'Garden plan'));
  await real(tester, () => store.send(question));
  await settleUntil(
    tester,
    () =>
        (store.state?.messages.any(
              (m) => m.role == 'assistant' && m.content == 'ok',
            ) ??
            false) &&
        !store.running,
    timeout: const Duration(seconds: 30),
  );
  store.debugAddReply(markdown);
  await settle(tester);
  return store;
}

BMark markOver(FlatBlock b, String text) {
  final at = b.text.indexOf(text);
  return b.marks.firstWhere((m) => m.start == at && m.end == at + text.length);
}

void main() {
  setUpAll(initCore);

  appTest(
    'explicit page URL uses its id even when its label names another page',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final id = tv.pages['Garden plan']!;
      final block = markdownBlocks(tv.vault.raw, '[Recipes](page:$id)').single;
      expect(
        (block.marks.single.kind, block.marks.single.url),
        (BMarkKind.pageLink, id),
      );
      final code = markdownBlocks(
        tv.vault.raw,
        '```\n[Recipes](page:$id)\n```',
      ).single;
      expect(code.text, '[Recipes](page:$id)');
      final inline = markdownBlocks(
        tv.vault.raw,
        '`[x](page:$id)` then **bold** and [Recipes](page:$id)',
      ).single;
      expect(inline.text, '[x](page:$id) then bold and Recipes');
      expect(markOver(inline, 'bold').kind, BMarkKind.bold);
      expect(
        (markOver(inline, 'Recipes').kind, markOver(inline, 'Recipes').url),
        (BMarkKind.pageLink, id),
      );
      expect(markOver(inline, '[x](page:$id)').kind, BMarkKind.code);
    },
  );

  appTest('markdown parses into blocks, with page links resolved', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final blocks = markdownBlocks(tv.vault.raw, sampleMarkdown);
    expect(
      [for (final b in blocks) (b.kind, b.depth)],
      [
        ('heading2', 0),
        ('paragraph', 0),
        ('bulleted', 0),
        ('bulleted', 1),
        ('bulleted', 0),
        ('numbered', 0),
        ('numbered', 0),
        ('quote', 0),
        ('divider', 0),
        ('paragraph', 0),
        ('code', 0),
      ],
    );
    final p = blocks[1];
    expect(
      p.text,
      'The seeds arrive Friday; see Garden plan and the guide. Run water() daily.',
    );
    expect(markOver(p, 'seeds').kind, BMarkKind.bold);
    expect(markOver(p, 'Friday').kind, BMarkKind.italic);
    expect(markOver(p, 'water()').kind, BMarkKind.code);
    final wiki = markOver(p, 'Garden plan');
    expect(
      (wiki.kind, wiki.url),
      (BMarkKind.pageLink, tv.pages['Garden plan']),
    );
    final web = markOver(p, 'the guide');
    expect((web.kind, web.url), (BMarkKind.link, 'https://example.com/guide'));
    final also = blocks[9];
    final byTarget = markOver(also, 'the recipes');
    expect(
      (byTarget.kind, byTarget.url),
      (BMarkKind.pageLink, tv.pages['Recipes']),
    );
    final byText = markOver(also, 'Weekly review');
    expect(
      (byText.kind, byText.url),
      (BMarkKind.pageLink, tv.pages['Weekly review']),
    );
    expect(blocks.last.text, 'plant("basil")');
    // Unknown targets stay ordinary links.
    final plain = markdownBlocks(
      tv.vault.raw,
      '[nothing](no-such-page)',
    ).single;
    expect(plain.marks.single.kind, BMarkKind.link);
  });

  appTest('a markdown reply renders as blocks; the user message stays plain', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    await chatWithReply(
      tester,
      sampleMarkdown,
      question: 'Plan it with **bold** and `code`',
    );
    expect(find.text('Garden plan', findRichText: true), findsWidgets);
    expect(find.text('Tomatoes', findRichText: true), findsOneWidget);
    // Nested items are indented under their parent.
    final tomato = tester
        .getTopLeft(find.text('Tomatoes', findRichText: true))
        .dx;
    final cherry = tester
        .getTopLeft(find.text('Cherry', findRichText: true))
        .dx;
    expect(cherry, greaterThan(tomato));
    expect(find.text('•'), findsNWidgets(2));
    expect(find.text('◦'), findsOneWidget);
    expect(find.text('1.'), findsOneWidget);
    expect(find.text('2.'), findsOneWidget);
    expect(
      find.text('Water in the morning.', findRichText: true),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: find.byType(CodeBox),
        matching: find.text('plant("basil")'),
      ),
      findsOneWidget,
    );
    // The user message is shown as typed.
    expect(find.text('Plan it with **bold** and `code`'), findsOneWidget);

    // Bold and inline code are styled spans.
    final rich = tester.widget<RichText>(
      find.text(
        'The seeds arrive Friday; see Garden plan and the guide. Run water() daily.',
        findRichText: true,
      ),
    );
    TextStyle? styleOf(String s) {
      TextStyle? found;
      rich.text.visitChildren((span) {
        if (span is TextSpan && span.text == s) found = span.style;
        return found == null;
      });
      return found;
    }

    expect(styleOf('seeds')?.fontWeight, FontWeight.w600);
    expect(styleOf('Friday')?.fontStyle, FontStyle.italic);
    expect(styleOf('water()')?.backgroundColor, isNotNull);
  });

  appTest('links: web links open outside, page links open the page', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final opened = <String>[];
    final saved = openWebLink;
    openWebLink = (url) async => opened.add(url);
    try {
      await chatWithReply(tester, sampleMarkdown);
      await tester.tapOnText(find.textRange.ofSubstring('the guide'));
      await settle(tester);
      expect(opened, ['https://example.com/guide']);
      expect(key('agent-screen'), findsOneWidget);

      await tester.tapOnText(find.textRange.ofSubstring('the recipes'));
      await settle(tester);
      expect(key('agent-screen'), findsNothing);
      expect(
        find.byWidgetPredicate(
          (w) => w is PageScreen && w.pageId == tv.pages['Recipes'],
        ),
        findsOneWidget,
      );
      expect(opened, hasLength(1));
    } finally {
      openWebLink = saved;
    }
  });
}
