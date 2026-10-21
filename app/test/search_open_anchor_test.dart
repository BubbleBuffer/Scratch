// Ctrl+K over a real native chat: the chat's rows come from the vault index the
// agent writes (the production sink, no fake). Choosing a section or a chapter
// opens the chat at its stored message range, in the presentation the user is
// looking at: the dock beside a page, the full Agent screen when it is open.
// The range sits in a closed chapter, so it is folded until the anchor opens
// it. A title-only chat row opens the chat as an ordinary session.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/shell/navigation.dart';

import 'agent_test.dart' show pumpUntil;
import 'harness.dart';
import 'search_chat_fixture.dart';
import 'sync_helpers.dart';

/// Opens the desktop overlay with Ctrl+K and types [query] into it.
Future<void> _searchFor(WidgetTester tester, String query) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await settle(tester);
  expect(find.byKey(const ValueKey('search-overlay')), findsOneWidget);
  final field = find.descendant(
    of: find.byKey(const ValueKey('search-overlay-field')),
    matching: find.byType(EditableText),
  );
  await tester.enterText(field, query);
  await settle(tester);
}

/// The message [messageId] inside the dock.
Finder _dockMessage(String messageId) => find.descendant(
  of: find.byKey(const ValueKey('agent-panel')),
  matching: find.byKey(ValueKey('agent-msg-$messageId')),
);

/// The message [messageId] inside the full Agent screen.
Finder _screenMessage(String messageId) => find.descendant(
  of: find.byKey(const ValueKey('agent-screen')),
  matching: find.byKey(ValueKey('agent-msg-$messageId')),
);

/// Asserts the message is built inside the chat's own scroll view: the nearest
/// scroll view above it, not the sidebar's or another one on the screen.
void _expectInViewport(WidgetTester tester, Finder message) {
  final viewport = tester.getRect(
    find.ancestor(of: message, matching: find.byType(Scrollable)).first,
  );
  final at = tester.getRect(message);
  expect(
    viewport.contains(at.center),
    isTrue,
    reason:
        'the anchored message sits inside the chat viewport: '
        'viewport $viewport, message $at',
  );
}

/// Shows the dock on [chat] as an ordinary open, then checks the closed
/// chapter's first message is folded: the control for the anchored open.
Future<void> _expectFoldedOnOrdinaryOpen(
  WidgetTester tester,
  SeededChat chat,
) async {
  final ui = current.app.ui
    ..agentPanelOpen = true
    ..changed();
  expect(ui.agentPanelOpen, isTrue);
  await real(tester, () => chat.store.openSession(chat.sessionId));
  // The folding needs the outline; without it every message shows. The outline
  // of an open chat is loaded before the fixture's chapters are written, and
  // only the outline poll (or a reload) brings them in. So wait for the closed
  // chapter itself: outlineFor alone is already true for the empty outline.
  await pumpUntil(
    tester,
    () =>
        chat.store.outlineFor == chat.sessionId &&
        chat.store.outline.chapters.any(
          (c) => !c.ongoing && c.firstMessageId == chat.closedMessages.first.id,
        ) &&
        _dockMessage(chat.openMessages.last.id).evaluate().isNotEmpty,
  );
  await settle(tester);
  expect(
    _dockMessage(chat.closedMessages.first.id),
    findsNothing,
    reason: 'the closed chapter is folded without an anchor',
  );
  await real(tester, () => chat.store.openSession(null));
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  appTest('Ctrl+K opens a section in a closed chapter at its message range', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final chat = await seedChat(
      tester,
      sessionTitle: 'Trip planning',
      chapterTitle: 'Lisbon trip: Hotels and transit',
      chapterSummary: 'Booked a hotel.',
      sectionTitle: 'Lisbon trip: Train booking',
      sectionSummary: 'Train at nine.',
      indexQuery: 'Train booking',
    );
    final section = await agentRowAt(
      tester,
      'Train booking',
      chat.sessionId,
      'section',
    );
    expect(section, isNotNull, reason: 'the section row reached the index');
    // The label is the helper's: chat › chapter › section, where the chapter
    // name drops "Hotels and transit" and the section drops the chapter prefix.
    expect(section!.title, endsWith(' › Lisbon trip › Train booking'));
    await _expectFoldedOnOrdinaryOpen(tester, chat);

    await _searchFor(tester, 'Train booking');
    final row = find.byKey(ValueKey('hit-agent/${section.id}'));
    await settleUntil(tester, () => row.evaluate().isNotEmpty);
    expect(find.text('CHATS'), findsOneWidget);
    await tester.tap(row);
    final first = chat.closedMessages.first;
    final last = chat.closedMessages.last;
    await pumpUntil(
      tester,
      () =>
          chat.store.openId == chat.sessionId &&
          chat.store.pendingAnchor == null &&
          _dockMessage(last.id).evaluate().isNotEmpty,
      diagnostics: () =>
          'openId=${chat.store.openId} pending=${chat.store.pendingAnchor?.sessionId}'
          ' overlay=${find.byKey(const ValueKey('search-overlay')).evaluate().length}'
          ' dock=${find.byKey(const ValueKey('agent-panel')).evaluate().length}'
          ' outline=${chat.store.outlineFor} error=${chat.store.error}',
    );
    await settle(tester);

    expect(find.byKey(const ValueKey('search-overlay')), findsNothing);
    expect(chat.store.openId, chat.sessionId);
    // The anchor revealed the folded chapter; the range is in the dock.
    expect(_dockMessage(first.id), findsOneWidget);
    expect(_dockMessage(last.id), findsOneWidget);
    _expectInViewport(tester, _dockMessage(first.id));
    expect(tester.takeException(), isNull);
  });

  appTest('Ctrl+K opens a chapter in a closed chapter at its message range', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final chat = await seedChat(
      tester,
      sessionTitle: 'Trip planning',
      chapterTitle: 'Lisbon trip: Hotels and transit',
      chapterSummary: 'Booked a hotel.',
      sectionTitle: 'Lisbon trip: Train booking',
      sectionSummary: 'Train at nine.',
      indexQuery: 'Hotels',
    );
    final chapter = await agentRowAt(
      tester,
      'Hotels',
      chat.sessionId,
      'chapter',
    );
    expect(chapter, isNotNull, reason: 'the chapter row reached the index');
    await _expectFoldedOnOrdinaryOpen(tester, chat);

    await _searchFor(tester, 'Hotels');
    final row = find.byKey(ValueKey('hit-agent/${chapter!.id}'));
    await settleUntil(tester, () => row.evaluate().isNotEmpty);
    await tester.tap(row);
    final first = chat.closedMessages.first;
    await pumpUntil(
      tester,
      () =>
          chat.store.pendingAnchor == null &&
          _dockMessage(first.id).evaluate().isNotEmpty,
    );
    await settle(tester);

    expect(find.byKey(const ValueKey('search-overlay')), findsNothing);
    expect(chat.store.openId, chat.sessionId);
    _expectInViewport(tester, _dockMessage(first.id));
    expect(tester.takeException(), isNull);
  });

  appTest('Ctrl+K from the full Agent screen opens the anchor there', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final chat = await seedChat(
      tester,
      sessionTitle: 'Trip planning',
      chapterTitle: 'Lisbon trip: Hotels and transit',
      chapterSummary: 'Booked a hotel.',
      sectionTitle: 'Lisbon trip: Train booking',
      sectionSummary: 'Train at nine.',
      indexQuery: 'Train booking',
    );
    final section = await agentRowAt(
      tester,
      'Train booking',
      chat.sessionId,
      'section',
    );
    expect(section, isNotNull, reason: 'the section row reached the index');

    // The full Agent route is open, so the dock is hidden: the screen takes
    // the anchor, and no dock opens beside it.
    // The route's future completes only when it closes, so it is not awaited.
    unawaited(openAgent(tester.element(find.byType(Overlay).first)));
    await settle(tester);
    await real(tester, () => chat.store.openSession(null));
    await settle(tester);
    await _searchFor(tester, 'Train booking');
    final row = find.byKey(ValueKey('hit-agent/${section!.id}'));
    await settleUntil(tester, () => row.evaluate().isNotEmpty);
    await tester.tap(row);
    final first = chat.closedMessages.first;
    final last = chat.closedMessages.last;
    final onScreen = find.descendant(
      of: find.byKey(const ValueKey('agent-screen')),
      matching: find.byKey(ValueKey('agent-msg-${last.id}')),
    );
    await pumpUntil(
      tester,
      () =>
          chat.store.openId == chat.sessionId &&
          chat.store.pendingAnchor == null &&
          onScreen.evaluate().isNotEmpty,
    );
    await settle(tester);

    expect(find.byKey(const ValueKey('search-overlay')), findsNothing);
    expect(find.byKey(const ValueKey('agent-panel')), findsNothing);
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('agent-screen')),
        matching: find.byKey(ValueKey('agent-msg-${first.id}')),
      ),
      findsOneWidget,
    );
    _expectInViewport(tester, _screenMessage(first.id));
    expect(tester.takeException(), isNull);
  });

  appTest('Ctrl+K opens a title-only chat row as an ordinary session', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final chat = await seedChat(
      tester,
      sessionTitle: 'Trip planning',
      chapterTitle: 'Lisbon trip: Hotels and transit',
      chapterSummary: 'Booked a hotel.',
      sectionTitle: 'Lisbon trip: Train booking',
      sectionSummary: 'Train at nine.',
      indexQuery: 'Trip planning',
    );
    final chatRow = await agentRowAt(
      tester,
      'Trip planning',
      chat.sessionId,
      'chat',
    );
    expect(chatRow, isNotNull, reason: 'the chat row reached the index');

    await real(tester, () => chat.store.openSession(null));
    await settle(tester);
    await _searchFor(tester, 'Trip planning');
    final row = find.byKey(ValueKey('hit-agent/${chatRow!.id}'));
    await settleUntil(tester, () => row.evaluate().isNotEmpty);
    await tester.tap(row);
    final last = chat.openMessages.last;
    await pumpUntil(
      tester,
      () =>
          chat.store.openId == chat.sessionId &&
          _dockMessage(last.id).evaluate().isNotEmpty,
    );
    await settle(tester);

    expect(find.byKey(const ValueKey('search-overlay')), findsNothing);
    expect(chat.store.openId, chat.sessionId);
    // No range was asked for, so no span was left pending.
    expect(chat.store.pendingAnchor, isNull);
    expect(tester.takeException(), isNull);
  });
}
