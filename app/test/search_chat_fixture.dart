// A real native chat with an outline, indexed the way the app indexes it, for
// the search tests and goldens: two exchanges, a closed chapter over the first
// and an ongoing chapter over the second, with a section in the closed one.
// Nothing here is faked: the agent writes its own rows into the vault index.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/src/rust/api/agent.dart';

import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

/// A seeded chat: the store that owns it, its session id, and the message ids
/// of its two exchanges.
class SeededChat {
  SeededChat({
    required this.store,
    required this.sessionId,
    required this.messages,
  });
  final AgentStore store;
  final String sessionId;

  /// The chat's messages in order: user, reply, user, reply.
  final List<AgentMessage> messages;

  /// The closed chapter's messages (the first exchange).
  List<AgentMessage> get closedMessages => messages.sublist(0, 2);

  /// The ongoing chapter's messages (the second exchange).
  List<AgentMessage> get openMessages => messages.sublist(2, 4);
}

/// Seeds the chat: a closed chapter [chapterTitle] with the section
/// [sectionTitle] over the first exchange, and an ongoing chapter after it.
/// The rows are indexed before this returns.
Future<SeededChat> seedChat(
  WidgetTester tester, {
  required String sessionTitle,
  required String chapterTitle,
  required String chapterSummary,
  required String sectionTitle,
  required String sectionSummary,
  required String indexQuery,
}) async {
  final store = await openAgentStore(tester);
  final session = await real(
    tester,
    () => store.createSession(title: sessionTitle),
  );
  await _exchange(tester, store, 'Say ok');
  await _exchange(tester, store, 'Say again');
  final id = session!.id;
  final agent = (await real(tester, store.ensureOpen))!;
  final messages = store.state!.messages;
  final first = messages[0];
  final closedLast = messages[1];
  final openFirst = messages[2];
  final openLast = messages[3];
  await real(
    tester,
    () => agent.upsertOutlineChapter(
      sessionId: id,
      chapter: OutlineChapter(
        id: 'closed',
        title: chapterTitle,
        summary: chapterSummary,
        decisions: const [],
        ongoing: false,
        firstMessageId: first.id,
        lastMessageId: closedLast.id,
        startedAt: first.createdAt,
        // The native outline needs an end once a chapter is closed.
        endedAt: closedLast.createdAt,
        sections: const [],
      ),
    ),
  );
  await real(
    tester,
    () => agent.upsertOutlineSection(
      sessionId: id,
      chapterFirstMessageId: first.id,
      section: OutlineSection(
        id: 'section',
        title: sectionTitle,
        summary: sectionSummary,
        decisions: const [],
        firstMessageId: first.id,
        lastMessageId: closedLast.id,
      ),
    ),
  );
  await real(
    tester,
    () => agent.upsertOutlineChapter(
      sessionId: id,
      chapter: OutlineChapter(
        id: 'ongoing',
        title: 'Trip planning: Next steps',
        summary: 'Still planning the trip.',
        decisions: const [],
        ongoing: true,
        firstMessageId: openFirst.id,
        lastMessageId: openLast.id,
        startedAt: openFirst.createdAt,
        sections: const [],
      ),
    ),
  );
  await waitForAgentRow(tester, indexQuery);
  return SeededChat(store: store, sessionId: id, messages: messages);
}

/// Sends [text] and waits for the reply to finish.
Future<void> _exchange(
  WidgetTester tester,
  AgentStore store,
  String text,
) async {
  final replies =
      store.state?.messages.where((m) => m.role == 'assistant').length ?? 0;
  await real(tester, () => store.send(text));
  await settleUntil(
    tester,
    () =>
        !store.running &&
        (store.state?.messages.where((m) => m.role == 'assistant').length ??
                0) >
            replies,
  );
}

/// The agent's search rows for [query] now, after the index is flushed.
Future<List<Hit>> agentRows(WidgetTester tester, String query) async {
  final vault = current.vault;
  await real(tester, () => vault.flush());
  final results = await real(
    tester,
    () => vault.run((v) => v.searchAdaptive(query: query, limit: 100)),
  );
  return [
    for (final hit in results.hits)
      if (hit.source == agentSearchSource) hit,
  ];
}

Future<void> _pause(WidgetTester tester) =>
    real(tester, () => Future<void>.delayed(const Duration(milliseconds: 50)));

/// The agent row of [sessionId] at [level] for [query], polled while the index
/// catches up; null when it never arrives.
Future<Hit?> agentRowAt(
  WidgetTester tester,
  String query,
  String sessionId,
  String level,
) async {
  for (var i = 0; i < 200; i++) {
    for (final hit in await agentRows(tester, query)) {
      final target = jsonDecode(hit.target ?? '{}') as Map<String, dynamic>;
      if (target['sessionId'] == sessionId && target['level'] == level) {
        return hit;
      }
    }
    await _pause(tester);
  }
  return null;
}

/// Waits until [query] finds at least one agent row.
Future<void> waitForAgentRow(WidgetTester tester, String query) async {
  for (var i = 0; i < 200; i++) {
    if ((await agentRows(tester, query)).isNotEmpty) return;
    await _pause(tester);
  }
  fail('the chat rows never reached the index for "$query"');
}
