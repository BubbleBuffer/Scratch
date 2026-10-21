// The grouped search results on the desktop overlay body: Pages then Chats
// headers, chat labels, keyboard selection over the grouped order, and the
// type chips that write the `type:` operator through the parser.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/search.dart';
import 'package:notes_app/shell/search_groups.dart';
import 'package:notes_app/shell/search_widgets.dart';
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';
import 'search_hits_fixture.dart';

/// Shows the overlay body over the app with [hits] and returns its field. The
/// chips rewrite the text through the app's real parser.
Future<TextEditingController> openBody(
  WidgetTester tester, {
  required List<Hit>? hits,
  String text = '',
  Set<String> types = const {},
  ValueChanged<Hit>? onPick,
  ValueChanged<String>? onQuery,
}) async {
  final controller = TextEditingController(text: text);
  final overlay = tester.element(find.byType(Overlay).first);
  final search = PageSearch(overlay.app.vault);
  unawaited(
    showModalPanel<void>(
      overlay,
      (_) => SearchPanelBody(
        controller: controller,
        onQueryChanged: (q) => onQuery?.call(q),
        onPick: (hit) => onPick?.call(hit),
        hits: hits,
        rewriteTypes: search.rewriteTypes,
        terms: const ['garden'],
        activeTypes: types,
      ),
      width: 640,
      height: 520,
    ),
  );
  await settle(tester);
  return controller;
}

/// Every text span under [span], the span itself included.
Iterable<TextSpan> _textSpans(InlineSpan span) sync* {
  if (span is! TextSpan) return;
  yield span;
  for (final child in span.children ?? const <InlineSpan>[]) {
    yield* _textSpans(child);
  }
}

void main() {
  setUpAll(initCore);

  appTest('results are grouped under Pages, then Chats', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    await openBody(
      tester,
      hits: [
        chatHit('c-1'),
        pageHit('p-1', 'Garden'),
        chatHit('c-2', summary: 'Second chat row.'),
        pageHit('p-2', 'Gardening'),
      ],
    );
    final pages = find.byKey(const ValueKey('search-group-pages'));
    final chats = find.byKey(const ValueKey('search-group-chats'));
    expect(pages, findsOneWidget);
    expect(chats, findsOneWidget);
    expect(find.text('PAGES'), findsOneWidget);
    expect(find.text('CHATS'), findsOneWidget);
    expect(tester.getTopLeft(pages).dy < tester.getTopLeft(chats).dy, isTrue);
    double rowY(String rowId) =>
        tester.getTopLeft(find.byKey(ValueKey('hit-$rowId'))).dy;
    // Within each group the core's order holds; the chat rows come last.
    expect(rowY('p-1') < rowY('p-2'), isTrue);
    expect(rowY('p-2') < rowY('agent/c-1'), isTrue);
    expect(rowY('agent/c-1') < rowY('agent/c-2'), isTrue);
  });

  appTest('chat rows show the breadcrumb and the summary', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    await openBody(tester, hits: [chatHit('c-1'), pageHit('p-1', 'Garden')]);
    expect(
      find.text('BubblePlay cursor › Native drag-and-drop'),
      findsOneWidget,
    );
    expect(
      find.text('Explorer stalls during the Windows handoff.'),
      findsOneWidget,
    );
    // A chat has no page path line.
    expect(find.byKey(const ValueKey('hit-path-agent/c-1')), findsNothing);
  });

  appTest('a chat and a page sharing an id keep separate rows', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    await openBody(tester, hits: [pageHit('c-1', 'Garden'), chatHit('c-1')]);
    expect(find.byKey(const ValueKey('hit-c-1')), findsOneWidget);
    expect(find.byKey(const ValueKey('hit-agent/c-1')), findsOneWidget);
  });

  appTest('matching words are highlighted in page titles', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    await openBody(tester, hits: [pageHit('p-1', 'Garden')]);
    // The matched word is a marked run inside the title's rich text.
    final marked = tester
        .widgetList<RichText>(
          find.descendant(
            of: find.byKey(const ValueKey('hit-p-1')),
            matching: find.byType(RichText),
          ),
        )
        .expand((r) => _textSpans(r.text))
        .where((span) => span.style?.backgroundColor != null);
    expect(marked.map((s) => s.text), ['Garden']);
  });

  appTest('Down and Enter pick the next row in grouped order', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final picked = <Hit>[];
    await openBody(
      tester,
      hits: [
        chatHit('c-1', breadcrumb: 'Chat one › Chapter'),
        pageHit('p-1', 'Garden'),
        pageHit('p-2', 'Gardening'),
      ],
      onPick: picked.add,
    );
    // Grouped order is p-1, p-2, c-1: one Down then Enter opens p-2.
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await settle(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await settle(tester);
    expect(picked.map((h) => h.id), ['p-2']);
  });

  appTest('Enter on a chat row picks it by its stored anchor', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final picked = <Hit>[];
    await openBody(
      tester,
      hits: [
        chatHit('c-1', breadcrumb: 'Chat one › Chapter'),
        pageHit('p-1', 'Garden'),
        pageHit('p-2', 'Gardening'),
      ],
      onPick: picked.add,
    );
    // Two Downs reach the chat (the last group), opened by its target ids.
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await settle(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await settle(tester);
    expect(picked.map((h) => h.id), ['c-1']);
    final target = ChatSearchTarget.parse(picked.single.target)!;
    expect(target.sessionId, 'sess-1');
    expect(target.firstMessageId, 'm-53');
    expect(target.lastMessageId, 'm-88');
    expect(target.refLevel, ChatRefLevel.section);
  });

  appTest('the Chats chip writes the type operator', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final queries = <String>[];
    final controller = await openBody(
      tester,
      hits: [pageHit('p-1', 'Garden')],
      text: 'garden tag:work',
      onQuery: queries.add,
    );
    await tester.tap(find.byKey(const ValueKey('search-type-chats')));
    await settle(tester);
    expect(controller.text, 'garden tag:work type:chat');
    expect(queries.last, 'garden tag:work type:chat');
  });

  appTest('tapping the active Chats chip returns to All', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final queries = <String>[];
    await openBody(
      tester,
      hits: [chatHit('c-1')],
      text: 'garden tag:work type:chat',
      types: searchTypeChats,
      onQuery: queries.add,
    );
    await tester.tap(find.byKey(const ValueKey('search-type-chats')));
    await settle(tester);
    expect(queries.last, 'garden tag:work');
  });

  appTest('a typed type: shows a selected chip that removes it', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final queries = <String>[];
    await openBody(
      tester,
      hits: [pageHit('p-1', 'Garden')],
      text: 'garden type:document',
      types: const {'document'},
      onQuery: queries.add,
    );
    // Documents is no fixed set, so it gets its own selected chip.
    expect(
      find.byKey(const ValueKey('search-type-value-document')),
      findsOneWidget,
    );
    expect(find.byKey(const ValueKey('search-type-pages')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('search-type-value-document')));
    await settle(tester);
    expect(queries.last, 'garden');
  });

  appTest('type:database also gets a removable chip', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final queries = <String>[];
    await openBody(
      tester,
      hits: [pageHit('p-1', 'Garden')],
      text: 'type:database garden',
      types: const {'database'},
      onQuery: queries.add,
    );
    expect(
      find.byKey(const ValueKey('search-type-value-database')),
      findsOneWidget,
    );
    await tester.tap(find.byKey(const ValueKey('search-type-value-database')));
    await settle(tester);
    expect(queries.last, 'garden');
  });

  appTest('a quoted phrase keeps its words when a chip is clicked', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final queries = <String>[];
    await openBody(
      tester,
      hits: [pageHit('p-1', 'Garden')],
      text: '"say type:chat" type:document',
      types: const {'document'},
      onQuery: queries.add,
    );
    await tester.tap(find.byKey(const ValueKey('search-type-chats')));
    await settle(tester);
    expect(queries.last, '"say type:chat" type:chat');
  });

  appTest('a negated type operator is not a chip', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    await openBody(
      tester,
      hits: [pageHit('p-1', 'Garden')],
      text: '-type:chat garden',
    );
    expect(find.byKey(const ValueKey('search-type-value-chat')), findsNothing);
  });

  appTest('a malformed chat target says why it cannot open', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    await openBody(tester, hits: [pageHit('p-1', 'Garden')]);
    // A toast needs an Overlay above its context: the body's is one.
    final inBody = tester.element(find.byType(SearchPanelBody));
    final broken = Hit(
      id: 'c-9',
      title: 'Broken',
      snippet: '',
      fuzzy: false,
      source: 'agent',
      pageType: 'chat',
      target: 'not json',
    );
    unawaited(openChatHit(inBody, broken));
    await settle(tester);
    expect(find.text('Could not open this chat'), findsOneWidget);
  });
}
