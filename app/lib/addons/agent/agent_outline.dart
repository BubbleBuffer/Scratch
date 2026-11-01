// The chat outline as the shared kit draws it. The store holds the backend's
// `ChatOutline`; the kit lays out and draws an `Outline` over any transcript.
// This maps one to the other: dates parsed, and each chapter's message count
// taken from the session's messages (the outline itself carries none).

import 'package:agent_kit/agent_kit.dart' as kit;

import '../../src/rust/api/agent.dart';

/// The kit's outline for [outline]. [messages] are the session's messages in
/// order; a chapter's message count is its span's messages that are not tool
/// results.
kit.Outline kitOutline(
  ChatOutline outline, {
  List<AgentMessage> messages = const [],
}) {
  if (outline.chapters.isEmpty) return kit.Outline.empty;
  final pos = <String, int>{
    for (var i = 0; i < messages.length; i++) messages[i].id: i,
  };
  return kit.Outline([
    for (final c in outline.chapters)
      kit.OutlineChapter(
        id: c.id,
        title: c.title,
        first: c.firstMessageId,
        last: c.lastMessageId,
        decisions: c.decisions,
        summary: c.summary,
        startedAt: DateTime.tryParse(c.startedAt),
        // A closed chapter with an end that is not a date reads as one day.
        endedAt: c.ongoing || c.endedAt == null
            ? null
            : DateTime.tryParse(c.endedAt!) ?? DateTime.tryParse(c.startedAt),
        ongoing: c.ongoing,
        messageCount: _countMessages(
          pos,
          messages,
          c.firstMessageId,
          c.lastMessageId,
        ),
        sections: [
          for (final s in c.sections)
            kit.OutlineSection(
              id: s.id,
              title: s.title,
              first: s.firstMessageId,
              last: s.lastMessageId,
            ),
        ],
      ),
  ]);
}

int _countMessages(
  Map<String, int> pos,
  List<AgentMessage> messages,
  String first,
  String last,
) {
  final a = pos[first] ?? pos[last], b = pos[last] ?? pos[first];
  if (a == null || b == null) return 0;
  var n = 0;
  for (var p = a < b ? a : b; p <= (a < b ? b : a); p++) {
    if (messages[p].role != 'tool') n++;
  }
  return n;
}
