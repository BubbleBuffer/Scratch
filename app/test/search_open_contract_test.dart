// The contract between a chat search hit and the code that opens it: what
// openChatHit asks for, and what it says when it cannot. The hit's `target`
// is opaque JSON that only the Agent add-on reads. A malformed target opens
// nothing and says so; an anchored target asks for its message range with the
// hit's own labels; a chat the agent does not have is reported, not guessed.
// search_open_anchor_test.dart covers the whole path over a real chat.

import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/core/models.dart';

import 'agent_test.dart' show openAgentStore;
import 'cert_helpers.dart' show homeContext;
import 'harness.dart';
import 'search_hits_fixture.dart';
import 'sync_helpers.dart' show real;

/// A context under the app's Overlay, as a toast needs (the home screen's).
BuildContext _overlayContext(WidgetTester tester) => homeContext(tester);

void main() {
  setUpAll(initCore);

  appTest('a malformed target opens nothing and says why', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final asked = <ChatReference>[];
    for (final target in [null, '', 'not json', '{"sessionId":"s"}', '[1,2]']) {
      final hit = Hit(
        id: 'c-9',
        title: 'Broken',
        snippet: '',
        fuzzy: false,
        source: 'agent',
        pageType: 'chat',
        target: target,
      );
      await openChatHit(
        _overlayContext(tester),
        hit,
        openReference: (context, reference) async => asked.add(reference),
      );
      await settle(tester);
      expect(find.text('Could not open this chat'), findsOneWidget);
      // Let the toast go before the next target.
      await tester.pump(const Duration(seconds: 10));
      await settle(tester);
    }
    expect(asked, isEmpty);
  });

  appTest('an anchored hit asks for its range with the hit labels', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final asked = <ChatReference>[];
    final hit = chatHit('c-1', summary: 'Train at nine.');
    await openChatHit(
      _overlayContext(tester),
      hit,
      openReference: (context, reference) async => asked.add(reference),
    );
    await settle(tester);
    final ref = asked.single;
    expect(ref.sessionId, 'sess-1');
    expect(ref.firstMessageId, 'm-53');
    expect(ref.lastMessageId, 'm-88');
    expect(ref.level, ChatRefLevel.section);
    expect(ref.chatTitle, hit.title);
    expect(ref.chapterTitle, 'Native drag-and-drop');
    expect(ref.sectionTitle, 'Chrome live drops pass');
    expect(ref.summary, 'Train at nine.');
    expect(find.text('Could not open this chat'), findsNothing);

    // A chapter-level hit opens at its chapter and carries no section.
    final chapter = Hit(
      id: 'c-2',
      title: 'Chat › Chapter',
      snippet: '',
      fuzzy: false,
      source: 'agent',
      pageType: 'chat',
      target: jsonEncode({
        'sessionId': 'sess-1',
        'firstMessageId': 'm-1',
        'lastMessageId': 'm-2',
        'level': 'chapter',
        'chapterTitle': 'Chapter',
      }),
    );
    await openChatHit(
      _overlayContext(tester),
      chapter,
      openReference: (context, reference) async => asked.add(reference),
    );
    expect(asked.last.level, ChatRefLevel.chapter);
    expect(asked.last.sectionTitle, isNull);
  });

  appTest('a chat the agent does not have is reported, anchored or not', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    expect(store.sessions, isEmpty);
    for (final hit in [
      chatHit('c-1', sessionId: 'gone'),
      chatHit(
        'c-2',
        sessionId: 'gone',
        level: 'chat',
        firstMessage: '',
        lastMessage: '',
      ),
    ]) {
      await real(tester, () => openChatHit(_overlayContext(tester), hit));
      await settle(tester);
      expect(
        find.text('This chat is not on this device any more'),
        findsOneWidget,
      );
      await tester.pump(const Duration(seconds: 10));
      await settle(tester);
    }
    expect(store.openId, isNull);
  });
}
